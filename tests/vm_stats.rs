//! `VM.stats` / `VM.aotRefusals` — the AOT coverage counters (docs/AOT_ARCH.md
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
