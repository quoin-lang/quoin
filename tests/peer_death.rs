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
//!
//! Slice 1 rides the same harness: lifecycle events (`events` /
//! `serviceEvents`), the `VM.peers` roster, and the extension exit watch —
//! whose whole point (gap c) is observing an idle death no call would find.

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
        let phasefile = dir.join("phase.txt");
        script = script.replace("@phasefile@", phasefile.to_str().unwrap());
        let main_path = dir.join("main.qn");
        std::fs::write(&main_path, &script).unwrap();

        let mut child = Command::new(env!("CARGO_BIN_EXE_qn"))
            .arg(&main_path)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("run qn");
        let worker_pid = if kill_via_pidfile {
            // Wait for the script to publish its worker's pid (rename-published,
            // so a partial write is never visible), give the parked call a beat
            // to open, then deliver the hard death.
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
            Some(pid)
        } else {
            None
        };
        // Bounded wait: a wedged qn must FAIL with its evidence, never hang the
        // suite for the job timeout to reap (the doc-check wedge lesson). 90s
        // dwarfs the slowest healthy run (~2s).
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(90);
        let mut timed_out = false;
        loop {
            if matches!(child.try_wait(), Ok(Some(_))) {
                break;
            }
            if std::time::Instant::now() > deadline {
                timed_out = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        // Probe the worker child BEFORE killing qn (qn's death takes the
        // worker with it): discriminates "the kill never landed" from "the
        // death was never detected".
        let worker_alive = timed_out
            && worker_pid.as_deref().is_some_and(|pid| {
                Command::new("kill")
                    .args(["-0", pid])
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false)
            });
        if timed_out {
            let _ = child.kill();
        }
        let out = child.wait_with_output().expect("qn output");
        let stdout = String::from_utf8_lossy(&out.stdout);
        if stdout.contains("PASS") && !timed_out {
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        // The phase file survives the kill (written through file I/O, not the
        // block-buffered stdout): the last line names where the script wedged.
        let phase = std::fs::read_to_string(dir.join("phase.txt")).unwrap_or_default();
        last_diag = format!(
            "timed out after 90s: {timed_out} (worker child still alive at timeout: \
             {worker_alive})\nphase file:\n{phase}\nstatus: {:?}\nstdout:\n{stdout}\nstderr:\n{}",
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
[IO]File.append:'hosted\n' to:'@phasefile@';
var pid = ((VM.ps.at:'workers').at:0).at:'pid';
[IO]File.write:pid.s to:'@pidfile@.tmp';
[IO]File.rename:'@pidfile@.tmp' to:'@pidfile@';
[IO]File.append:'pid-published\n' to:'@phasefile@';
var t1 = { svc.sleepLong; 'no-error' }.catch:{ |e:PeerDiedError|
    (e.reason == #exited).if:{ 'died' } else:{ 'wrong-reason' } };
[IO]File.append:('t1=' + t1 + '\n') to:'@phasefile@';
var t2 = { svc.ping; 'no-error' }.catch:{ |e:PeerDiedError| 'dead-refused' };
[IO]File.append:('t2=' + t2 + '\n') to:'@phasefile@';
var gone = nil;
VM.claims.each:{ |p| ((p.at:'gone') == 'died').if:{ gone = 'marked' } };
[IO]File.append:'claims-checked\n' to:'@phasefile@';
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
[IO]File.write:pid.s to:'@pidfile@.tmp';
[IO]File.rename:'@pidfile@.tmp' to:'@pidfile@';
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

#[test]
fn lifecycle_events_tell_the_stop_from_the_death() {
    // SUPERVISION.md slice 1: events + roster. A thread worker finishing is
    // 'stopped'; a terminated process worker is 'stopped'("terminated") on the
    // supervision surface even though `join` honestly raises the typed death;
    // history is kept for late consumers; asking twice answers one channel.
    let script = r#"
var ok = true;
var w1 = Worker.start:{ 41 + 1 };
(w1.join == 42).else:{ ok = false };
var ev1 = w1.events;
((ev1.receive.at:'kind') == 'spawned').else:{ ok = false; 'e1'.print };
((ev1.receive.at:'kind') == 'stopped').else:{ ok = false; 'e2'.print };
(ev1.receive == nil).else:{ ok = false; 'e3'.print };

var w2 = Worker.start:{ Async.sleep:60000 } backing:'process';
w2.terminate;
var j = { w2.join; 'no-error' }.catch:{ |e:PeerDiedError| 'typed' };
(j == 'typed').else:{ ok = false; 'e4'.print };
var ev2 = w2.events;
(w2.events == ev2).else:{ ok = false; 'e5'.print };
((ev2.receive.at:'kind') == 'spawned').else:{ ok = false; 'e6'.print };
var t = ev2.receive;
(((t.at:'kind') == 'stopped') && ((t.at:'message') == 'terminated'))
    .else:{ ok = false; ('e7 ' + t.s).print };

var stopped = 0;
VM.peers.each:{ |p| ((p.at:'status') == 'stopped').if:{ stopped = stopped + 1 } };
(stopped == 2).else:{ ok = false; 'e8'.print };
ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;
    assert_passes("events", script, &[], false);
}

#[test]
fn extension_idle_death_surfaces_through_the_exit_watch() {
    // Gap (c) closed: NOBODY calls the extension after it dies — the armed
    // exit watch alone must deliver the death event, type the next call's
    // fail-fast, and mark the roster row.
    let ext_bin = env!("CARGO_BIN_EXE_ext_crash");
    let script = format!(
        r#"
var e = Extension.spawn:'{ext_bin}';
((e.call:'ping' with:'') == 'pong').else:{{ 'FAIL ping'.print }};
var ev = e.events;
((ev.receive.at:'kind') == 'spawned').else:{{ 'FAIL spawned'.print }};
var pid = nil;
VM.peers.each:{{ |p| ((p.at:'kind') == 'extension').if:{{ pid = p.at:'pid' }} }};
[IO]File.write:pid.s to:'@pidfile@.tmp';
[IO]File.rename:'@pidfile@.tmp' to:'@pidfile@';
var d = ev.receive;
var deadCall = {{ e.call:'ping' with:'' }}.catch:{{ |ex:PeerDiedError| 'typed' }};
var marked = nil;
VM.peers.each:{{ |p| ((p.at:'kind') == 'extension').if:{{ marked = p.at:'status' }} }};
((((d.at:'kind') == 'died') && ((d.at:'reason') == #exited))
    && ((deadCall == 'typed') && (marked == 'died')))
    .if:{{ 'PASS'.print }}
    else:{{ ('FAIL ' + d.s + ' ' + deadCall.s + ' ' + marked.s).print }};
"#
    );
    assert_passes("extwatch", &script, &[], true);
}
