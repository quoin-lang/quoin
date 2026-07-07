//! Compiled frames inside user fibers (the lifted entry gate): value parity
//! with the interpreter, suspension ACROSS `Fiber.yield` with a compiled
//! frame live on the fiber stack, nested fibers, and — the reason the gate
//! existed — abandonment: a fiber dropped while suspended over compiled
//! frames must leak that stack (`Fiber::drop` + `force_reset`), never abort
//! the process in corosensei's forced unwind.

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
    panic!("aot-fibers script did not pass after {ATTEMPTS} attempts.\n{last_diag}");
}

/// One script covering the whole surface; runs on BOTH the maximal-compilation
/// stress mode and the kill switch, asserting identical values — the parity
/// contract for compiled-in-fiber execution.
const SCRIPT: &str = r#"
var ok = true;

"* a compiled method whose frame stays LIVE across a Fiber.yield: the yield
"* suspends the fiber with the Cranelift frame frozen on its stack
GenBody <- {
    .meta <-- {
        emit: -> { |n: Integer ^Integer|
            Fiber.yield:(n * 2);
            n * 3
        }
    };
    .sealed!
};
var f = Fiber.new:{ |z|
    var acc = 0;
    (0..5).each:{ |k| acc = acc + (GenBody.emit:k) };
    acc
};
var yields = #();
yields.add:(f.resume:0);
(0..4).each:{ |i| yields.add:(f.resume) };
var fin = f.resume;
(yields == #( 0 2 4 6 8 )).else:{ ok = false };
(fin == 30).else:{ ok = false };

"* nested fibers, compiled bodies in both layers
var inner = Fiber.new:{ |z| Fiber.yield:(GenBody.emit:10); 0 };
var outer = Fiber.new:{ |z|
    Fiber.yield:(inner.resume:0);
    0
};
((outer.resume:0) == 20).else:{ ok = false };

"* abandonment: drop fibers suspended over compiled frames, churn the GC,
"* and keep running (pre-change this force-unwound across Cranelift frames
"* and aborted the process)
var dead1 = Fiber.new:{ |z| (0..1000000).each:{ |k| Fiber.yield:(GenBody.emit:k) }; 0 };
dead1.resume:0;
dead1.resume;
dead1 = nil;
var lazyGot = ((1..100).lazyCollect:{ |x| x * 3 }).take:4;
((lazyGot.at:3) == 12).else:{ ok = false };
var churn = 0;
(0..200000).each:{ |i|
    churn = churn + ((('x' + i.s).contains?:'42').if:{ 1 } else:{ 0 })
};
(churn > 0).else:{ ok = false };

"* an error inside a compiled frame in a fiber surfaces to the resumer
var bad = Fiber.new:{ |z| GenBody.emit:('nope') ; 0 };
var caught = { bad.resume:0; 'no-error' }.catch:{ |e| 'caught' };
(caught == 'caught').else:{ ok = false };

ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;

#[test]
fn compiled_frames_in_fibers_maximal() {
    assert_script_passes_env("qn_aot_fibers_warm.qn", SCRIPT, &[("QN_AOT_WARM", "1")]);
}

#[test]
fn compiled_frames_in_fibers_default_warmth() {
    assert_script_passes_env("qn_aot_fibers_default.qn", SCRIPT, &[]);
}

#[test]
fn compiled_frames_in_fibers_parity_interpreted() {
    // The kill switch: identical values with the AOT tier off entirely.
    assert_script_passes_env("qn_aot_fibers_off.qn", SCRIPT, &[("QN_AOT", "0")]);
}

/// The fiber-heavy path actually compiles (not silently bailed): under the
/// forced-warmth mode, VM.stats must show compiled members and the fiber
/// exercise above must not have pushed entry bails.
#[test]
fn fiber_entries_compile_not_bail() {
    let script = r#"
Hot <- {
    .meta <-- {
        step: -> { |n: Integer ^Integer| n * 2 + 1 }
    };
    .sealed!
};
var f = Fiber.new:{ |z| (0..20).each:{ |k| Fiber.yield:(Hot.step:k) }; 0 };
var s = f.resume:0;
(0..9).each:{ |i| s = s + (f.resume) };
var aot = VM.stats.at:'aot';
var ok = (s == 100) && ((aot.at:'compiled') >= 1) && ((aot.at:'entryBails') == 0);
ok.if:{ 'PASS'.print } else:{ ('FAIL s=' + s.s).print };
"#;
    assert_script_passes_env("qn_aot_fibers_stats.qn", script, &[("QN_AOT_WARM", "1")]);
}
