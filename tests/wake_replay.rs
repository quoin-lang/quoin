//! The wake-log divergence test (`ACTOR_OBJECTS.md` §8): record a stressed run, replay
//! it, and require the replayed run to produce the identical event stream and output.
//! This is the enforcement that every scheduler wake path flows through the logged
//! choke points — a new wake source that bypasses them diverges here long before the
//! full replayer (arc 4) exists.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn scratch_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("qn_wake_replay_{}_{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run_qn(script: &Path, envs: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_qn"));
    cmd.arg(script);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    cmd.output().expect("run qn")
}

/// Contested scheduling with deterministic result *content*: two producers race sends
/// into one channel while a consumer drains it, a timeout that does NOT fire leaves a
/// stale deadline timer (its late firing is a logged delivery), and one that DOES fire
/// exercises the deadline-win path. Which task wins each race is the stress seed's
/// choice — exactly what record/replay must pin.
const SCRIPT: &str = r#"var ch = Channel.new;
var t1 = Task.spawn:{
    var i = 0;
    { i < 6 }.whileDo:{ i = i + 1; Async.sleep:1; ch.send:(10 + i) };
};
var t2 = Task.spawn:{
    var i = 0;
    { i < 6 }.whileDo:{ i = i + 1; Async.sleep:1; ch.send:(20 + i) };
};
var got = #();
var j = 0;
{ j < 12 }.whileDo:{ j = j + 1; got.add:ch.receive };
t1.join;
t2.join;
Async.timeout:200 do:{ Async.sleep:2 };
var late = { Async.timeout:2 do:{ Async.sleep:200; 'slept' } }.catch:{ |e| 'timedout' };
('order: ' + got.s).print;
('late: ' + late).print;
((got.count == 12) && (late == 'timedout')).if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;

#[test]
fn stressed_run_replays_identically() {
    let dir = scratch_dir("diverge");
    let script = dir.join("prog.qn");
    std::fs::write(&script, SCRIPT).unwrap();

    for seed in ["1", "20260713"] {
        let log1 = dir.join(format!("record_{seed}.log"));
        let log2 = dir.join(format!("replay_{seed}.log"));

        let rec = run_qn(
            &script,
            &[
                ("QN_SCHED_STRESS", seed),
                ("QN_WAKE_RECORD", log1.to_str().unwrap()),
            ],
        );
        let rec_out = String::from_utf8_lossy(&rec.stdout).to_string();
        assert!(
            rec.status.success() && rec_out.contains("PASS"),
            "record run (seed {seed}) failed.\nstdout:\n{rec_out}\nstderr:\n{}",
            String::from_utf8_lossy(&rec.stderr)
        );

        // Replay under the same stress env (same yield cadence); the rng itself is
        // never consulted — every decision comes from the log. Recording during the
        // replay produces the second stream to compare.
        let rep = run_qn(
            &script,
            &[
                ("QN_SCHED_STRESS", seed),
                ("QN_WAKE_REPLAY", log1.to_str().unwrap()),
                ("QN_WAKE_RECORD", log2.to_str().unwrap()),
            ],
        );
        let rep_out = String::from_utf8_lossy(&rep.stdout).to_string();
        let rep_err = String::from_utf8_lossy(&rep.stderr).to_string();
        assert!(
            rep.status.success() && rep_out.contains("PASS"),
            "replay run (seed {seed}) failed.\nstdout:\n{rep_out}\nstderr:\n{rep_err}"
        );
        assert!(
            !rep_err.contains("divergence") && !rep_err.contains("unconsumed"),
            "replay run (seed {seed}) diverged.\nstderr:\n{rep_err}"
        );

        // The whole point: identical program output AND identical event streams.
        assert_eq!(rec_out, rep_out, "stdout diverged (seed {seed})");
        let stream1 = std::fs::read_to_string(&log1).unwrap();
        let stream2 = std::fs::read_to_string(&log2).unwrap();
        assert_eq!(stream1, stream2, "event streams diverged (seed {seed})");
        assert!(
            stream1.lines().count() > 50,
            "suspiciously small log — did the hooks record?\n{stream1}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Different seeds genuinely schedule differently — otherwise the test above proves
/// nothing. (Compared without hashes: I/O result payloads can legitimately match.)
#[test]
fn different_seeds_record_different_streams() {
    let dir = scratch_dir("seeds");
    let script = dir.join("prog.qn");
    std::fs::write(&script, SCRIPT).unwrap();

    let mut streams = Vec::new();
    for seed in ["7", "8675309"] {
        let log = dir.join(format!("record_{seed}.log"));
        let out = run_qn(
            &script,
            &[
                ("QN_SCHED_STRESS", seed),
                ("QN_WAKE_RECORD", log.to_str().unwrap()),
            ],
        );
        assert!(out.status.success());
        // Drop the header (it names the seed) and the hashes; keep decision shape.
        let decisions: Vec<String> = std::fs::read_to_string(&log)
            .unwrap()
            .lines()
            .skip(1)
            .map(|l| l.split_whitespace().take(2).collect::<Vec<_>>().join(" "))
            .collect();
        streams.push(decisions);
    }
    assert_ne!(
        streams[0], streams[1],
        "two seeds produced identical decision streams — stress is not varying the schedule"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The `QN_WAKE_LOG` ring's consumer: a global deadlock dumps the recent wake events.
#[test]
fn wake_log_ring_dumps_on_deadlock() {
    let dir = scratch_dir("deadlock");
    let script = dir.join("deadlock.qn");
    std::fs::write(
        &script,
        "var ch = Channel.new;\nvar t = Task.spawn:{ ch.receive };\nch.receive;\n",
    )
    .unwrap();

    let out = run_qn(&script, &[("QN_WAKE_LOG", "1"), ("QN_SCHED_STRESS", "1")]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "deadlock should fail the run");
    assert!(
        stderr.contains("deadlock"),
        "expected the deadlock error.\nstderr:\n{stderr}"
    );
    assert!(
        stderr.contains("wake log (global deadlock"),
        "expected the ring dump.\nstderr:\n{stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
