//! `VM.stats` / `VM.aotRefusals` — the AOT coverage counters (docs/internal/AOT_ARCH.md
//! observability): compiled totals, distinct refusals/skips with per-kind
//! counts, and the per-member drill-down. Run under `QN_AOT_WARM=1` so lazily
//! tiered members compile on first use inside the script.

use std::process::Command;

/// Run one `.qn` script through `qn` with extra env vars, asserting it prints
/// `PASS`. Retries a few times (same transient-subprocess rationale as
/// `tests/extension.rs`).
fn assert_script_passes_env(name: &str, script: &str, envs: &[(&str, &str)]) {
    const ATTEMPTS: u32 = 4;
    let mut last_diag = String::new();
    for attempt in 1..=ATTEMPTS {
        let path = std::env::temp_dir().join(name);
        std::fs::write(&path, script).unwrap();
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_qn"));
        cmd.arg(&path);
        for (k, v) in envs {
            cmd.env(k, v);
        }
        let out = cmd.output().expect("run qn");
        let _ = std::fs::remove_file(&path);
        let stdout = String::from_utf8_lossy(&out.stdout);
        if stdout.contains("PASS") {
            return;
        }
        last_diag = format!(
            "status: {:?}\nstdout:\n{stdout}\nstderr:\n{}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        if attempt < ATTEMPTS {
            std::thread::sleep(std::time::Duration::from_millis(100 * attempt as u64));
        }
    }
    panic!("vm-stats script did not pass after {ATTEMPTS} attempts.\n{last_diag}");
}

/// One script exercises all three outcome kinds and then reads them back:
/// a typed sealed method that COMPILES, an annotated method whose >8-element
/// list literal REFUSES (arityCap), and typed multimethod arms that SKIP at
/// candidacy (precheckMultiVariant) — plus the per-member drill-down naming
/// the refused selector with its kind.
#[test]
fn vm_stats_reports_aot_coverage() {
    let script = r#"
var ok = true;

"* compiles: sealed owner, scalar param + return
StatsDbl <- {
    .meta <-- {
        dbl: -> { |n: Integer ^Integer| n * 2 }
    };
    .sealed!
};
((StatsDbl.dbl:4) == 8).else:{ ok = false };

"* refuses: the 9-element list literal is past the compiled ABI's 8-wide cap
StatsNine <- {
    .meta <-- {
        nine -> { |^List| #( 1 2 3 4 5 6 7 8 9 ) }
    };
    .sealed!
};
((StatsNine.nine).count == 9).else:{ ok = false };

"* skips at candidacy: a typed multimethod selector
StatsOver <- {
    .meta <-- {
        over: -> { |x: Integer| 1 };
        over: -> { |x: String| 2 }
    };
    .sealed!
};
((StatsOver.over:1) == 1).else:{ ok = false };
((StatsOver.over:'s') == 2).else:{ ok = false };

var aot = VM.stats.at:'aot';
((aot.at:'compiled') >= 1).else:{ ok = false };
((aot.at:'refused') >= 1).else:{ ok = false };
((aot.at:'skipped') >= 1).else:{ ok = false };
var reasons = aot.at:'reasons';
((reasons.at:'arityCap') >= 1).else:{ ok = false };
((reasons.at:'precheckMultiVariant') >= 1).else:{ ok = false };

"* the drill-down names the member and its bucket
var found = nil;
VM.aotRefusals.each:{ |r| ((r.at:'selector') == 'nine').if:{ found = r } };
found.defined?.else:{ ok = false };
found.defined?.if:{ ((found.at:'kind') == 'arityCap').else:{ ok = false } };

ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;
    assert_script_passes_env("qn_vm_stats_test.qn", script, &[("QN_AOT_WARM", "1")]);
}

/// With the AOT tier killed, the surface still answers (zeroed sections, not
/// an error) — `VM.stats` must be safe to call unconditionally.
#[test]
fn vm_stats_with_aot_disabled() {
    let script = r#"
var aot = VM.stats.at:'aot';
((aot.at:'compiled') == 0).if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;
    assert_script_passes_env("qn_vm_stats_disabled_test.qn", script, &[("QN_AOT", "0")]);
}

/// The `'dispatch'` section (`src/vm/counters.rs`). A tight loop over one
/// receiver shape drives the inline cache to a hit rate far above its miss
/// count; a second site fed ten different shapes flips repeatedly and crosses
/// the megamorphic threshold. Both are asserted as inequalities, not exact
/// numbers — the prelude dispatches too, and its traffic is not this test's
/// business.
#[test]
fn vm_stats_reports_dispatch_counters() {
    let script = r#"
var ok = true;

"* Monomorphic: 20k sends at one site, all Integer -> hits dominate.
var total = 0;
(1..20000).each:{ |i| total = total + (i * 2) };
(total > 0).else:{ ok = false };

"* Polymorphic: the SAME `.s` site sees ten distinct value shapes, five times
"* over -- far past the 8 flips that mark a site megamorphic.
var vals = #( 1 'a' 2.0 true false nil #(1) #{'k': 1} #sym 9 );
var sink = 0;
(1..5).each:{ |round| vals.each:{ |v| sink = sink + (v.s).length } };
(sink > 0).else:{ ok = false };

var d = VM.stats.at:'dispatch';
((d.at:'icHits') > 10000).else:{ ok = false };
((d.at:'icHits') > (d.at:'icMisses')).else:{ ok = false };
((d.at:'icFills') >= 1).else:{ ok = false };
((d.at:'icFlips') >= 8).else:{ ok = false };
((d.at:'megamorphic') >= 1).else:{ ok = false };

"* The global (class+selector) cache sits behind every IC fill, so a run that
"* filled anything must have resolved through it.
((d.at:'cacheHits') + (d.at:'cacheMisses') >= 1).else:{ ok = false };
((d.at:'cacheEntries') >= 1).else:{ ok = false };
((d.at:'epoch') >= 1).else:{ ok = false };

ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;
    assert_script_passes_env("qn_vm_stats_dispatch_test.qn", script, &[]);
}

/// The `'gc'` section. Allocating hard enough to wake the collector must move
/// `collections` off zero, and the live-heap numbers `gc_arena` tracks must be
/// non-zero for any running program. `QN_GC_SLEEP` is turned down so the test
/// doesn't have to allocate its way through the throughput-tuned default pacing.
#[test]
fn vm_stats_reports_gc_counters() {
    let script = r#"
var ok = true;
var sink = nil;
(1..50000).each:{ |i| sink = #( i (i + 1) (i + 2) ) };

var gc = VM.stats.at:'gc';
((gc.at:'pumps') >= 1).else:{ ok = false };
((gc.at:'collections') >= 1).else:{ ok = false };
((gc.at:'liveObjects') >= 1).else:{ ok = false };
((gc.at:'liveBytes') >= 1).else:{ ok = false };
((gc.at:'debt') >= 0).else:{ ok = false };
((gc.at:'sleepFactor') > 0).else:{ ok = false };

"* The phase is one of gc_arena's four, whatever the sample caught.
var phase = gc.at:'phase';
var known = false;
#( 'sleeping' 'marking' 'marked' 'sweeping' ).each:{ |p| (p == phase).if:{ known = true } };
known.else:{ ok = false };

ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;
    assert_script_passes_env("qn_vm_stats_gc_test.qn", script, &[("QN_GC_SLEEP", "0.5")]);
}

/// The `'compile'` section counts BYTECODE-compiler decisions, so it fills in
/// with the AOT tier switched off entirely — that is the whole point of keeping
/// it separate from `'aot'`. A sealed class's leaf self-send inlines, typed
/// operands lower to `Int*` ops, and `if:`/`each:` fuse into native control flow.
#[test]
fn vm_stats_reports_compile_counters_without_aot() {
    let script = r#"
var ok = true;

"* Inlines: `.width` is a leaf self-send inside a sealed class (Phase 5.1).
Boxy <- { |@width| width -> { @width }; getW -> { .width }; .sealed! };
var b = Boxy.new:{ width = 7 };
((b.getW) == 7).else:{ ok = false };

"* Devirt ops: both operands statically Integer -> IntAdd, not a send.
var twice = { |x: Integer ^Integer| x + x };
((twice.value:21) == 42).else:{ ok = false };

"* Fused control flow: `if:else:` on a Bool receiver, and `each:` on a List.
var n = 0;
(1 < 2).if:{ n = 1 } else:{ n = 2 };
#( 1 2 3 ).each:{ |x| n = n + x };
((n == 7)).else:{ ok = false };

var c = VM.stats.at:'compile';
((c.at:'methodsInlined') >= 1).else:{ ok = false };
((c.at:'devirtOps') >= 1).else:{ ok = false };
((c.at:'controlFlowInlined') >= 1).else:{ ok = false };

"* ... and the AOT section really is off, so these are not its numbers.
((((VM.stats.at:'aot').at:'compiled')) == 0).else:{ ok = false };

ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;
    assert_script_passes_env("qn_vm_stats_compile_test.qn", script, &[("QN_AOT", "0")]);
}

/// Every section is present and readable in one call — `VM.stats` is documented
/// as safe to call unconditionally, and the `stats` doc example pins this exact
/// key list (it is executed by `qn doc --check`).
///
/// Every section also declares its `scope`, and the split is not cosmetic: the
/// compile and JIT tiers cache their artifacts per process and reuse them across
/// VM sessions, so those numbers cannot be per-VM, while dispatch/gc/pins are one
/// VM's own runtime state. Reading two of them side by side without knowing which
/// is which is the trap the key exists to close.
#[test]
fn vm_stats_lists_every_section_with_its_scope() {
    let script = r#"
var ok = true;
var want = #( 'aot' 'compile' 'dispatch' 'gc' 'compute' 'workers' 'pins' );
var got = VM.stats.keys;
(got == want).else:{ ok = false };
want.each:{ |k| (VM.stats.at:k).defined?.else:{ ok = false } };

"* Scope is declared by every section, and is one of the two known values.
want.each:{ |k|
    var s = (VM.stats.at:k).at:'scope';
    ((s == 'process') || (s == 'vm')).else:{ ok = false }
};

"* ... and each section is on the side its backing store forces.
#( 'aot' 'compile' 'compute' 'workers' ).each:{ |k|
    (((VM.stats.at:k).at:'scope') == 'process').else:{ ok = false }
};
#( 'dispatch' 'gc' 'pins' ).each:{ |k|
    (((VM.stats.at:k).at:'scope') == 'vm').else:{ ok = false }
};

ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;
    assert_script_passes_env("qn_vm_stats_sections_test.qn", script, &[]);
}
