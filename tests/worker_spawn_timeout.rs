//! Issue #147: process-backed worker startup must compose with
//! `Async.timeout:` and cancellation like any other I/O wait — the blocking
//! accept/handshake runs on a helper thread while the spawning task parks
//! (`IoRequest::WorkerSpawnJoin`), so a stalled child no longer freezes every
//! task in the VM for the 10s backstop. The stall is injected with the
//! debug-only `QN_WORKER_SERVE_STALL_MS` hook, read at the top of
//! `worker_serve_main` before the child connects.

use std::process::Command;
use std::time::{Duration, Instant};

/// Run `script` under the given env and return (stdout, stderr, elapsed).
/// Units are materialized beside it with `@name@` substitution, as in the
/// worker_service / peer_death harnesses.
fn run_script(
    name: &str,
    script: &str,
    units: &[(&str, &str)],
    envs: &[(&str, &str)],
) -> (String, String, Duration) {
    let dir = std::env::temp_dir().join(format!("qn_spawn_to_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut script = script.to_string();
    for (unit_name, source) in units {
        let path = dir.join(unit_name);
        std::fs::write(&path, source).unwrap();
        script = script.replace(&format!("@{unit_name}@"), path.to_str().unwrap());
    }
    let main_path = dir.join("main.qn");
    std::fs::write(&main_path, &script).unwrap();
    let started = Instant::now();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_qn"));
    cmd.arg(&main_path);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run qn");
    let elapsed = started.elapsed();
    let _ = std::fs::remove_dir_all(&dir);
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        elapsed,
    )
}

/// The reporter's exact scenario: a 15s-stalled child under `Async.timeout:800`
/// must answer `TimeoutError` at the deadline — not the untyped 10s-backstop
/// string at 10s. The elapsed bound is what pins the fix: under the old
/// driver-blocking spawn this takes 10s+ regardless of the timeout.
#[test]
fn timeout_interrupts_a_stalled_spawn() {
    let script = r#"
var r = { Async.timeout:800 do:{ Worker.with:{ Duration.seconds:1 } backing:'process' }; 'no-error' }
    .catch:{ |e| e.class.name };
(r == 'TimeoutError').if:{ 'PASS'.print } else:{ ('FAIL got ' + r).print };
"#;
    let (stdout, stderr, elapsed) = run_script(
        "timeout",
        script,
        &[],
        &[("QN_WORKER_SERVE_STALL_MS", "15000")],
    );
    assert!(
        stdout.contains("PASS"),
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        elapsed < Duration::from_secs(6),
        "the timeout took {elapsed:?} — the spawn wait is blocking again"
    );
}

/// Scheduler liveness: while one task's spawn is stalled, other tasks keep
/// running. The sleeper's 100ms tick must land BEFORE the 1200ms-stalled
/// spawn completes — under the old blocking spawn the driver freezes first
/// and the order inverts.
#[test]
fn other_tasks_run_while_a_spawn_stalls() {
    let script = r#"
var order = List.new;
Async.gather:#(
    { var w = Worker.with:{ Duration.seconds:1 } backing:'process'; order.add:'spawned'; 0 }
    { Async.sleep:100; order.add:'tick'; 0 }
);
(((order.at:0) == 'tick') && ((order.at:1) == 'spawned'))
    .if:{ 'PASS'.print } else:{ ('FAIL order ' + order.s).print };
"#;
    let (stdout, stderr, _elapsed) = run_script(
        "liveness",
        script,
        &[],
        &[("QN_WORKER_SERVE_STALL_MS", "1200")],
    );
    assert!(
        stdout.contains("PASS"),
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
}

/// The cancel guard kills the child AT the timeout (through the
/// early-published grip), and the collapsing helper thread reaps it — no
/// live child, no zombie, well before the 10s backstop would have fired.
/// Synchronized on the script's own verdict line (a fixed wall-clock sample
/// races VM boot on a slow runner), then pgrep polls to empty: the kill is
/// issued before the catch resumes, but SIGKILL delivery and the helper's
/// reap are asynchronous.
#[test]
fn a_timed_out_spawn_leaves_no_child_behind() {
    use std::io::BufRead;
    let script = r#"
var r = { Async.timeout:700 do:{ Worker.with:{ Duration.seconds:1 } backing:'process' }; 'no-error' }
    .catch:{ |e| e.class.name };
r.print;
Async.sleep:8000;
'DONE'.print;
"#;
    let dir = std::env::temp_dir().join("qn_spawn_to_nochild");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let main_path = dir.join("main.qn");
    std::fs::write(&main_path, script).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_qn"))
        .arg(&main_path)
        .env("QN_WORKER_SERVE_STALL_MS", "15000")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("run qn");
    let mut lines = std::io::BufReader::new(child.stdout.take().expect("piped stdout")).lines();
    let verdict = lines
        .next()
        .expect("a verdict line before EOF")
        .expect("readable stdout");
    if verdict.trim() != "TimeoutError" {
        let _ = child.kill();
        let _ = child.wait();
        panic!("expected TimeoutError, got {verdict:?}");
    }
    // The verdict means the timeout fired and the guard's kill was issued;
    // poll for the kill/reap to settle while the script's sleep keeps the
    // parent alive. Anything still visible at the deadline is a real leak.
    let deadline = Instant::now() + Duration::from_secs(5);
    let kids = loop {
        let pgrep = Command::new("pgrep")
            .args(["-P", &child.id().to_string()])
            .output()
            .expect("run pgrep");
        let kids = String::from_utf8_lossy(&pgrep.stdout).trim().to_string();
        if kids.is_empty() || Instant::now() > deadline {
            break kids;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        kids.is_empty(),
        "the timed-out spawn left children behind: {kids}"
    );
}

/// Rule 5 under a STALLED respawn: the restart gate is now held across a
/// park (the spawn wait), and a send landing inside that widened window must
/// park through it and answer from the fresh incarnation — the
/// `peer_death.rs` rule-5 contract, exercised with the spawn actually parked.
#[test]
fn senders_park_across_a_stalled_restart() {
    let script = r#"
var svc = Worker.host:'@kaboom.qn@' with:{ Kaboom.new } backing:'process';
var pid = ((VM.ps.at:'workers').at:0).at:'pid';
[IO]File.write:pid.s to:'@pidfile@.tmp';
[IO]File.rename:'@pidfile@.tmp' to:'@pidfile@';
var t1 = { svc.sleepLong }.catch:{ |e:PeerDiedError| 'died' };
var results = Async.gather:#(
    { svc.serviceRestart; 'restarted' }
    { Async.sleep:1; { svc.ping }.catch:{ |e:PeerDiedError| 'window-error' } }
);
((t1 == 'died') && (results == #( 'restarted' 'pong' )))
    .if:{ 'PASS'.print } else:{ ('FAIL ' + t1 + ' ' + results.s).print };
"#;
    let kaboom = r#"
Kaboom <- {
    sleepLong -> { Async.sleep:60000; 'unreachable' }
    ping -> { 'pong' }
}
"#;
    const ATTEMPTS: u32 = 3;
    let dir = std::env::temp_dir().join("qn_spawn_to_restart");
    let mut last_diag = String::new();
    for attempt in 1..=ATTEMPTS {
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let unit_path = dir.join("kaboom.qn");
        std::fs::write(&unit_path, kaboom).unwrap();
        let pidfile = dir.join("pid.txt");
        let script = script
            .replace("@kaboom.qn@", unit_path.to_str().unwrap())
            .replace("@pidfile@", pidfile.to_str().unwrap());
        let main_path = dir.join("main.qn");
        std::fs::write(&main_path, &script).unwrap();
        let child = Command::new(env!("CARGO_BIN_EXE_qn"))
            .arg(&main_path)
            .env("QN_WORKER_SERVE_STALL_MS", "900")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("run qn");
        // Rename-published pid: wait for it, give the parked call a beat to
        // open, then deliver the hard death that arms serviceRestart.
        let mut pid = String::new();
        for _ in 0..150 {
            if let Ok(s) = std::fs::read_to_string(&pidfile)
                && !s.trim().is_empty()
            {
                pid = s.trim().to_string();
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(!pid.is_empty(), "the script never published a pid");
        std::thread::sleep(Duration::from_millis(300));
        let _ = Command::new("kill").args(["-9", &pid]).status();
        let out = child.wait_with_output().expect("qn exits");
        let stdout = String::from_utf8_lossy(&out.stdout);
        if stdout.contains("PASS") {
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        last_diag = format!(
            "status: {:?}\nstdout:\n{stdout}\nstderr:\n{}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
        if attempt < ATTEMPTS {
            std::thread::sleep(Duration::from_millis(200 * attempt as u64));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    panic!("stalled-restart script did not pass after {ATTEMPTS} attempts.\n{last_diag}");
}
