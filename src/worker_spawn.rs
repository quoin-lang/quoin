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
    // Thread backing: the dispatch lane IS the transport — owned `Msg` values,
    // no pump, no bytes (ACTOR_OBJECTS.md §1).
    let (dispatch_tx, dispatch_rx) = async_channel::unbounded();
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
                    dispatch_rx,
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
        dispatch_tx,
    }
}

/// The hosting line appended to a hosted unit's source: `Worker.hostServe:`
/// (src/runtime/worker.rs) instantiates the class, roots it in the worker's
/// hosted-object table, reports ready, and serves peer-protocol `Call`
/// dispatches from the dispatch lane — one at a time, actor-style — until
/// the reserved stop op or the lane closing ends it.
const SERVICE_LOOP_QN: &str = "\nWorker.hostServe:'@CLASS@';\n";

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
// Process backing (docs/internal/CONCURRENCY_ARCH.md §13.1, converged per
// ACTOR_OBJECTS.md §1/§4): the SAME lanes, bridged to a child
// `qn worker-serve` process over TWO unix sockets speaking the extension
// protocol's `Msg` frames (u32-LE + msgpack, `quoin-ext-proto`) — one
// protocol, three peer kinds; the bespoke `{t,v}` envelope is retired.
//
// The two sockets are two LANES with different disciplines (lanes, never
// frame-multiplexing — the §5 rule):
//
// - The CONVERSATION socket: extension-protocol discipline verbatim,
//   parent = host. Opens with the `GetManifest`/`ManifestReturn` version
//   handshake (the gate workers previously lacked; provided classes stay
//   empty until hosted objects). Control requests are conversations —
//   `Call{op:"psTree"}` answered by `CallReturnData` — one at a time, so
//   the old request-id correlation machinery is gone on both sides.
//   Hosted-object dispatch (arc-2 slice 4) rides this socket next.
//
// - The MAILBOX socket: the async flows as ONE long-lived implicit
//   conversation — the spawn is the "call". `Worker.send:` in either
//   direction is an intermediate `Call{op:"send", data}` frame (fire and
//   forget: an enqueue-ack from a pump thread would prove nothing;
//   real cross-isolate backpressure is the channel-relay design), and the
//   child's done report is the conversation's TERMINAL: `CallReturnData`
//   with the join value, or `CallReturnError` on failure (whose
//   `remote_stack` slot the structured-stacks work will fill).
//
// Everything above the lanes (handles, registry, services, psTree)
// inherits process backing by construction, unchanged.
// =====================================================================

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};

use quoin_ext_proto::{Msg, PROTOCOL_VERSION};

/// A bare `Call` frame carrying only an op and an optional data payload —
/// the worker link's message shape.
fn call_frame(op: &str, data: Option<WireData>) -> Msg {
    Msg::Call {
        op: op.to_string(),
        arg: String::new(),
        handles: Vec::new(),
        resources: Vec::new(),
        releases: Vec::new(),
        arrays: Vec::new(),
        data,
        class_name: String::new(),
        recv: 0,
        method_args: Vec::new(),
    }
}

fn write_msg_frame(sock: &mut UnixStream, msg: &Msg) -> std::io::Result<()> {
    let bytes = quoin_ext_proto::encode(msg);
    sock.write_all(&(bytes.len() as u32).to_le_bytes())?;
    sock.write_all(&bytes)
}

fn read_msg_frame(sock: &mut UnixStream) -> std::io::Result<Msg> {
    let mut len = [0u8; 4];
    sock.read_exact(&mut len)?;
    let mut buf = vec![0u8; u32::from_le_bytes(len) as usize];
    sock.read_exact(&mut buf)?;
    quoin_ext_proto::decode_frame(&buf)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
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
    // child that dies pre-connect becomes an error, not a hang. The child
    // connects TWICE, in a fixed order: conversation socket, then mailbox.
    listener
        .set_nonblocking(true)
        .map_err(|e| format!("worker socket: {e}"))?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let accept_one = |listener: &UnixListener| -> Result<UnixStream, String> {
        loop {
            match listener.accept() {
                Ok((s, _)) => {
                    s.set_nonblocking(false).ok();
                    return Ok(s);
                }
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
        }
    };
    let mut conv_sock = accept_one(&listener)?;
    let mail_sock = accept_one(&listener)?;
    let _ = std::fs::remove_file(&sock_path);

    // The manifest handshake — the version gate (mirrors the extension spawn:
    // this side enforces; a mismatched child is killed, never misdecoded).
    // Bounded, so a wedged child is an error rather than a hang.
    let fail = |grip: &ChildGrip, msg: String| -> String {
        if let Some(c) = grip.lock().expect("child grip").as_mut() {
            let _ = c.kill();
        }
        msg
    };
    conv_sock
        .set_read_timeout(Some(std::time::Duration::from_secs(10)))
        .ok();
    write_msg_frame(
        &mut conv_sock,
        &Msg::GetManifest {
            version: PROTOCOL_VERSION,
        },
    )
    .map_err(|e| fail(&grip, format!("worker handshake: {e}")))?;
    match read_msg_frame(&mut conv_sock) {
        Ok(Msg::ManifestReturn { version, .. }) if version == PROTOCOL_VERSION => {}
        Ok(Msg::ManifestReturn { version, .. }) => {
            return Err(fail(
                &grip,
                format!(
                    "worker process speaks peer-protocol version {version}; this host \
                     speaks {PROTOCOL_VERSION} (mixed qn binaries?)"
                ),
            ));
        }
        Ok(other) => {
            return Err(fail(
                &grip,
                format!("worker handshake: expected ManifestReturn, got {other:?}"),
            ));
        }
        Err(e) => return Err(fail(&grip, format!("worker handshake: {e}"))),
    }
    conv_sock.set_read_timeout(None).ok();

    let (inbox_tx, inbox_rx) = async_channel::unbounded::<WorkerMsg>();
    let (outbox_tx, outbox_rx) = async_channel::unbounded::<WorkerMsg>();
    let (done_tx, done_rx) = async_channel::bounded::<Result<WireData, String>>(1);
    let (control_tx, control_rx) = async_channel::unbounded::<ControlReq>();
    let (dispatch_tx, dispatch_rx) = async_channel::unbounded::<DispatchReq>();

    // Conversation pump: one conversation at a time (write the Call, read to its
    // terminal), so each reply pairs with the request it answers by shape alone.
    // Serves both request sources — control (ps) and hosted-object dispatch; a
    // closed lane becomes a pending future so the other keeps being served (the
    // registry pins `control_tx` open, so the pump lives with the worker).
    {
        let mut sock = conv_sock;
        std::thread::spawn(move || {
            enum ConvReq {
                Ctl(ControlReq),
                Dispatch(Box<DispatchReq>),
            }
            async fn recv_or_pending<T>(rx: &async_channel::Receiver<T>) -> T {
                match rx.recv().await {
                    Ok(v) => v,
                    Err(_) => std::future::pending().await,
                }
            }
            loop {
                let req = futures_lite::future::block_on(futures_lite::future::or(
                    async { ConvReq::Ctl(recv_or_pending(&control_rx).await) },
                    async { ConvReq::Dispatch(Box::new(recv_or_pending(&dispatch_rx).await)) },
                ));
                match req {
                    ConvReq::Ctl(req) => {
                        let op = match req.kind {
                            ControlKind::PsTree => OP_PS_TREE,
                        };
                        if write_msg_frame(&mut sock, &call_frame(op, None)).is_err() {
                            break;
                        }
                        match read_msg_frame(&mut sock) {
                            Ok(Msg::CallReturnData { value }) => {
                                let _ = req.reply.send_blocking(WorkerMsg::Data(value));
                            }
                            // An error terminal (or unexpected frame) drops the reply;
                            // the requester's bounded-staleness deadline reads
                            // 'unresponsive'.
                            Ok(_) => {}
                            Err(_) => break,
                        }
                    }
                    ConvReq::Dispatch(req) => {
                        // The encode seam refuses blocks for process backing, so a
                        // shipped-block sidecar can never reach the socket — but the
                        // wire must never silently drop one either.
                        if !req.blocks.is_empty() {
                            let _ = req.reply.send_blocking(Msg::CallReturnError {
                                message: "blocks cannot cross a process boundary".to_string(),
                                remote_stack: String::new(),
                            });
                            continue;
                        }
                        if write_msg_frame(&mut sock, &req.frame).is_err() {
                            break;
                        }
                        match read_msg_frame(&mut sock) {
                            // Any terminal answers the dispatch; a dropped reply
                            // lane (cancelled caller) is the sender's problem.
                            Ok(msg) => {
                                let _ = req.reply.send_blocking(msg);
                            }
                            Err(_) => break,
                        }
                    }
                }
            }
        });
    }
    // Mailbox writer: parent sends -> Call{op:"send"} frames.
    {
        let mut sock = mail_sock
            .try_clone()
            .map_err(|e| format!("socket clone: {e}"))?;
        std::thread::spawn(move || {
            while let Ok(msg) = inbox_rx.recv_blocking() {
                match msg {
                    WorkerMsg::Data(dv) => {
                        if write_msg_frame(&mut sock, &call_frame(OP_SEND, Some(dv))).is_err() {
                            break;
                        }
                    }
                    // Refused at the send seams; unreachable in practice.
                    WorkerMsg::Block(_) => {
                        eprintln!("qn: dropped a block on a process-worker lane");
                    }
                }
            }
            let _ = sock.shutdown(std::net::Shutdown::Write);
        });
    }
    // Mailbox reader: the child's sends, then the lane's terminal (= done).
    // EOF without a terminal means the child vanished; reap it either way.
    {
        let mut sock = mail_sock;
        let reader_grip = grip.clone();
        std::thread::spawn(move || {
            let mut done_sent = false;
            while let Ok(msg) = read_msg_frame(&mut sock) {
                match msg {
                    Msg::Call { op, data, .. } if op == OP_SEND => {
                        let dv = data.unwrap_or(WireData::Null);
                        let _ = outbox_tx.send_blocking(WorkerMsg::Data(dv));
                    }
                    Msg::CallReturnData { value } => {
                        let _ = done_tx.send_blocking(Ok(value));
                        done_sent = true;
                    }
                    Msg::CallReturnError { message, .. } => {
                        let _ = done_tx.send_blocking(Err(message));
                        done_sent = true;
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
            dispatch_tx,
        },
        pid,
        grip,
    ))
}

/// The CHILD entry (`qn worker-serve <sock> <unit> [<serviceClass>]`):
/// connect back TWICE (conversation socket first, then mailbox — the order
/// the parent accepts in), answer the manifest handshake, bridge the mailbox
/// to the lanes, run the standard worker body, and ship the done terminal.
pub fn worker_serve_main(sock_path: &str, unit: &str, service: Option<&str>) -> i32 {
    let connect = |what: &str| match UnixStream::connect(sock_path) {
        Ok(s) => Some(s),
        Err(e) => {
            eprintln!("qn worker-serve: cannot connect {what} socket at {sock_path}: {e}");
            None
        }
    };
    let Some(mut conv_sock) = connect("conversation") else {
        return 1;
    };
    let Some(mail_sock) = connect("mailbox") else {
        return 1;
    };

    // Answer the manifest handshake SYNCHRONOUSLY, before anything that can
    // fail (a missing unit, a compile error) exits the process — the parent's
    // spawn blocks on this reply, and a fast-failing body must still get its
    // done terminal read, which requires the spawn to succeed first. No
    // classes are provided yet (hosted objects are the next slice); the
    // version in the reply is what the PARENT enforces.
    match read_msg_frame(&mut conv_sock) {
        Ok(Msg::GetManifest { .. }) => {
            if let Err(e) = write_msg_frame(
                &mut conv_sock,
                &Msg::ManifestReturn {
                    classes: Vec::new(),
                    version: PROTOCOL_VERSION,
                },
            ) {
                eprintln!("qn worker-serve: handshake reply: {e}");
                return 1;
            }
        }
        other => {
            eprintln!("qn worker-serve: handshake: expected GetManifest, got {other:?}");
            return 1;
        }
    }

    let (inbox_tx, inbox_rx) = async_channel::unbounded::<WorkerMsg>();
    let (outbox_tx, outbox_rx) = async_channel::unbounded::<WorkerMsg>();
    let (control_tx, control_rx) = async_channel::unbounded::<ControlReq>();
    let (dispatch_tx, dispatch_rx) = async_channel::unbounded::<DispatchReq>();

    // Conversation thread: serve conversations one at a time. A `psTree` (empty
    // class) routes to the driver's control lane; anything else is a
    // hosted-object dispatch for the serve loop's dispatch lane.
    {
        let mut sock = conv_sock;
        let control_tx = control_tx.clone();
        std::thread::spawn(move || {
            while let Ok(frame) = read_msg_frame(&mut sock) {
                let Msg::Call {
                    ref op,
                    ref class_name,
                    ..
                } = frame
                else {
                    continue;
                };
                let terminal = if op == OP_PS_TREE && class_name.is_empty() {
                    // One conversation at a time: a fresh reply lane per request,
                    // the terminal written before the next Call is read.
                    let (reply_tx, reply_rx) = async_channel::bounded::<WorkerMsg>(1);
                    if control_tx
                        .send_blocking(ControlReq {
                            kind: ControlKind::PsTree,
                            reply: reply_tx,
                        })
                        .is_err()
                    {
                        return;
                    }
                    let Ok(WorkerMsg::Data(value)) = reply_rx.recv_blocking() else {
                        return;
                    };
                    Msg::CallReturnData { value }
                } else {
                    let (reply_tx, reply_rx) = async_channel::bounded::<Msg>(1);
                    if dispatch_tx
                        .send_blocking(DispatchReq {
                            frame,
                            blocks: Vec::new(),
                            reply: reply_tx,
                        })
                        .is_err()
                    {
                        // No serve loop (a plain worker, or one that already
                        // stopped): answer recoverably, stay in sync.
                        Msg::CallReturnError {
                            message: "this worker hosts no objects".to_string(),
                            remote_stack: String::new(),
                        }
                    } else {
                        match reply_rx.recv_blocking() {
                            Ok(msg) => msg,
                            Err(_) => Msg::CallReturnError {
                                message: "the hosted serve loop exited mid-call".to_string(),
                                remote_stack: String::new(),
                            },
                        }
                    }
                };
                if write_msg_frame(&mut sock, &terminal).is_err() {
                    return;
                }
            }
        });
    }
    // Mailbox reader: the parent's sends -> inbox.
    {
        let mut sock = match mail_sock.try_clone() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("qn worker-serve: socket clone: {e}");
                return 1;
            }
        };
        std::thread::spawn(move || {
            while let Ok(msg) = read_msg_frame(&mut sock) {
                if let Msg::Call { op, data, .. } = msg
                    && op == OP_SEND
                {
                    let dv = data.unwrap_or(WireData::Null);
                    let _ = inbox_tx.send_blocking(WorkerMsg::Data(dv));
                }
            }
            // Parent gone: closing inbox_tx (drop) ends Worker.receive with nil.
        });
    }
    // Mailbox writer — a funnel, because the done terminal (from this thread)
    // must follow every queued send; `None` is the close sentinel that lets it
    // flush before the process exits.
    let (to_mail_tx, to_mail_rx) = std::sync::mpsc::channel::<Option<Msg>>();
    let writer = {
        let mut sock = mail_sock;
        std::thread::spawn(move || {
            while let Ok(Some(msg)) = to_mail_rx.recv() {
                if write_msg_frame(&mut sock, &msg).is_err() {
                    break;
                }
            }
            let _ = sock.shutdown(std::net::Shutdown::Write);
        })
    };
    // fwd: worker sends -> Call{op:"send"} frames into the funnel.
    {
        let to_mail = to_mail_tx.clone();
        std::thread::spawn(move || {
            while let Ok(msg) = outbox_rx.recv_blocking() {
                if let WorkerMsg::Data(dv) = msg
                    && to_mail.send(Some(call_frame(OP_SEND, Some(dv)))).is_err()
                {
                    break;
                }
            }
        });
    }

    let link = WorkerLink {
        inbox_rx,
        outbox_tx,
        control_rx,
        dispatch_rx,
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
    // The mailbox lane's terminal: the whole run was one implicit conversation.
    let done = match &out {
        Ok(v) => Msg::CallReturnData { value: v.clone() },
        Err(m) => Msg::CallReturnError {
            message: m.clone(),
            remote_stack: String::new(),
        },
    };
    let _ = to_mail_tx.send(Some(done));
    let _ = to_mail_tx.send(None); // close sentinel (see writer)
    drop(to_mail_tx);
    let _ = writer.join();
    // Exit now: the reader/conversation threads are parked on sockets the
    // PARENT still holds open; returning would leave this process lingering.
    i32::from(out.is_err())
}
