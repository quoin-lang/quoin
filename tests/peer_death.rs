//! Peer deaths are TYPED (SUPERVISION.md slice 0): a vanished isolate raises
//! `PeerDiedError` (reason symbol + peer name) — never an untyped string, never an
//! `IoError` — at every seam: `join`, a service call mid-conversation, and the
//! fail-fast on a corpse. Death housekeeping rides along: the dead service's claim
//! rows carry the `gone` marker, and a dead link's parked remote channel RECEIVERS
//! are purged so a later local send reaches a live receiver instead of vanishing
//! into the closed lane (the silent value-loss bug).
//!
//! The extension flavors of the same seams live in `extension.rs`
//! (`extension_crash_isolation`, `extension_death_while_queued_fails_fast`).

use std::process::Command;

/// Run `script` (with `@unit@` placeholders materialized beside it) and assert it
/// prints PASS. When `kill_via_pidfile` is set, the script is expected to write a
/// child pid to `pid.txt` in its directory mid-run; the harness SIGKILLs that pid —
/// a real hard death, which no in-VM surface can fake — and then reads the verdict.
fn assert_passes(name: &str, script: &str, units: &[(&str, &str)], kill_via_pidfile: bool) {
    const ATTEMPTS: u32 = 3;
    let dir = std::env::temp_dir().join(format!("qn_peerdeath_{name}"));
    let mut last_diag = String::new();
    for attempt in 1..=ATTEMPTS {
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut script = script.to_string();
        for (unit_name, source) in units {
            let path = dir.join(unit_name);
            std::fs::write(&path, source).unwrap();
            script = script.replace(&format!("@{unit_name}@"), path.to_str().unwrap());
        }
        let pidfile = dir.join("pid.txt");
        script = script.replace("@pidfile@", pidfile.to_str().unwrap());
        let main_path = dir.join("main.qn");
        std::fs::write(&main_path, &script).unwrap();

        let child = Command::new(env!("CARGO_BIN_EXE_qn"))
            .arg(&main_path)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("run qn");
        if kill_via_pidfile {
            // Wait for the script to publish its worker's pid, give the parked
            // call a beat to open, then deliver the hard death.
            let mut pid = String::new();
            for _ in 0..100 {
                if let Ok(s) = std::fs::read_to_string(&pidfile)
                    && !s.trim().is_empty()
                {
                    pid = s.trim().to_string();
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            assert!(!pid.is_empty(), "the script never published a pid");
            std::thread::sleep(std::time::Duration::from_millis(300));
            let _ = Command::new("kill").args(["-9", &pid]).status();
        }
        let out = child.wait_with_output().expect("qn output");
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
            std::thread::sleep(std::time::Duration::from_millis(200 * attempt as u64));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    panic!("peer-death script {name} did not pass after {ATTEMPTS} attempts.\n{last_diag}");
}

#[test]
fn terminated_worker_join_raises_peer_died() {
    // `terminate` kills the child; `join` must report the death as the typed
    // class with reason #exited and a non-nil peer name — this is also the
    // bootstrap doc example's shape, verified for real.
    let script = r#"
var w = Worker.start:{ Async.sleep:60000 } backing:'process';
w.terminate;
var tag = { w.join; 'no-error' }.catch:{ |e:PeerDiedError|
    ((e.reason == #exited) && (e.peer != nil)).if:{ 'died-exited' } else:{ 'bad-fields' } };
(tag == 'died-exited').if:{ 'PASS'.print } else:{ ('FAIL ' + tag).print };
"#;
    assert_passes("join", script, &[], false);
}

#[test]
fn service_hard_death_types_every_seam_and_marks_claims() {
    // SIGKILL the service's child while a call is parked mid-conversation:
    // the in-flight call raises the typed death; the next call fails fast with
    // the same class ("the service has exited"); and the peer's claim rows
    // carry the explicit post-mortem marker instead of implying death.
    let script = r#"
var svc = Worker.host:'@kaboom.qn@' with:{ Kaboom.new } backing:'process';
var pid = ((VM.ps.at:'workers').at:0).at:'pid';
[IO]File.write:pid.s to:'@pidfile@';
var t1 = { svc.sleepLong; 'no-error' }.catch:{ |e:PeerDiedError|
    (e.reason == #exited).if:{ 'died' } else:{ 'wrong-reason' } };
var t2 = { svc.ping; 'no-error' }.catch:{ |e:PeerDiedError| 'dead-refused' };
var gone = nil;
VM.claims.each:{ |p| ((p.at:'gone') == 'died').if:{ gone = 'marked' } };
((t1 == 'died') && (t2 == 'dead-refused') && (gone == 'marked'))
    .if:{ 'PASS'.print }
    else:{ ('FAIL ' + t1 + ' ' + t2 + ' ' + (gone == 'marked').s).print };
"#;
    let kaboom = r#"
Kaboom <- {
    sleepLong -> { Async.sleep:60000; 'unreachable' }
    ping -> { 'pong' }
}
"#;
    assert_passes("service", script, &[("kaboom.qn", kaboom)], true);
}

#[test]
fn link_death_purges_remote_channel_waiters() {
    // The child parks receiving on a shipped channel endpoint, then dies hard.
    // The owner must purge the dead remote receiver at link death: the later
    // send lands in the buffer for the local receiver. Without the purge, the
    // send pops the dead waiter and emits the value into the closed lane —
    // silently lost, and the local receive deadlocks this script.
    let script = r#"
var jobs = Channel.buffered:1;
var w = Worker.start:{ |ch| ch.receive; 'never' } args:#( jobs ) backing:'process';
"* let the child reach its endpoint receive (the remote waiter registers here)
Async.sleep:300;
var pid = ((VM.ps.at:'workers').at:0).at:'pid';
[IO]File.write:pid.s to:'@pidfile@';
var j = { w.join; 'no-error' }.catch:{ |e:PeerDiedError| 'join-died' };
"* the join has fired; give the relay agent a beat to process the link closure
Async.sleep:200;
jobs.send:'v1';
var got = jobs.receive;
((j == 'join-died') && (got == 'v1'))
    .if:{ 'PASS'.print } else:{ ('FAIL ' + j + ' ' + got.s).print };
"#;
    assert_passes("chan", script, &[], true);
}
