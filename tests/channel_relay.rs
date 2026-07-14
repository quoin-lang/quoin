//! Cross-isolate channels (docs/internal/ACTOR_OBJECTS.md §6): endpoints ship
//! across worker links (plain lanes, service args, service returns, block
//! answers) and relay ops preserve channel semantics — FIFO handoff,
//! backpressure via delayed acks, close propagation both ways, cancellation
//! without value loss — with fan-in/fan-out across several isolates.

use std::process::Command;

fn assert_channel_script_passes(name: &str, script: &str, units: &[(&str, &str)]) {
    const ATTEMPTS: u32 = 4;
    let dir = std::env::temp_dir().join(format!("qn_chan_{name}"));
    std::fs::create_dir_all(&dir).unwrap();
    let mut script = script.to_string();
    for (unit_name, source) in units {
        let path = dir.join(unit_name);
        std::fs::write(&path, source).unwrap();
        script = script.replace(&format!("@{unit_name}@"), path.to_str().unwrap());
    }
    let main_path = dir.join("main.qn");
    std::fs::write(&main_path, &script).unwrap();

    let mut last_diag = String::new();
    for attempt in 1..=ATTEMPTS {
        let out = Command::new(env!("CARGO_BIN_EXE_qn"))
            .arg(&main_path)
            .output()
            .expect("run qn");
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
            std::thread::sleep(std::time::Duration::from_millis(150 * attempt as u64));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    panic!("channel script {name} did not pass after {ATTEMPTS} attempts.\n{last_diag}");
}

/// The worker-pool pattern §6 exists for: N plain workers all consuming ONE
/// parent-owned jobs channel and feeding ONE results channel — fan-out of
/// endpoints, fan-in of values, `each:` ending on close, nothing lost.
#[test]
fn channel_fan_out_worker_pool() {
    const CONSUMER: &str = r#"
var jobs = Worker.receive;
var results = Worker.receive;
jobs.each:{ |j| results.send:(j * 2) };
results.send:(0 - 1);
"#;
    let script = r#"
var ok = true;
var jobs = Channel.buffered:4;
var results = Channel.buffered:4;
var w1 = Worker.spawn:'@consumer.qn@';
var w2 = Worker.spawn:'@consumer.qn@';
w1.send:jobs; w1.send:results;
w2.send:jobs; w2.send:results;
(1..11).each:{ |i| jobs.send:i };
jobs.close;
var sum = 0;
var enders = 0;
{ enders < 2 }.whileDo:{
    var v = results.receive;
    (v < 0).if:{ enders = enders + 1 } else:{ sum = sum + v };
};
(sum == 110).else:{ ok = false; ('FAIL sum: ' + sum.s).print };
w1.join; w2.join;
ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;
    assert_channel_script_passes("fanout", script, &[("consumer.qn", CONSUMER)]);
}

/// Channels cross every hosted-service seam: as method ARGUMENTS (the worker
/// pumps into a parent-owned channel), as method RETURNS (a worker-owned
/// channel comes back as a live endpoint the parent drains), and as a parent
/// BLOCK's answer to worker code (the host-op path).
#[test]
fn channel_service_args_returns_and_block_answers() {
    const PUMP_UNIT: &str = r#"
Pump <- { |@out|
    init -> { @out = nil };
    fill:upTo: -> { |ch n|
        var i = 1;
        { i < (n + 1) }.whileDo:{ ch.send:(i * 10); i = i + 1 };
        ch.close;
        'filled'
    };
    stream: -> { |n|
        var ch = Channel.buffered:2;
        @out = ch;
        Task.spawn:{
            var i = 1;
            { i < (n + 1) }.whileDo:{ ch.send:i; i = i + 1 };
            ch.close
        };
        ch
    };
    drainVia: -> { |blk|
        var ch = blk.value:0;
        var got = 0;
        ch.each:{ |v| got = got + v };
        got
    }
};
"#;
    let script = r#"
var ok = true;
var p = Worker.host:'@pump.qn@' class:'Pump';

"* argument: the worker fills a PARENT-owned channel and closes it
var sink = Channel.buffered:2;
var t = Task.spawn:{ p.fill:sink upTo:4 };
var got = 0;
sink.each:{ |v| got = got + v };
(got == 100).else:{ ok = false; ('FAIL fill: ' + got.s).print };
(t.join == 'filled').else:{ ok = false; 'FAIL: fill return'.print };

"* return: a WORKER-owned channel comes back as a live endpoint
var stream = p.stream:5;
var streamed = 0;
stream.each:{ |v| streamed = streamed + v };
(streamed == 15).else:{ ok = false; ('FAIL stream: ' + streamed.s).print };

"* block answer: worker code asks a parent block for a channel, then PUMPS
"* FROM it (a parent-owned channel used inside the worker)
var feed = Channel.buffered:8;
(1..4).each:{ |i| feed.send:i };
feed.close;
((p.drainVia:{ |n| feed }) == 6).else:{ ok = false; 'FAIL: drainVia'.print };

p.serviceStop;
ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;
    assert_channel_script_passes("service", script, &[("pump.qn", PUMP_UNIT)]);
}

/// The service seams again, over PROCESS backing: channel args, returns, and
/// block answers all relay across the sockets (the `Arg::Chan` /
/// `CallReturnChannel` / `Msg::Chan` wire forms end to end).
#[test]
fn channel_service_seams_process() {
    const PUMP_UNIT: &str = r#"
Pump <- { |@out|
    init -> { @out = nil };
    fill:upTo: -> { |ch n|
        var i = 1;
        { i < (n + 1) }.whileDo:{ ch.send:(i * 10); i = i + 1 };
        ch.close;
        'filled'
    };
    stream: -> { |n|
        var ch = Channel.buffered:2;
        @out = ch;
        Task.spawn:{
            var i = 1;
            { i < (n + 1) }.whileDo:{ ch.send:i; i = i + 1 };
            ch.close
        };
        ch
    };
    drainVia: -> { |blk|
        var ch = blk.value:0;
        var got = 0;
        ch.each:{ |v| got = got + v };
        got
    }
};
"#;
    let script = r#"
var ok = true;
var p = Worker.host:'@pump.qn@' class:'Pump' backing:'process';

var sink = Channel.buffered:2;
var t = Task.spawn:{ p.fill:sink upTo:4 };
var got = 0;
sink.each:{ |v| got = got + v };
(got == 100).else:{ ok = false; ('FAIL fill: ' + got.s).print };
(t.join == 'filled').else:{ ok = false; 'FAIL: fill return'.print };

var stream = p.stream:5;
var streamed = 0;
stream.each:{ |v| streamed = streamed + v };
(streamed == 15).else:{ ok = false; ('FAIL stream: ' + streamed.s).print };

var feed = Channel.buffered:8;
(1..4).each:{ |i| feed.send:i };
feed.close;
((p.drainVia:{ |n| feed }) == 6).else:{ ok = false; 'FAIL: drainVia'.print };

p.serviceStop;
ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;
    assert_channel_script_passes("service_proc", script, &[("pump.qn", PUMP_UNIT)]);
}

/// Backpressure and rendezvous across the boundary: a cap-0 send parks until
/// the remote receiver takes it, and close/error semantics survive relaying.
#[test]
fn channel_backpressure_close_and_refusals() {
    const SLOWPOKE: &str = r#"
var ch = Worker.receive;
var back = Worker.receive;
Async.sleep:150;
var v = ch.receive;
back.send:v;
"* send on a channel the parent CLOSED raises here, catchably
var msg = { ch.send:99; 'sent' }.catch:{ |e| 'refused' };
back.send:msg;
"* and this worker can close a PARENT-owned channel
back.close;
"#;
    let script = r#"
var ok = true;
var ch = Channel.new;
var back = Channel.buffered:4;
var w = Worker.spawn:'@slowpoke.qn@';
w.send:ch; w.send:back;

"* rendezvous across the boundary: the send parks ~150ms until the worker
"* receives — 'sent' must come after 'pre'
var order = List.new;
var t = Task.spawn:{ ch.send:42; order.add:'sent' };
Async.sleep:40;
order.add:'pre';
t.join;
((order.at:0) == 'pre').else:{ ok = false; ('FAIL order: ' + order.s).print };
((back.receive) == 42).else:{ ok = false; 'FAIL: rendezvous value'.print };

"* close propagates remote-ward: the worker's next send refuses
ch.close;
((back.receive) == 'refused').else:{ ok = false; 'FAIL: remote closed send'.print };

"* close propagates parent-ward: the worker closed `back`, so receive is nil
((back.receive) == nil).else:{ ok = false; 'FAIL: remote close'.print };

"* a shipped channel refuses non-portable values AT THE SENDER, catchably
var ch2 = Channel.buffered:2;
w.join;
var w2 = Worker.spawn:'@sink.qn@';
w2.send:ch2;
var bad2 = { ch2.send:(Marker.new); 'sent' }.catch:{ |e| 'refused' };
(bad2 == 'refused').else:{ ok = false; 'FAIL: non-portable send'.print };
ch2.send:1;
ch2.close;
w2.join;
ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;
    let script = format!("Marker <- {{ x -> {{ 1 }} }};\n{script}");
    const SINK: &str = r#"
var ch = Worker.receive;
ch.each:{ |v| v };
"#;
    assert_channel_script_passes(
        "backpressure",
        &script,
        &[("slowpoke.qn", SLOWPOKE), ("sink.qn", SINK)],
    );
}

/// Cancellation without value loss: a timed-out remote receive retracts its
/// pending op; a value sent afterwards is still there for the next receive.
/// Re-shipping still refuses with a clear error.
#[test]
fn channel_cancellation_and_v1_refusals() {
    const WAITER: &str = r#"
var ch = Worker.receive;
var back = Worker.receive;
"* a timed-out relay receive must not eat a later value
var t = { Async.timeout:40 do:{ ch.receive } ; 'no-timeout' }.catch:{ |e| 'timed-out' };
back.send:t;
back.send:(ch.receive);
"* re-shipping a relay endpoint refuses clearly
var re = { Worker.send:ch; 'shipped' }.catch:{ |e| 'refused' };
back.send:re;
"#;
    let script = r#"
var ok = true;
var ch = Channel.buffered:2;
var back = Channel.buffered:4;
var w = Worker.spawn:'@waiter.qn@';
w.send:ch; w.send:back;

((back.receive) == 'timed-out').else:{ ok = false; 'FAIL: no timeout'.print };
ch.send:7;
((back.receive) == 7).else:{ ok = false; 'FAIL: value after cancel'.print };
((back.receive) == 'refused').else:{ ok = false; 'FAIL: re-ship allowed'.print };
w.join;

"* channels cross PROCESS links too: the worker doubles values from one
"* parent channel into another, over the relay socket
var pin = Channel.buffered:2;
var pout = Channel.buffered:2;
var pw = Worker.spawn:'@doubler.qn@' backing:'process';
pw.send:pin;
pw.send:pout;
(1..4).each:{ |i| pin.send:i };
pin.close;
var psum = 0;
pout.each:{ |v| psum = psum + v };
(psum == 12).else:{ ok = false; ('FAIL psum: ' + psum.s).print };
pw.join;

ok.if:{ 'PASS'.print } else:{ 'FAIL'.print };
"#;
    const NOOP: &str = r#"
var pin = Worker.receive;
var pout = Worker.receive;
pin.each:{ |v| pout.send:(v * 2) };
pout.close;
"#;
    assert_channel_script_passes(
        "cancel",
        script,
        &[("waiter.qn", WAITER), ("doubler.qn", NOOP)],
    );
}
