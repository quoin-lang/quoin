//! The native worker spawn/boot machinery: thread-backed workers boot a full runner
//! arena (`crate::runner`), process-backed workers ride a Unix socket — both
//! impossible on wasm32, so this whole file is a `not(wasm32)` `#[path]` child of
//! `worker.rs`. The portable types and snapshot/rebuild walkers stay in the parent.

use super::*;

/// Spawn a worker running the unit at `path` on its own OS thread. Returns
/// immediately with the parent's channel ends; boot/parse/run failures
/// travel the done lane. The thread is detached — its lifecycle is observed
/// through the lanes (`join`), and process exit ends unjoined workers.
pub fn spawn_worker(path: String) -> WorkerChannels {
    spawn_worker_with(move |link| run_worker_unit(&path, link))
}

/// Spawn a worker running a portable block (docs/internal/CONCURRENCY_ARCH.md §10):
/// same lanes, same lifecycle; `join` returns the BLOCK'S VALUE (copied),
/// unlike unit workers' nil.
pub fn spawn_worker_block(job: PortableBlock) -> WorkerChannels {
    spawn_worker_with(move |link| run_worker_block(job, link))
}

/// The shared thread + lane setup; `body` is the worker's whole life.
fn spawn_worker_with(
    body: impl FnOnce(WorkerLink) -> Result<WireData, String> + Send + 'static,
) -> WorkerChannels {
    let (inbox_tx, inbox_rx) = async_channel::unbounded();
    let (outbox_tx, outbox_rx) = async_channel::unbounded();
    let (done_tx, done_rx) = async_channel::bounded(1);
    let (control_tx, control_rx) = async_channel::unbounded();
    let id = SPAWNED.fetch_add(1, Ordering::Relaxed);
    std::thread::Builder::new()
        .name(format!("qn-worker-{id}"))
        .spawn(move || {
            // A panic anywhere in the worker (parser internals, VM bugs)
            // must not tear down the process silently — it becomes the
            // done-lane error. The closure owns everything it touches, so
            // unwind-safety is vacuous.
            let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                body(WorkerLink {
                    inbox_rx,
                    outbox_tx,
                    control_rx,
                    process: false,
                })
            }))
            .unwrap_or_else(|p| {
                let what = p
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_else(|| "unknown panic".to_string());
                Err(format!("worker panicked: {what}"))
            });
            COMPLETED.fetch_add(1, Ordering::Relaxed);
            let _ = done_tx.send_blocking(out);
        })
        .expect("spawn worker thread");
    WorkerChannels {
        inbox_tx,
        outbox_rx,
        done_rx,
        control_tx,
    }
}

/// The generic service loop appended to a hosted unit's source
/// (docs/internal/CONCURRENCY_ARCH.md §10 L4): instantiate the hosted class, report
/// ready, then serve calls forever — one at a time, actor-style. Calls are
/// reflective sends (`perform:args:`), so a missing method raises the same
/// MessageNotUnderstood a direct send would, and it travels back as the
/// reply's 'err'. A nil message (serviceStop) ends the loop; the unit then
/// completes and the done lane reports.
const SERVICE_LOOP_QN: &str = r#"
var svcHostInstance = @CLASS@.new;
Worker.send:#{ 'ready': true };
var svcHostRunning = true;
{ svcHostRunning }.whileDo:{
    var svcHostCall = Worker.receive;
    (svcHostCall == nil).if:{ svcHostRunning = false }
    else:{
        var svcHostReply = {
            #{ 'ret': (svcHostInstance.perform:(svcHostCall.at:'sel') args:(svcHostCall.at:'args')) }
        }.catch:{ |e| #{ 'err': e.s } };
        Worker.send:svcHostReply
    }
};
"#;

/// Spawn a SERVICE worker: the unit at `path` (which defines `class_name`)
/// plus the generic serve loop, compiled as one program.
pub fn spawn_worker_service(path: String, class_name: String) -> WorkerChannels {
    spawn_worker_with(move |link| run_worker_service(&path, &class_name, link))
}

pub(crate) fn run_worker_service(
    path: &str,
    class_name: &str,
    link: WorkerLink,
) -> Result<WireData, String> {
    // The class name is interpolated into synthesized source — insist on a
    // plain class identifier so a hostile string can't smuggle code.
    if class_name.is_empty()
        || !class_name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_uppercase())
        || !class_name.chars().all(|c| c.is_ascii_alphanumeric())
    {
        return Err(format!(
            "WorkerService: '{class_name}' is not a plain class name"
        ));
    }
    let unit_source = std::fs::read_to_string(PathBuf::from(path))
        .map_err(|e| format!("service unit {path}: {e}"))?;
    let source = format!(
        "{unit_source}
{}",
        SERVICE_LOOP_QN.replace("@CLASS@", class_name)
    );
    run_worker_source(path, &source, link)
}

/// The worker thread body: boot a fresh VM (builtins + full qnlib prelude,
/// exactly the `qn <file>` recipe), inject the link, compile and drive the
/// unit to completion. v1 join carries no payload (`Null` on success) —
/// results travel as messages.
fn run_worker_unit(path: &str, link: WorkerLink) -> Result<WireData, String> {
    let source = std::fs::read_to_string(PathBuf::from(path))
        .map_err(|e| format!("worker unit {path}: {e}"))?;
    run_worker_source(path, &source, link)
}

fn canonical_unit(path: &str) -> String {
    std::fs::canonicalize(path)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_string())
}

fn run_worker_source(path: &str, source: &str, link: WorkerLink) -> Result<WireData, String> {
    let ast = try_parse_quoin_string_named(source, path)
        .map_err(|e| format!("worker unit {path}: parse error: {e}"))?;
    let NodeValue::Program(program_node) = &ast.value else {
        return Err(format!("worker unit {path}: root AST is not a program"));
    };

    let mut arena = boot_worker_arena(link)?;
    let unit = canonical_unit(path);
    arena.mutate_root(|_mc, vm| vm.unit_path = Some(unit));

    let mut compile_err = None;
    arena.mutate_root(|mc, vm| {
        let mut compiler = unit_compiler();
        compiler.set_seen_types(vm.options.seen_types.clone());
        compiler.set_class_table(vm.options.class_table.clone());
        crate::class_table::populate_from_vm(vm, &vm.options.class_table);
        let program = match compiler.compile_program(program_node) {
            Ok(p) => p,
            Err(e) => {
                compile_err = Some(format!("worker unit {path}: compile error: {e}"));
                return;
            }
        };
        vm.report_type_warnings(compiler.diagnostics());
        compile_unit_aot(vm, &mut compiler);
        let main_block = vm.block_from_template(mc, std::sync::Arc::new(program), None, None);
        vm.start_block(mc, main_block, Vec::new(), None, None);
        install_main_task(mc, vm);
    });
    if let Some(msg) = compile_err {
        return Err(msg);
    }

    drive_main_task(&mut arena).map_err(|e| format!("worker unit {path}: {e}"))?;
    Ok(WireData::Null)
}

/// Boot a fresh worker VM: arena + native builtins + the full qnlib prelude
/// (the exact `qn <file>` recipe), with the parent link injected. Shared by
/// the unit and portable-block worker bodies.
fn boot_worker_arena(link: WorkerLink) -> Result<ReplArena, String> {
    let mut arena: ReplArena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
        let mut vm = VmState::new(mc, VmOptions::default());
        register_builtins(mc, &mut vm);
        vm.worker_link = Some(link);
        vm
    });
    arena.metrics().set_pacing(crate::vm::gc_pacing());

    for ast in prelude_asts() {
        let mut failed = None;
        arena.mutate_root(|mc, vm| {
            let NodeValue::Program(p) = &ast.value else {
                return;
            };
            match Compiler::new().with_template_ids().compile_program(p) {
                Ok(sb) => {
                    let block = build_block(mc, &sb);
                    if let Err(e) = vm.execute_block(mc, block, Vec::new(), None) {
                        failed = Some(format!("worker prelude failed: {e}"));
                    }
                }
                Err(e) => failed = Some(format!("worker prelude compile error: {e}")),
            }
        });
        if let Some(msg) = failed {
            return Err(msg);
        }
    }
    Ok(arena)
}

/// The portable-block worker body: boot, verify the block's global
/// references against THIS VM's globals (clear error over silent nil),
/// rebuild the closure over a snapshot env frame, drive it as the main
/// task, and copy its value back for `join`.
fn run_worker_block(job: PortableBlock, link: WorkerLink) -> Result<WireData, String> {
    let mut arena = boot_worker_arena(link)?;

    let mut start_err = None;
    arena.mutate_root(|mc, vm| {
        let env = match rebuild_env(vm, mc, &job) {
            Ok(env) => env,
            Err(e) => {
                start_err = Some(e);
                return;
            }
        };
        let block = vm.block_from_template(mc, localize_template(&job.template), Some(env), None);
        vm.start_block(mc, block, Vec::new(), None, None);
        install_main_task(mc, vm);
    });
    if let Some(msg) = start_err {
        return Err(msg);
    }

    drive_main_task(&mut arena).map_err(|e| format!("worker block: {e}"))?;

    // The completed main task leaves the block's value on the stack top.
    arena.mutate_root(|_mc, vm| {
        let v = vm.stack.last().copied().unwrap_or(Value::Nil);
        value_to_wire(v, None)
            .map_err(|e| format!("the worker block's result is not portable data: {e}"))
    })
}

// =====================================================================
// Process backing (docs/internal/CONCURRENCY_ARCH.md §13.1): the SAME lanes, with a
// PUMP on the other end — four small threads bridging the channels over a
// unix socket (length-prefixed msgpack `DataValue` frames) to a child
// `qn worker-serve` process, whose own pump feeds identical channels into
// the same worker body. Everything above the lanes (handles, registry,
// services, psTree) inherits process backing by construction.
// =====================================================================

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};

/// One wire frame, itself a `DataValue` map:
/// d=data, k=done-ok, e=done-err, c=control, r=control-reply.
fn frame_write(sock: &mut UnixStream, frame: &WireData) -> std::io::Result<()> {
    let bytes = quoin_ext_proto::codec::pack_dv(frame);
    sock.write_all(&(bytes.len() as u32).to_le_bytes())?;
    sock.write_all(&bytes)
}

fn frame_read(sock: &mut UnixStream) -> std::io::Result<WireData> {
    let mut len = [0u8; 4];
    sock.read_exact(&mut len)?;
    let mut buf = vec![0u8; u32::from_le_bytes(len) as usize];
    sock.read_exact(&mut buf)?;
    quoin_ext_proto::codec::unpack_dv(&buf)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

fn fget<'a>(pairs: &'a [(String, WireData)], key: &str) -> Option<&'a WireData> {
    pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

/// Spawn a PROCESS-backed worker running `unit` (with `service` naming a
/// hosted class for the WorkerService form). Returns the standard channel
/// ends plus the child's pid; the pump threads own the socket.
pub fn spawn_worker_process(
    unit: String,
    service: Option<String>,
) -> Result<(WorkerChannels, u32, ChildGrip), String> {
    let sock_path = format!(
        "/tmp/quoin-worker-{}-{}.sock",
        std::process::id(),
        SPAWNED.fetch_add(1, Ordering::Relaxed)
    );
    let _ = std::fs::remove_file(&sock_path);
    let listener = UnixListener::bind(&sock_path).map_err(|e| format!("worker socket: {e}"))?;

    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("worker-serve").arg(&sock_path).arg(&unit);
    if let Some(class) = &service {
        cmd.arg(class);
    }
    let child = cmd
        .spawn()
        .map_err(|e| format!("spawn worker process: {e}"))?;
    let pid = child.id();
    let grip: ChildGrip = std::sync::Arc::new(std::sync::Mutex::new(Some(child)));

    // Accept with a bounded wait: poll the listener in nonblocking mode so a
    // child that dies pre-connect becomes an error, not a hang.
    listener
        .set_nonblocking(true)
        .map_err(|e| format!("worker socket: {e}"))?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let sock = loop {
        match listener.accept() {
            Ok((s, _)) => break s,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                let mut slot = grip.lock().expect("child grip");
                if let Some(c) = slot.as_mut()
                    && let Ok(Some(status)) = c.try_wait()
                {
                    let _ = std::fs::remove_file(&sock_path);
                    return Err(format!(
                        "worker process exited before connecting ({status})"
                    ));
                }
                if std::time::Instant::now() > deadline {
                    if let Some(c) = slot.as_mut() {
                        let _ = c.kill();
                    }
                    let _ = std::fs::remove_file(&sock_path);
                    return Err("worker process did not connect within 10s".to_string());
                }
                drop(slot);
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Err(e) => {
                let _ = std::fs::remove_file(&sock_path);
                return Err(format!("worker socket accept: {e}"));
            }
        }
    };
    sock.set_nonblocking(false).ok();
    let _ = std::fs::remove_file(&sock_path);

    let (inbox_tx, inbox_rx) = async_channel::unbounded::<WorkerMsg>();
    let (outbox_tx, outbox_rx) = async_channel::unbounded::<WorkerMsg>();
    let (done_tx, done_rx) = async_channel::bounded::<Result<WireData, String>>(1);
    let (control_tx, control_rx) = async_channel::unbounded::<ControlReq>();

    // Single write path: every to-child frame funnels through one channel.
    let (to_sock_tx, to_sock_rx) = std::sync::mpsc::channel::<WireData>();
    let ctl_map: std::sync::Arc<
        std::sync::Mutex<std::collections::HashMap<u64, async_channel::Sender<WorkerMsg>>>,
    > = std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));

    // fwd: parent sends -> data frames
    {
        let to_sock = to_sock_tx.clone();
        std::thread::spawn(move || {
            while let Ok(msg) = inbox_rx.recv_blocking() {
                match msg {
                    WorkerMsg::Data(dv) => {
                        if to_sock
                            .send(WireData::Map(vec![
                                ("t".into(), WireData::Str("d".into())),
                                ("v".into(), dv),
                            ]))
                            .is_err()
                        {
                            break;
                        }
                    }
                    // Refused at the send seams; unreachable in practice.
                    WorkerMsg::Block(_) => {
                        eprintln!("qn: dropped a block on a process-worker lane");
                    }
                }
            }
        });
    }
    // fwd: control requests -> ctl frames (reply senders parked in the map)
    {
        let to_sock = to_sock_tx.clone();
        let map = ctl_map.clone();
        std::thread::spawn(move || {
            let mut next_id: u64 = 1;
            while let Ok(req) = control_rx.recv_blocking() {
                let id = next_id;
                next_id += 1;
                map.lock().expect("ctl map").insert(id, req.reply);
                let kind = match req.kind {
                    ControlKind::PsTree => "ps",
                };
                if to_sock
                    .send(WireData::Map(vec![
                        ("t".into(), WireData::Str("c".into())),
                        ("i".into(), WireData::Int(id as i64)),
                        ("k".into(), WireData::Str(kind.into())),
                    ]))
                    .is_err()
                {
                    break;
                }
            }
        });
    }
    // writer: the only thread that touches the socket's write half
    {
        let mut wsock = sock.try_clone().map_err(|e| format!("socket clone: {e}"))?;
        std::thread::spawn(move || {
            while let Ok(frame) = to_sock_rx.recv() {
                if frame_write(&mut wsock, &frame).is_err() {
                    break;
                }
            }
            let _ = wsock.shutdown(std::net::Shutdown::Write);
        });
    }
    // reader: frames -> lanes; EOF closes everything and reaps the child
    {
        let mut rsock = sock;
        let map = ctl_map;
        let reader_grip = grip.clone();
        std::thread::spawn(move || {
            let mut done_sent = false;
            while let Ok(frame) = frame_read(&mut rsock) {
                let WireData::Map(pairs) = frame else {
                    continue;
                };
                match fget(&pairs, "t") {
                    Some(WireData::Str(t)) if t == "d" => {
                        if let Some(v) = fget(&pairs, "v") {
                            let _ = outbox_tx.send_blocking(WorkerMsg::Data(v.clone()));
                        }
                    }
                    Some(WireData::Str(t)) if t == "k" => {
                        let v = fget(&pairs, "v").cloned().unwrap_or(WireData::Null);
                        let _ = done_tx.send_blocking(Ok(v));
                        done_sent = true;
                    }
                    Some(WireData::Str(t)) if t == "e" => {
                        let m = match fget(&pairs, "m") {
                            Some(WireData::Str(m)) => m.clone(),
                            _ => "worker process error".to_string(),
                        };
                        let _ = done_tx.send_blocking(Err(m));
                        done_sent = true;
                    }
                    Some(WireData::Str(t)) if t == "r" => {
                        if let (Some(WireData::Int(id)), Some(v)) =
                            (fget(&pairs, "i"), fget(&pairs, "v"))
                            && let Some(reply) = map.lock().expect("ctl map").remove(&(*id as u64))
                        {
                            let _ = reply.send_blocking(WorkerMsg::Data(v.clone()));
                        }
                    }
                    _ => {}
                }
            }
            if !done_sent {
                let status = reader_grip
                    .lock()
                    .expect("child grip")
                    .as_mut()
                    .and_then(|c| c.try_wait().ok().flatten())
                    .map(|s| format!(" ({s})"))
                    .unwrap_or_default();
                let _ = done_tx.send_blocking(Err(format!("worker process exited{status}")));
            }
            map.lock().expect("ctl map").clear();
            COMPLETED.fetch_add(1, Ordering::Relaxed);
            // Reap; leave None so a late `terminate` is a clean no-op.
            if let Some(mut c) = reader_grip.lock().expect("child grip").take() {
                let _ = c.wait();
            }
        });
    }

    Ok((
        WorkerChannels {
            inbox_tx,
            outbox_rx,
            done_rx,
            control_tx,
        },
        pid,
        grip,
    ))
}

/// The CHILD entry (`qn worker-serve <sock> <unit> [<serviceClass>]`):
/// connect back, bridge socket<->lanes with the mirror pumps, run the
/// standard worker body, ship the done frame.
pub fn worker_serve_main(sock_path: &str, unit: &str, service: Option<&str>) -> i32 {
    let sock = match UnixStream::connect(sock_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("qn worker-serve: cannot connect {sock_path}: {e}");
            return 1;
        }
    };
    let (inbox_tx, inbox_rx) = async_channel::unbounded::<WorkerMsg>();
    let (outbox_tx, outbox_rx) = async_channel::unbounded::<WorkerMsg>();
    let (control_tx, control_rx) = async_channel::unbounded::<ControlReq>();
    let (to_sock_tx, to_sock_rx) = std::sync::mpsc::channel::<WireData>();
    // Control replies ride ONE lane, correlated FIFO: the driver services
    // its queue in order, so ids pop in the order the reader pushed them.
    let (ctl_reply_tx, ctl_reply_rx) = async_channel::unbounded::<WorkerMsg>();
    let ctl_ids: std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<u64>>> =
        std::sync::Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new()));

    // reader: frames -> inbox / control
    {
        let mut rsock = match sock.try_clone() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("qn worker-serve: socket clone: {e}");
                return 1;
            }
        };
        let ids = ctl_ids.clone();
        std::thread::spawn(move || {
            while let Ok(frame) = frame_read(&mut rsock) {
                let WireData::Map(pairs) = frame else {
                    continue;
                };
                match fget(&pairs, "t") {
                    Some(WireData::Str(t)) if t == "d" => {
                        if let Some(v) = fget(&pairs, "v") {
                            let _ = inbox_tx.send_blocking(WorkerMsg::Data(v.clone()));
                        }
                    }
                    Some(WireData::Str(t)) if t == "c" => {
                        if let Some(WireData::Int(id)) = fget(&pairs, "i") {
                            ids.lock().expect("ctl ids").push_back(*id as u64);
                            let _ = control_tx.send_blocking(ControlReq {
                                kind: ControlKind::PsTree,
                                reply: ctl_reply_tx.clone(),
                            });
                        }
                    }
                    _ => {}
                }
            }
            // Parent gone: closing inbox_tx (drop) ends Worker.receive with nil.
        });
    }
    // fwd: worker sends -> data frames
    {
        let to_sock = to_sock_tx.clone();
        std::thread::spawn(move || {
            while let Ok(msg) = outbox_rx.recv_blocking() {
                if let WorkerMsg::Data(dv) = msg
                    && to_sock
                        .send(WireData::Map(vec![
                            ("t".into(), WireData::Str("d".into())),
                            ("v".into(), dv),
                        ]))
                        .is_err()
                {
                    break;
                }
            }
        });
    }
    // fwd: control replies -> ctlr frames (FIFO id correlation)
    {
        let to_sock = to_sock_tx.clone();
        let ids = ctl_ids;
        std::thread::spawn(move || {
            while let Ok(msg) = ctl_reply_rx.recv_blocking() {
                let id = ids.lock().expect("ctl ids").pop_front().unwrap_or(0);
                if let WorkerMsg::Data(dv) = msg
                    && to_sock
                        .send(WireData::Map(vec![
                            ("t".into(), WireData::Str("r".into())),
                            ("i".into(), WireData::Int(id as i64)),
                            ("v".into(), dv),
                        ]))
                        .is_err()
                {
                    break;
                }
            }
        });
    }
    // writer — `Null` is the close sentinel: the fwd threads hold sender
    // clones (one is pinned on a reader that outlives us), so the channel
    // never closes on its own; the sentinel lets the done frame flush and
    // the writer finish before the process exits.
    let writer = {
        let mut wsock = sock;
        std::thread::spawn(move || {
            while let Ok(frame) = to_sock_rx.recv() {
                if matches!(frame, WireData::Null) {
                    break;
                }
                if frame_write(&mut wsock, &frame).is_err() {
                    break;
                }
            }
            let _ = wsock.shutdown(std::net::Shutdown::Write);
        })
    };

    let link = WorkerLink {
        inbox_rx,
        outbox_tx,
        control_rx,
        process: true,
    };
    let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match service {
        Some(class) => run_worker_service(unit, class, link),
        None => run_worker_unit(unit, link),
    }))
    .unwrap_or_else(|p| {
        let what = p
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_else(|| "unknown panic".to_string());
        Err(format!("worker panicked: {what}"))
    });
    let done = match &out {
        Ok(v) => WireData::Map(vec![
            ("t".into(), WireData::Str("k".into())),
            ("v".into(), v.clone()),
        ]),
        Err(m) => WireData::Map(vec![
            ("t".into(), WireData::Str("e".into())),
            ("m".into(), WireData::Str(m.clone())),
        ]),
    };
    let _ = to_sock_tx.send(done);
    let _ = to_sock_tx.send(WireData::Null); // close sentinel (see writer)
    drop(to_sock_tx);
    let _ = writer.join();
    // Exit now: the reader/fwd threads are parked on a socket the PARENT
    // still holds open; returning would leave this process lingering.
    i32::from(out.is_err())
}
