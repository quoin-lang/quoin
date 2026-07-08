//! Portable blocks (docs/CONCURRENCY_ARCH.md §10): `Worker.start:{...}`
//! ships a block as (Send template reference + deep-copied snapshot of its
//! free reads); join returns the block's value. The portability scan
//! refuses — loudly, at submit time — everything that can't cross:
//! write-captures, `^^`, self/@fields, non-portable capture values,
//! class/method definition. Missing user globals error clearly from the
//! worker instead of resolving to silent nil.

use std::process::Command;

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
            std::thread::sleep(std::time::Duration::from_millis(150 * attempt as u64));
        }
    }
    panic!("portable-blocks script did not pass after {ATTEMPTS} attempts.\n{last_diag}");
}

const SCRIPT: &str = r#"
var ok = true;

"* value: join returns the block's result
(((Worker.start:{ 40 + 2 }).join) == 42).else:{ ok = false };

"* captures are a SNAPSHOT: parent mutation after spawn is invisible
var factor = 3;
var w = Worker.start:{ factor * 14 };
factor = 100;
((w.join) == 42).else:{ ok = false };

"* structured captures deep-copy; nested blocks may write shipped-scope
"* locals and read free names through the snapshot
var offsets = #( 1 2 3 );
var n = 5;
var got = (Worker.start:{
    var s = 0;
    (0..n).each:{ |i| s = s + i };
    offsets.each:{ |o| s = s + o };
    s
}).join;
(got == 16).else:{ ok = false };

"* the worker-side lanes work from a block worker; gather composes joins
var w1 = Worker.start:{ (Worker.receive) * 2 };
var w2 = Worker.start:{ (Worker.receive) * 3 };
w1.send:10;
w2.send:10;
((Async.gather:#( { w1.join } { w2.join } )) == #( 20 30 )).else:{ ok = false };

"* nil result is fine (Null crosses)
(((Worker.start:{ nil }).join) == nil).else:{ ok = false };

ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;

#[test]
fn portable_blocks_values_and_snapshots() {
    assert_script_passes_env("qn_pb_values.qn", SCRIPT, &[]);
}

/// Same script under maximal AOT warmth: the shipped template may compile in
/// the worker (per-worker AOT state; the fibers arc's per-task machinery).
#[test]
fn portable_blocks_under_aot_warm() {
    assert_script_passes_env("qn_pb_warm.qn", SCRIPT, &[("QN_AOT_WARM", "1")]);
}

#[test]
fn portable_blocks_refusals() {
    let script = r#"
var ok = true;

"* write-capture refuses at submit, catchable, naming the binding
var c = 0;
var wc = { Worker.start:{ c = c + 1 }; 'started' }.catch:{ |e| e.s };
((wc.contains?:'c') && (wc.contains?:'writes captured')).else:{ ok = false };

"* a captured BLOCK ships recursively since L3 (the combinator enabler)
var g = { |x| x * 2 };
(((Worker.start:{ g.valueWithSelfOrArg:21 }).join) == 42).else:{ ok = false };

"* a genuinely non-portable capture (a user-class instance) refuses, named
Box <- { |@v| init -> { @v = 1 } };
var inst = Box.new;
var bc = { Worker.start:{ inst }; 'started' }.catch:{ |e| e.s };
(bc.contains?:'inst').else:{ ok = false };

"* ^^ refuses (its home method can't exist over there)
Nlr <- {
    .meta <-- {
        tryStart -> {
            { Worker.start:{ ^^5 }; 'started' }.catch:{ |e| 'refused-nlr' }
        }
    }
};
((Nlr.tryStart) == 'refused-nlr').else:{ ok = false };

"* @field access refuses
Holder <- { |@x|
    .meta <-- { make -> { Holder.new } };
    init -> { @x = 1 };
    tryStart -> {
        { Worker.start:{ @x + 1 }; 'started' }.catch:{ |e| 'refused-field' }
    }
};
((Holder.make.tryStart) == 'refused-field').else:{ ok = false };

"* parameterized blocks refuse (no argument channel)
var pb = { Worker.start:{ |x| x }; 'started' }.catch:{ |e| 'refused-params' };
(pb == 'refused-params').else:{ ok = false };

"* a missing user global errors CLEARLY from the worker (not silent nil)
Helper <- { .meta <-- { seven -> { 7 } } };
var hg = { (Worker.start:{ Helper.seven }).join; 'ran' }.catch:{ |e|
    (e.s.contains?:'Helper').if:{ 'named' } else:{ e.s }
};
(hg == 'named').else:{ ok = false };

ok.if:{ 'PASS'.print } else:{ ('FAIL wc=' + wc + ' bc=' + bc).print };
"#;
    assert_script_passes_env("qn_pb_refusals.qn", script, &[]);
}

/// The L3 preview: a hand-rolled parallel map over portable blocks —
/// spawn N workers each capturing a different input, gather the joins.
#[test]
fn portable_blocks_parallel_map_preview() {
    let script = r#"
var jobs = #();
#( 1 2 3 4 5 6 ).each:{ |x|
    var input = x;
    jobs.add:(Worker.start:{ input * input })
};
var outs = Async.gather:(jobs.collect:{ |j| { j.join } });
(outs == #( 1 4 9 16 25 36 )).if:{ 'PASS'.print } else:{ ('FAIL: ' + outs.s).print };
"#;
    assert_script_passes_env("qn_pb_parmap.qn", script, &[]);
}
