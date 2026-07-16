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
        script = script.replace("@dir@", dir.to_str().unwrap());
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
fn dead_worker_send_raises_peer_died() {
    // A send meeting an exited worker's closed inbox is the typed
    // unavailability error, not a bare string throw — the seam the web
    // pool's `catch:{ |e:PeerDiedError| }` route-around depends on (found
    // by the web soak: the untyped throw leaked through the pool to the
    // transport as a 500). The inbox close can lag `terminate` by a hair,
    // so sends retry until one is refused.
    let script = r#"
var w = Worker.start:{ Async.sleep:60000 } backing:'process';
w.terminate;
var tag = nil;
var tries = 0;
{ (tag == nil) && (tries < 500) }.whileDo:{
    tag = { w.send:'x'; Async.sleep:10; nil }
        .catch:{ |e:PeerDiedError|
            ((e.reason == #exited) && (e.peer != nil)).if:{ 'typed' } else:{ 'bad-fields' } }
        catch:{ |e| 'untyped [' + e.s + ']' };
    tries = tries + 1
};
(tag == 'typed').if:{ 'PASS'.print } else:{ ('FAIL ' + tag.s).print };
"#;
    assert_passes("send", script, &[], false);
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

#[test]
fn service_restart_rebinds_the_root_and_stales_the_rest() {
    // Slice 2 (SUPERVISION.md §4): restart refuses while running; after a hard
    // death, `serviceRestart` re-runs the frozen recipe and REBINDS the root in
    // place — new sends work; the dead incarnation's sub-proxy raises the typed
    // #staleIncarnation; the roster shows one row per incarnation.
    let script = r#"
var svc = Worker.host:'@kaboom.qn@' with:{ Kaboom.new } backing:'process';
var ok = true;
(svc.ping == 'pong').else:{ ok = false; 'e1'.print };
var sub = svc.mate;
(sub.ping == 'pong').else:{ ok = false; 'e2'.print };
var early = { svc.serviceRestart; 'no' }.catch:{ |e| 'refused-running' };
(early == 'refused-running').else:{ ok = false; 'e3'.print };
var pid = ((VM.ps.at:'workers').at:0).at:'pid';
[IO]File.write:pid.s to:'@pidfile@.tmp';
[IO]File.rename:'@pidfile@.tmp' to:'@pidfile@';
var t1 = { svc.sleepLong }.catch:{ |e:PeerDiedError| 'died' };
(t1 == 'died').else:{ ok = false; 'e4'.print };
svc.serviceRestart;
(svc.ping == 'pong').else:{ ok = false; 'e5'.print };
var t2 = { sub.ping }.catch:{ |e:PeerDiedError|
    (e.reason == #staleIncarnation).if:{ 'stale' } else:{ 'wrong-reason' } };
(t2 == 'stale').else:{ ok = false; ('e6 ' + t2).print };
var one = nil;
var two = nil;
VM.peers.each:{ |p| ((p.at:'kind') == 'hosted').if:{
    ((p.at:'incarnation') == 1).if:{ one = p.at:'status' };
    ((p.at:'incarnation') == 2).if:{ two = p.at:'status' } } };
((one == 'died') && (two == 'running')).else:{ ok = false; 'e7'.print };
ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;
    let kaboom = r#"
Kaboom <- {
    sleepLong -> { Async.sleep:60000; 'unreachable' }
    ping -> { 'pong' }
    mate -> { Kaboom.new }
}
"#;
    assert_passes("restart", script, &[("kaboom.qn", kaboom)], true);
}

#[test]
fn restart_window_parks_senders_into_the_new_incarnation() {
    // Rule 5: the restart task sets the gate synchronously before its first
    // park, so a send after sleep:1 provably lands INSIDE the window — it must
    // park through the respawn and answer from the fresh incarnation.
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
    assert_passes("window", script, &[("kaboom.qn", kaboom)], true);
}

#[test]
fn restart_manifest_gate_refuses_a_changed_recipe_outcome() {
    // Rule 9: a recipe whose re-run answers a DIFFERENT class refuses to
    // rebind with a clear error and leaves the service dead but retryable.
    let script = r#"
var marker = '@pidfile@.marker';
var svc = Worker.host:'@twoclass.qn@'
    with:{ |m| ([IO]File.exists?:m).if:{ KabB.new } else:{ KabA.new } }
    args:#( marker )
    backing:'process';
var ok = true;
(svc.ping == 'a').else:{ ok = false; 'e1'.print };
var pid = ((VM.ps.at:'workers').at:0).at:'pid';
[IO]File.write:pid.s to:'@pidfile@.tmp';
[IO]File.rename:'@pidfile@.tmp' to:'@pidfile@';
var t1 = { svc.sleepLong }.catch:{ |e:PeerDiedError| 'died' };
(t1 == 'died').else:{ ok = false; 'e2'.print };
[IO]File.write:'x' to:marker;
var r = { svc.serviceRestart; 'rebound' }.catch:{ |e| e.s };
(r.contains?:'does not match the installed class').else:{ ok = false; ('e3 ' + r).print };
var again = { svc.ping }.catch:{ |e:PeerDiedError| 'still-dead' };
(again == 'still-dead').else:{ ok = false; 'e4'.print };
[IO]File.delete:marker;
svc.serviceRestart;
(svc.ping == 'a').else:{ ok = false; 'e5'.print };
ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;
    let twoclass = r#"
KabA <- {
    sleepLong -> { Async.sleep:60000; 'unreachable' }
    ping -> { 'a' }
}
KabB <- {
    ping -> { 'b' }
}
"#;
    assert_passes("manifest", script, &[("twoclass.qn", twoclass)], true);
}

#[test]
fn channel_args_reship_across_restart() {
    // Rule 2: the recipe's channel arg is retained as a VALUE and re-ships
    // against the new incarnation's link — the fresh worker holds a live
    // endpoint to the SAME parent channel.
    let script = r#"
var pipe = Channel.buffered:8;
var svc = Worker.host:'@notifier.qn@' with:{ |ch| Notifier.new:{ var out = ch } }
    args:#( pipe ) backing:'process';
var ok = true;
(svc.poke == 'ok').else:{ ok = false; 'e1'.print };
(pipe.receive == 'hi').else:{ ok = false; 'e2'.print };
var pid = ((VM.ps.at:'workers').at:0).at:'pid';
[IO]File.write:pid.s to:'@pidfile@.tmp';
[IO]File.rename:'@pidfile@.tmp' to:'@pidfile@';
var t1 = { svc.sleepLong }.catch:{ |e:PeerDiedError| 'died' };
(t1 == 'died').else:{ ok = false; 'e3'.print };
svc.serviceRestart;
(svc.poke == 'ok').else:{ ok = false; 'e4'.print };
(pipe.receive == 'hi').else:{ ok = false; 'e5'.print };
ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;
    let notifier = r#"
Notifier <- { |@out|
    poke -> { @out.send:'hi'; 'ok' }
    sleepLong -> { Async.sleep:60000 }
}
"#;
    assert_passes("chanrecipe", script, &[("notifier.qn", notifier)], true);
}

#[test]
fn extension_restart_after_mid_call_crash() {
    // Slice 2b: `Extension.restart` refuses while running, and after a
    // mid-call crash (which the EOF-races-try_wait hardening must type even
    // before the exit is reap-visible) re-runs the frozen spawn recipe and
    // rebinds the handle in place — the same installed surface keeps working.
    let ext_bin = env!("CARGO_BIN_EXE_ext_crash");
    let script = format!(
        r#"
var e = Extension.spawn:'{ext_bin}';
var ok = true;
((e.call:'ping' with:'') == 'pong').else:{{ ok = false; 'e1'.print }};
var early = {{ e.restart; 'no' }}.catch:{{ |x| 'refused-running' }};
(early == 'refused-running').else:{{ ok = false; 'e2'.print }};
var crashed = {{ e.call:'crash' with:'' }}.catch:{{ |x:PeerDiedError| 'died' }};
(crashed == 'died').else:{{ ok = false; 'e3'.print }};
e.restart;
((e.call:'ping' with:'') == 'pong').else:{{ ok = false; 'e4'.print }};
var one = nil;
var two = nil;
VM.peers.each:{{ |p| ((p.at:'kind') == 'extension').if:{{
    ((p.at:'incarnation') == 1).if:{{ one = p.at:'status' }};
    ((p.at:'incarnation') == 2).if:{{ two = p.at:'status' }} }} }};
((one == 'died') && (two == 'running')).else:{{ ok = false; 'e5'.print }};
ok.if:{{ 'PASS'.print }} else:{{ 'FAIL'.print }};
"#
    );
    assert_passes("extrestart", &script, &[], false);
}

#[test]
fn extension_restart_stales_the_dead_incarnations_instances() {
    // Rule 6 for extension-backed classes: an instance minted by the dead
    // incarnation raises #staleIncarnation after the restart (its reap-queue
    // identity no longer matches); fresh instances work. The death is an
    // IDLE kill observed by the exit watch — nobody calls between death and
    // the events delivery.
    let ext_bin = env!("CARGO_BIN_EXE_ext_vector");
    let script = format!(
        r#"
var e = Extension.spawn:'{ext_bin}';
var ok = true;
var v = Vector.ofFloats:#( 1.0 2.0 );
(v.sum == 3).else:{{ ok = false; 'e1'.print }};
var pid = nil;
VM.peers.each:{{ |p| ((p.at:'kind') == 'extension').if:{{ pid = p.at:'pid' }} }};
[IO]File.write:pid.s to:'@pidfile@.tmp';
[IO]File.rename:'@pidfile@.tmp' to:'@pidfile@';
var ev = e.events;
ev.receive;
((ev.receive.at:'kind') == 'died').else:{{ ok = false; 'e2'.print }};
var t1 = {{ v.sum }}.catch:{{ |x:PeerDiedError| 'typed-dead' }};
(t1 == 'typed-dead').else:{{ ok = false; 'e3'.print }};
e.restart;
var t2 = {{ v.sum }}.catch:{{ |x:PeerDiedError|
    (x.reason == #staleIncarnation).if:{{ 'stale' }} else:{{ 'wrong-reason' }} }};
(t2 == 'stale').else:{{ ok = false; ('e4 ' + t2).print }};
var v2 = Vector.ofFloats:#( 3.0 4.0 );
(v2.sum == 7).else:{{ ok = false; 'e5'.print }};
ok.if:{{ 'PASS'.print }} else:{{ 'FAIL'.print }};
"#
    );
    assert_passes("extstale", &script, &[], true);
}

#[test]
fn supervised_service_restarts_itself() {
    // Slice 3: with a policy attached, a hard death needs NO manual restart —
    // the in-flight call errors typed (rule 4: never replayed), the very next
    // send parks through the supervisor's backoff+respawn (rule 5) and lands
    // on the new incarnation. Manual serviceRestart refuses: the policy owns
    // the cycle.
    let script = r#"
var svc = Worker.host:'@kaboom.qn@' with:{ Kaboom.new } backing:'process';
svc.serviceSupervise:(Supervise.always.backoff:10 cap:50);
var ok = true;
var early = { svc.serviceRestart; 'no' }.catch:{ |e| 'policy-owns' };
(early == 'policy-owns').else:{ ok = false; 'e1'.print };
var pid = ((VM.ps.at:'workers').at:0).at:'pid';
[IO]File.write:pid.s to:'@pidfile@.tmp';
[IO]File.rename:'@pidfile@.tmp' to:'@pidfile@';
var t1 = { svc.sleepLong }.catch:{ |e:PeerDiedError| 'died-typed' };
(t1 == 'died-typed').else:{ ok = false; 'e2'.print };
(svc.ping == 'pong').else:{ ok = false; 'e3'.print };
var two = nil;
VM.peers.each:{ |p| (((p.at:'kind') == 'hosted') && ((p.at:'incarnation') == 2)).if:{
    two = p.at:'status' } };
(two == 'running').else:{ ok = false; 'e4'.print };
ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;
    let kaboom = r#"
Kaboom <- {
    sleepLong -> { Async.sleep:60000; 'unreachable' }
    ping -> { 'pong' }
}
"#;
    assert_passes("supervised", script, &[("kaboom.qn", kaboom)], true);
}

#[test]
fn supervised_extension_gives_up_at_the_budget() {
    // Rule 7, fully deterministic (each `crash` call is a death, no harness
    // kill needed): two deaths restart inside the budget of max 2; the third
    // exceeds it — the supervisor GIVES UP, later calls raise the typed
    // #gaveUp, and the roster's last incarnation says 'gaveUp'.
    let ext_bin = env!("CARGO_BIN_EXE_ext_crash");
    let script = format!(
        r#"
var e = Extension.spawn:'{ext_bin}';
e.supervise:((Supervise.always.backoff:5 cap:10).max:2 within:60000);
var ok = true;
var c1 = {{ e.call:'crash' with:'' }}.catch:{{ |x:PeerDiedError| 'died' }};
(c1 == 'died').else:{{ ok = false; 'e1'.print }};
Async.sleep:100;
((e.call:'ping' with:'') == 'pong').else:{{ ok = false; 'e2'.print }};
var c2 = {{ e.call:'crash' with:'' }}.catch:{{ |x:PeerDiedError| 'died' }};
(c2 == 'died').else:{{ ok = false; 'e3'.print }};
Async.sleep:100;
((e.call:'ping' with:'') == 'pong').else:{{ ok = false; 'e4'.print }};
var c3 = {{ e.call:'crash' with:'' }}.catch:{{ |x:PeerDiedError| 'died' }};
(c3 == 'died').else:{{ ok = false; 'e5'.print }};
Async.sleep:200;
var after = {{ e.call:'ping' with:'' }}.catch:{{ |x:PeerDiedError|
    (x.reason == #gaveUp).if:{{ 'gave-up' }} else:{{ 'wrong-reason' }} }};
(after == 'gave-up').else:{{ ok = false; ('e6 ' + after).print }};
var st = nil;
VM.peers.each:{{ |p| ((p.at:'status') == 'gaveUp').if:{{ st = p.at:'incarnation' }} }};
(st == 3).else:{{ ok = false; 'e7'.print }};
ok.if:{{ 'PASS'.print }} else:{{ 'FAIL'.print }};
"#
    );
    assert_passes("giveup", &script, &[], false);
}

#[test]
fn package_toml_supervision_keys_attach() {
    // §10.5: package extensions declare their policy in quoin.toml — the
    // loader attaches it (watch armed, supervisor spawned) with no call site
    // anywhere. A crash respawns without user code.
    let ext_bin = env!("CARGO_BIN_EXE_ext_crash");
    let toml = format!(
        "[package]\nname = \"crashpkg\"\n\n[extension]\ncommand = \"{ext_bin}\"\n\
         restart = \"always\"\nbackoff-ms = 5\ncap-ms = 10\n"
    );
    let script = r#"
var e = Extension.loadPackage:'@dir@';
var ok = true;
var c1 = { e.call:'crash' with:'' }.catch:{ |x:PeerDiedError| 'died' };
(c1 == 'died').else:{ ok = false; 'e1'.print };
Async.sleep:100;
((e.call:'ping' with:'') == 'pong').else:{ ok = false; 'e2'.print };
ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;
    assert_passes("pkgtoml", script, &[("quoin.toml", &toml)], false);
}
