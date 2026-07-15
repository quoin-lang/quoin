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
    // no pump, no bytes (ACTOR_OBJECTS.md §1). The channel-relay lanes (§6)
    // likewise carry owned frames.
    let (dispatch_tx, dispatch_rx) = async_channel::unbounded();
    let (chan_to_worker_tx, chan_to_worker_rx) = async_channel::unbounded();
    let (chan_to_parent_tx, chan_to_parent_rx) = async_channel::unbounded();
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
                    chan_tx: chan_to_parent_tx,
                    chan_rx: chan_to_worker_rx,
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
        chan_tx: chan_to_worker_tx,
        chan_rx: chan_to_parent_rx,
    }
}

/// Synthesize the hosting lines appended to a hosted unit's source
/// (src/runtime/worker.rs): `Worker.hostRoot:` instantiates the class, roots
/// it in the worker's hosted-object table, and reports ready; then a gather
/// of `Worker.hostServeLane` thunks serves peer-protocol `Call` dispatches —
/// one fiber per lane, all consuming the shared dispatch channel — until the
/// reserved stop op (one per lane) or the lane closing ends them
/// (ACTOR_OBJECTS.md §5.1).
fn service_loop_qn(class_name: &str, lanes: u32) -> String {
    let thunks = "{ Worker.hostServeLane } ".repeat(lanes.max(1) as usize);
    format!("\nWorker.hostRoot:'{class_name}';\nAsync.gather:#( {thunks});\n")
}

/// Spawn a SERVICE worker: the unit at `path` (which defines `class_name`)
/// plus the generic serve loop, compiled as one program. `lanes` fibers serve
/// concurrently (the parent's claim machinery bounds in-flight conversations
/// to the same number).
pub fn spawn_worker_service(path: String, class_name: String, lanes: u32) -> WorkerChannels {
    spawn_worker_with(move |link| run_worker_service(&path, &class_name, lanes, link))
}

/// Spawn a hosted-BLOCK worker (`Worker.host:'unit.qn' with:{ … }` /
/// `Worker.with:{ … }`): boot, load the unit if any, then run the shipped
/// block via `Worker.hostBlockRoot` and host the object it answers.
pub fn spawn_worker_hosted_block(
    path: Option<String>,
    pb: PortableBlock,
    lanes: u32,
) -> WorkerChannels {
    spawn_worker_with(move |link| run_worker_hosted_block(path.as_deref(), pb, lanes, link))
}

fn run_worker_hosted_block(
    path: Option<&str>,
    pb: PortableBlock,
    lanes: u32,
    link: WorkerLink,
) -> Result<WireData, String> {
    let unit_source = match path {
        Some(p) => {
            std::fs::read_to_string(PathBuf::from(p)).map_err(|e| format!("host unit {p}: {e}"))?
        }
        None => String::new(),
    };
    let thunks = "{ Worker.hostServeLane } ".repeat(lanes.max(1) as usize);
    let source = format!(
        "{unit_source}
Worker.hostBlockRoot;
Async.gather:#( {thunks});
"
    );
    run_worker_source_with(path.unwrap_or("{host block}"), &source, link, move |vm| {
        vm.pending_host_block = Some(pb);
    })
}

/// Parse + compile a shipped block literal's source in THIS process and hand
/// back its template. The text is exactly the `{ … }` literal; compiled as a
/// one-expression program it yields a wrapper whose bytecode pushes the block
/// constant — that inner template is the block. The capture names pre-declare
/// as locals (the strict undefined-name check would otherwise refuse the
/// block's free reads — parent-side they were real bindings, and the rebuilt
/// env provides them at run time); globals resolve at run time against the
/// child's own unit. The checker's diagnostics are deliberately not reported
/// here (the parent's compile of the same text already showed them).
fn compile_block_source(
    source: &str,
    filename: &str,
    capture_names: HashSet<String>,
) -> Result<Arc<StaticBlock>, String> {
    let ast = try_parse_quoin_string_named(source, filename)
        .map_err(|e| format!("shipped block: parse error: {e}"))?;
    let NodeValue::Program(program) = &ast.value else {
        return Err("shipped block: root AST is not a program".to_string());
    };
    let wrapper = Compiler::new_with_locals(capture_names)
        .with_template_ids()
        .compile_program(program)
        .map_err(|e| format!("shipped block: compile error: {e}"))?;
    for inst in wrapper.bytecode.iter() {
        if let Instruction::Push(Constant::Block(inner)) = inst {
            return Ok(inner.clone());
        }
    }
    Err("shipped block: the compiled source contains no block literal".to_string())
}

/// Rebuild a [`PortableBlock`] from its wire form (`portable_block_to_wire`):
/// re-compile each level's source, intern the capture names (symbols are
/// process-global), and re-parse the global names for the rebuild-time
/// verification `rebuild_env` performs.
fn portable_block_from_wire(w: &WireData) -> Result<PortableBlock, String> {
    let WireData::Map(fields) = w else {
        return Err("shipped block: malformed payload (not a map)".to_string());
    };
    let get = |k: &str| fields.iter().find(|(n, _)| n == k).map(|(_, v)| v);
    let Some(WireData::Str(source)) = get("source") else {
        return Err("shipped block: payload has no source text".to_string());
    };
    let filename = match get("filename") {
        Some(WireData::Str(f)) if !f.is_empty() => f.as_str(),
        _ => "{shipped block}",
    };
    let mut captures = Vec::new();
    if let Some(WireData::List(items)) = get("captures") {
        for item in items {
            let WireData::Map(kv) = item else {
                return Err("shipped block: malformed capture entry".to_string());
            };
            let field = |k: &str| kv.iter().find(|(n, _)| n == k).map(|(_, v)| v);
            let Some(WireData::Str(name)) = field("name") else {
                return Err("shipped block: capture entry has no name".to_string());
            };
            let cap = match (field("data"), field("block")) {
                (Some(d), _) => PortableCapture::Data(d.clone()),
                (None, Some(b)) => PortableCapture::Block(Box::new(
                    portable_block_from_wire(b).map_err(|e| format!("capture '{name}': {e}"))?,
                )),
                (None, None) => {
                    return Err(format!("shipped block: capture '{name}' has no payload"));
                }
            };
            captures.push((Symbol::intern(name), cap));
        }
    }
    let globals = match get("globals") {
        Some(WireData::List(names)) => names
            .iter()
            .map(|n| match n {
                WireData::Str(s) => Ok(NamespacedName::parse(s)),
                _ => Err("shipped block: malformed global name".to_string()),
            })
            .collect::<Result<Vec<_>, _>>()?,
        _ => Vec::new(),
    };
    let capture_names: HashSet<String> = captures
        .iter()
        .map(|(sym, _)| sym.as_str().to_string())
        .collect();
    let template = compile_block_source(source, filename, capture_names)?;
    Ok(PortableBlock {
        template,
        captures,
        globals,
    })
}

pub(crate) fn run_worker_service(
    path: &str,
    class_name: &str,
    lanes: u32,
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
    let source = format!("{unit_source}\n{}", service_loop_qn(class_name, lanes));
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
    run_worker_source_with(path, source, link, |_| {})
}

/// As `run_worker_source`, with a post-boot hook run on the fresh VM before
/// the program starts (the hosted-block spawn stashes its shipped block here).
fn run_worker_source_with(
    path: &str,
    source: &str,
    link: WorkerLink,
    prepare: impl FnOnce(&mut VmState<'_>),
) -> Result<WireData, String> {
    run_worker_source_impl(path, source, link, prepare, false)
}

/// As [`run_worker_source_with`], but the program's VALUE (stack top at
/// completion) is the result — the parameterized-job bootstrap needs the
/// block's answer for `join`, where unit workers deliberately answer nil.
fn run_worker_source_valued(
    path: &str,
    source: &str,
    link: WorkerLink,
    prepare: impl FnOnce(&mut VmState<'_>),
) -> Result<WireData, String> {
    run_worker_source_impl(path, source, link, prepare, true)
}

fn run_worker_source_impl(
    path: &str,
    source: &str,
    link: WorkerLink,
    prepare: impl FnOnce(&mut VmState<'_>),
    result_from_stack: bool,
) -> Result<WireData, String> {
    let ast = try_parse_quoin_string_named(source, path)
        .map_err(|e| format!("worker unit {path}: parse error: {e}"))?;
    let NodeValue::Program(program_node) = &ast.value else {
        return Err(format!("worker unit {path}: root AST is not a program"));
    };

    let mut arena = boot_worker_arena(link)?;
    {
        let mut prepare = Some(prepare);
        arena.mutate_root(|_mc, vm| {
            if let Some(p) = prepare.take() {
                p(vm);
            }
        });
    }
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
    if !result_from_stack {
        return Ok(WireData::Null);
    }
    arena.mutate_root(|_mc, vm| {
        let v = vm.stack.last().copied().unwrap_or(Value::Nil);
        value_to_wire(v, None)
            .map_err(|e| format!("the worker block's result is not portable data: {e}"))
    })
}

/// Boot a fresh worker VM: arena + native builtins + the full qnlib prelude
/// (the exact `qn <file>` recipe), with the parent link injected. Shared by
/// the unit and portable-block worker bodies.
fn boot_worker_arena(link: WorkerLink) -> Result<ReplArena, String> {
    let mut arena: ReplArena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
        let mut vm = VmState::new(mc, VmOptions::default());
        register_builtins(mc, &mut vm);
        // Register the parent link's channel-relay lanes (§6) before the link
        // moves into place: channels crossing plain lanes or dispatches relay
        // through this entry.
        let idx = crate::runtime::channel_relay::register_chan_link(
            &mut vm,
            link.chan_tx.clone(),
            link.chan_rx.clone(),
        );
        vm.parent_chan_link = Some(idx);
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
    // A parameterized job can't start directly — its arguments arrive as the
    // first N mailbox messages, which only VM code can receive. `Worker.jobRoot`
    // is that bootstrap: receive, rebuild, invoke; its value is the program's.
    if !job.template.param_syms.is_empty() {
        return run_worker_source_valued("{job}", "Worker.jobRoot\n", link, move |vm| {
            vm.pending_host_block = Some(job);
        });
    }
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

use quoin_ext_proto::{Msg, PROTOCOL_VERSION, ReplyMeta};

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

/// As `write_msg_frame`, stamping `ReplyMeta.handler_micros` on the frame —
/// used for the frame that closes a conversation (its outer `CallReturn*`
/// terminal), so boundary profiling gets the worker's servicing time across
/// the process boundary (§7's out-of-band pattern; a no-op for frame kinds
/// that carry no meta).
fn write_msg_frame_meta(
    sock: &mut UnixStream,
    msg: &Msg,
    handler_micros: u64,
) -> std::io::Result<()> {
    let meta = ReplyMeta { handler_micros };
    let bytes = quoin_ext_proto::encode_with_meta(msg, Some(&meta));
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

/// As `read_msg_frame`, surfacing the frame's `ReplyMeta` (zeroed when the
/// frame kind carries none, or the peer didn't stamp it).
fn read_msg_frame_meta(sock: &mut UnixStream) -> std::io::Result<(Msg, ReplyMeta)> {
    let mut len = [0u8; 4];
    sock.read_exact(&mut len)?;
    let mut buf = vec![0u8; u32::from_le_bytes(len) as usize];
    sock.read_exact(&mut buf)?;
    quoin_ext_proto::decode_frame_with_meta(&buf)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// One parent-side conversation pump (one per lane socket): serves control
/// (ps) and hosted-object dispatch requests off the shared queues, one LIFO
/// conversation at a time — write the Call, relay to its terminal, so each
/// reply pairs with the request it answers by shape alone. A closed queue
/// becomes a pending future so the other keeps being served (the registry
/// pins `control_tx` open, so the pumps live with the worker).
fn parent_conv_pump(
    mut sock: UnixStream,
    control_rx: async_channel::Receiver<ControlReq>,
    dispatch_rx: async_channel::Receiver<DispatchReq>,
) {
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
                // The encode seam routes blocks to the handle path for
                // process backing, so a shipped-block sidecar can never
                // reach the socket — but the wire must never silently
                // drop one either.
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
                // Relay the conversation to its outer terminal (strict
                // LIFO alternation): worker→parent frames forward up the
                // reply lane; between them a parent→worker frame (host-op
                // reply or nested call) comes down `hostops` and goes to
                // the socket. `depth` counts open conversation levels — a
                // `Call` in either direction opens one, a `CallReturn*`
                // closes one; the worker frame that closes level 0 ends
                // the relay, and its `ReplyMeta` carries the child's
                // handler time (§7). A closed `hostops` lane means the
                // caller abandoned the conversation (cancellation): pending
                // worker requests are answered with an error so the child
                // unwinds, and the relay still runs to the terminal so
                // the socket stays in sync for the next dispatch.
                let mut depth: u32 = 1;
                let mut broken = false;
                loop {
                    let (up, meta) = match read_msg_frame_meta(&mut sock) {
                        Ok(f) => f,
                        Err(_) => {
                            broken = true;
                            break;
                        }
                    };
                    if matches!(up, Msg::Call { .. }) {
                        depth += 1;
                    } else {
                        depth = depth.saturating_sub(1);
                    }
                    // Stamp the handler time BEFORE forwarding the terminal:
                    // the caller wakes on the forward and reads the stamp.
                    if depth == 0 {
                        req.handler_micros
                            .store(meta.handler_micros, Ordering::Relaxed);
                    }
                    // A dropped reply lane (cancelled caller) must not
                    // stop the relay — the wire has to reach its terminal.
                    let _ = req.reply.send_blocking(up);
                    if depth == 0 {
                        break;
                    }
                    let down = match req.hostops.recv_blocking() {
                        Ok(f) => f,
                        Err(_) => Msg::CallReturnError {
                            message: "the caller abandoned the conversation".to_string(),
                            remote_stack: String::new(),
                        },
                    };
                    if matches!(down, Msg::Call { .. }) {
                        depth += 1;
                    } else {
                        depth = depth.saturating_sub(1);
                    }
                    if write_msg_frame(&mut sock, &down).is_err() {
                        broken = true;
                        break;
                    }
                }
                if broken {
                    break;
                }
            }
        }
    }
}

/// Spawn a PROCESS-backed worker running `unit` (with `service` naming a
/// hosted class for the WorkerService form, and `lanes` conversation sockets
/// — §5.1: lanes, never frame multiplexing; each socket speaks the protocol
/// unchanged, one LIFO conversation at a time). Returns the standard channel
/// ends plus the child's pid; the pump threads own the sockets.
pub fn spawn_worker_process(
    unit: Option<String>,
    body: ProcessBody,
    lanes: u32,
) -> Result<(WorkerChannels, u32, ChildGrip), String> {
    let lanes = lanes.max(1);
    let sock_path = format!(
        "/tmp/quoin-worker-{}-{}.sock",
        std::process::id(),
        SPAWNED.fetch_add(1, Ordering::Relaxed)
    );
    let _ = std::fs::remove_file(&sock_path);
    let listener = UnixListener::bind(&sock_path).map_err(|e| format!("worker socket: {e}"))?;

    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let mut cmd = std::process::Command::new(exe);
    // `@none` = unit-less (a hosted block booting bare qnlib); `@block` in the
    // service slot = "a hosted-block payload follows the version gate". Both
    // sentinels are unspellable as class names or sane unit paths.
    cmd.arg("worker-serve")
        .arg(&sock_path)
        .arg(unit.as_deref().unwrap_or("@none"));
    match &body {
        ProcessBody::Plain => {}
        ProcessBody::Class(class) => {
            cmd.arg(class);
            cmd.arg(lanes.to_string());
        }
        ProcessBody::Block(_) => {
            cmd.arg("@block");
            cmd.arg(lanes.to_string());
        }
        ProcessBody::Job(_) => {
            cmd.arg("@job");
            cmd.arg(lanes.to_string());
        }
    }
    let child = cmd
        .spawn()
        .map_err(|e| format!("spawn worker process: {e}"))?;
    let pid = child.id();
    let grip: ChildGrip = std::sync::Arc::new(std::sync::Mutex::new(Some(child)));

    // Accept with a bounded wait: poll the listener in nonblocking mode so a
    // child that dies pre-connect becomes an error, not a hang. The child
    // connects `lanes + 1` times, in a fixed order: every conversation
    // socket, then the mailbox.
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
    let mut conv_socks = Vec::with_capacity(lanes as usize);
    for _ in 0..lanes {
        conv_socks.push(accept_one(&listener)?);
    }
    let mail_sock = accept_one(&listener)?;
    let chan_sock = accept_one(&listener)?;
    let _ = std::fs::remove_file(&sock_path);

    // The manifest handshake — the version gate (mirrors the extension spawn:
    // this side enforces; a mismatched child is killed, never misdecoded).
    // Bounded, so a wedged child is an error rather than a hang. One gate per
    // worker: it runs on the FIRST conversation socket only (all lanes are
    // the same binary).
    let fail = |grip: &ChildGrip, msg: String| -> String {
        if let Some(c) = grip.lock().expect("child grip").as_mut() {
            let _ = c.kill();
        }
        msg
    };
    let conv_sock = &mut conv_socks[0];
    conv_sock
        .set_read_timeout(Some(std::time::Duration::from_secs(10)))
        .ok();
    write_msg_frame(
        conv_sock,
        &Msg::GetManifest {
            version: PROTOCOL_VERSION,
        },
    )
    .map_err(|e| fail(&grip, format!("worker handshake: {e}")))?;
    match read_msg_frame(conv_sock) {
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
    // Hosted-block spawn: the payload rides the same gated exchange — one
    // `Call{op:"hostBlock"}` carrying source + captures, acknowledged with a
    // `CallReturn` terminal before any serving begins (still under the
    // handshake read timeout, so a wedged child is an error, not a hang).
    if let ProcessBody::Block(payload) | ProcessBody::Job(payload) = &body {
        write_msg_frame(
            conv_sock,
            &Msg::Call {
                op: "hostBlock".to_string(),
                arg: String::new(),
                handles: Vec::new(),
                resources: Vec::new(),
                releases: Vec::new(),
                arrays: Vec::new(),
                data: Some(payload.clone()),
                class_name: String::new(),
                recv: 0,
                method_args: Vec::new(),
            },
        )
        .map_err(|e| fail(&grip, format!("worker host-block payload: {e}")))?;
        match read_msg_frame(conv_sock) {
            Ok(Msg::CallReturn { .. }) => {}
            Ok(other) => {
                return Err(fail(
                    &grip,
                    format!("worker host-block payload: expected CallReturn, got {other:?}"),
                ));
            }
            Err(e) => return Err(fail(&grip, format!("worker host-block payload: {e}"))),
        }
    }
    conv_sock.set_read_timeout(None).ok();

    let (inbox_tx, inbox_rx) = async_channel::unbounded::<WorkerMsg>();
    let (outbox_tx, outbox_rx) = async_channel::unbounded::<WorkerMsg>();
    let (done_tx, done_rx) = async_channel::bounded::<Result<WireData, String>>(1);
    let (control_tx, control_rx) = async_channel::unbounded::<ControlReq>();
    let (dispatch_tx, dispatch_rx) = async_channel::unbounded::<DispatchReq>();
    // Channel-relay lanes (§6): dumb frame pumps over their own socket — the
    // relay protocol is event-shaped (correlation ids), so unlike the
    // conversation pumps there is no state to track, just bytes both ways.
    let (chan_to_worker_tx, chan_out_rx) = async_channel::unbounded::<ChanFrame>();
    let (chan_in_tx, chan_to_parent_rx) = async_channel::unbounded::<ChanFrame>();
    {
        let mut wsock = chan_sock
            .try_clone()
            .map_err(|e| format!("chan socket clone: {e}"))?;
        std::thread::spawn(move || {
            while let Ok(f) = chan_out_rx.recv_blocking() {
                if write_msg_frame(&mut wsock, &chan_frame_to_msg(f)).is_err() {
                    break;
                }
            }
            let _ = wsock.shutdown(std::net::Shutdown::Write);
        });
    }
    {
        let mut rsock = chan_sock;
        std::thread::spawn(move || {
            while let Ok(msg) = read_msg_frame(&mut rsock) {
                if let Some(f) = msg_to_chan_frame(msg)
                    && chan_in_tx.send_blocking(f).is_err()
                {
                    break;
                }
            }
        });
    }

    // Conversation pumps: one per lane socket, all consuming the SHARED
    // control/dispatch queues (MPMC — whichever pump is free takes the next
    // request; lanes are fungible tokens, exactly as thread backing's serve
    // fibers). Each pump runs one LIFO conversation at a time on its socket.
    for sock in conv_socks {
        let control_rx = control_rx.clone();
        let dispatch_rx = dispatch_rx.clone();
        std::thread::spawn(move || parent_conv_pump(sock, control_rx, dispatch_rx));
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
                    // Spawn-time `args:` blocks; the seam pre-validated the
                    // encode (source text present), so a failure here is a bug.
                    WorkerMsg::Block(pb) => match portable_block_to_wire(&pb) {
                        Ok(dv) => {
                            if write_msg_frame(&mut sock, &call_frame(OP_SEND_BLOCK, Some(dv)))
                                .is_err()
                            {
                                break;
                            }
                        }
                        Err(e) => eprintln!("qn: dropped an unencodable block: {e}"),
                    },
                    // A shipped channel endpoint: the id rides the mailbox as
                    // its own op; the relay traffic rides the chan socket.
                    WorkerMsg::Channel(chan) => {
                        let mut f = call_frame(OP_SEND_CHAN, None);
                        if let Msg::Call { recv, .. } = &mut f {
                            *recv = chan;
                        }
                        if write_msg_frame(&mut sock, &f).is_err() {
                            break;
                        }
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
                    Msg::Call { op, recv, .. } if op == OP_SEND_CHAN => {
                        let _ = outbox_tx.send_blocking(WorkerMsg::Channel(recv));
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
            chan_tx: chan_to_worker_tx,
            chan_rx: chan_to_parent_rx,
        },
        pid,
        grip,
    ))
}

/// One child-side conversation loop (one per lane socket): serve
/// conversations one at a time. A `psTree` (empty class) routes to the
/// driver's control lane; anything else is a hosted-object dispatch for the
/// serve loop's shared dispatch lane.
fn child_conv_loop(
    mut sock: UnixStream,
    control_tx: async_channel::Sender<ControlReq>,
    dispatch_tx: async_channel::Sender<DispatchReq>,
) {
    while let Ok(frame) = read_msg_frame(&mut sock) {
        let Msg::Call {
            ref op,
            ref class_name,
            ..
        } = frame
        else {
            continue;
        };
        if op == OP_PS_TREE && class_name.is_empty() {
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
            if write_msg_frame(&mut sock, &Msg::CallReturnData { value }).is_err() {
                return;
            }
            continue;
        }
        let (reply_tx, reply_rx) = async_channel::unbounded::<Msg>();
        let (hostop_tx, hostop_rx) = async_channel::unbounded::<Msg>();
        let handler = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
        if dispatch_tx
            .send_blocking(DispatchReq {
                frame,
                blocks: Vec::new(),
                reply: reply_tx,
                hostops: hostop_rx,
                handler_micros: handler.clone(),
            })
            .is_err()
        {
            // No serve loop (a plain worker, or one that already
            // stopped): answer recoverably, stay in sync.
            let refuse = Msg::CallReturnError {
                message: "this worker hosts no objects".to_string(),
                remote_stack: String::new(),
            };
            if write_msg_frame(&mut sock, &refuse).is_err() {
                return;
            }
            continue;
        }
        // Relay this conversation (mirror of the parent pump): serve-
        // fiber frames — host-ops, then the terminal — go up the
        // socket; parent frames (host-op replies, nested calls) come
        // back down to the fiber. Call opens a level, CallReturn*
        // closes one; the fiber frame that closes level 0 ends the
        // relay and carries the serve fiber's handler time as
        // `ReplyMeta` (§7). A dead serve loop mid-conversation closes
        // every open level with an error so the parent stays in sync.
        let mut depth: u32 = 1;
        while depth > 0 {
            let up = match reply_rx.recv_blocking() {
                Ok(f) => f,
                Err(_) => {
                    let e = Msg::CallReturnError {
                        message: "the hosted serve loop exited mid-call".to_string(),
                        remote_stack: String::new(),
                    };
                    while depth > 0 {
                        if write_msg_frame(&mut sock, &e).is_err() {
                            return;
                        }
                        depth -= 1;
                    }
                    break;
                }
            };
            if matches!(up, Msg::Call { .. }) {
                depth += 1;
            } else {
                depth = depth.saturating_sub(1);
            }
            if depth == 0 {
                if write_msg_frame_meta(
                    &mut sock,
                    &up,
                    handler.load(std::sync::atomic::Ordering::Relaxed),
                )
                .is_err()
                {
                    return;
                }
                break;
            }
            if write_msg_frame(&mut sock, &up).is_err() {
                return;
            }
            let down = match read_msg_frame(&mut sock) {
                Ok(f) => f,
                Err(_) => return,
            };
            if matches!(down, Msg::Call { .. }) {
                depth += 1;
            } else {
                depth = depth.saturating_sub(1);
            }
            if hostop_tx.send_blocking(down).is_err() {
                // The serve fiber vanished with parent frames owed;
                // the next reply_rx recv reports it and closes the
                // open levels.
                continue;
            }
        }
    }
}

/// The CHILD entry (`qn worker-serve <sock> <unit> [<serviceClass> [<lanes>]]`):
/// connect back `lanes + 1` times (every conversation socket first, then the
/// mailbox — the order the parent accepts in), answer the manifest handshake
/// on the first conversation socket, bridge the mailbox to the lanes, run the
/// standard worker body, and ship the done terminal.
pub fn worker_serve_main(sock_path: &str, unit: &str, service: Option<&str>, lanes: u32) -> i32 {
    let lanes = lanes.max(1);
    let connect = |what: &str| match UnixStream::connect(sock_path) {
        Ok(s) => Some(s),
        Err(e) => {
            eprintln!("qn worker-serve: cannot connect {what} socket at {sock_path}: {e}");
            None
        }
    };
    let mut conv_socks = Vec::with_capacity(lanes as usize);
    for _ in 0..lanes {
        let Some(s) = connect("conversation") else {
            return 1;
        };
        conv_socks.push(s);
    }
    let Some(mail_sock) = connect("mailbox") else {
        return 1;
    };
    let Some(chan_sock) = connect("channel relay") else {
        return 1;
    };

    // Answer the manifest handshake SYNCHRONOUSLY, before anything that can
    // fail (a missing unit, a compile error) exits the process — the parent's
    // spawn blocks on this reply, and a fast-failing body must still get its
    // done terminal read, which requires the spawn to succeed first. One gate
    // per worker, on the first conversation socket. No classes are provided
    // yet (hosted manifests are a later slice); the version in the reply is
    // what the PARENT enforces.
    let conv_sock = &mut conv_socks[0];
    match read_msg_frame(conv_sock) {
        Ok(Msg::GetManifest { .. }) => {
            if let Err(e) = write_msg_frame(
                conv_sock,
                &Msg::ManifestReturn {
                    classes: Vec::new(),
                    version: PROTOCOL_VERSION,
                    // Workers don't self-declare lanes: the parent decides the count at
                    // spawn and opens that many conversation sockets. 0 = no declaration.
                    lanes: 0,
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
    // Hosted-block mode (`@block`): the payload frame is part of the gated
    // exchange, read synchronously before the conversation loops spawn. Only
    // receipt is acknowledged here — parse/compile errors surface later
    // through the done lane, exactly like any unit error.
    let mut host_block_payload: Option<WireData> = None;
    if service == Some("@block") || service == Some("@job") {
        match read_msg_frame(conv_sock) {
            Ok(Msg::Call { op, data, .. }) if op == "hostBlock" => {
                let Some(payload) = data else {
                    eprintln!("qn worker-serve: host-block payload carried no data");
                    return 1;
                };
                host_block_payload = Some(payload);
                if let Err(e) = write_msg_frame(
                    conv_sock,
                    &Msg::CallReturn {
                        result: String::new(),
                    },
                ) {
                    eprintln!("qn worker-serve: host-block ack: {e}");
                    return 1;
                }
            }
            other => {
                eprintln!("qn worker-serve: expected the host-block payload, got {other:?}");
                return 1;
            }
        }
    }

    let (inbox_tx, inbox_rx) = async_channel::unbounded::<WorkerMsg>();
    let (outbox_tx, outbox_rx) = async_channel::unbounded::<WorkerMsg>();
    let (control_tx, control_rx) = async_channel::unbounded::<ControlReq>();
    let (dispatch_tx, dispatch_rx) = async_channel::unbounded::<DispatchReq>();

    // One conversation thread per lane socket, feeding the SHARED dispatch
    // lane the serve fibers consume (the thread-backing shape, over sockets).
    for sock in conv_socks {
        let control_tx = control_tx.clone();
        let dispatch_tx = dispatch_tx.clone();
        std::thread::spawn(move || child_conv_loop(sock, control_tx, dispatch_tx));
    }
    // The conv threads now hold the only senders that matter: drop the
    // originals so PARENT DEATH unwinds this process. When the parent dies,
    // every conversation socket EOFs, the conv threads exit, their sender
    // clones drop, the dispatch/control queues close, and the serve fibers'
    // `DispatchRecv` unparks — the body ends and the child exits instead of
    // lingering as an orphan (which also pins the parent's inherited stdio
    // pipes open, wedging anything reading them).
    drop(control_tx);
    drop(dispatch_tx);
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
                if let Msg::Call { op, data, recv, .. } = msg {
                    if op == OP_SEND {
                        let dv = data.unwrap_or(WireData::Null);
                        let _ = inbox_tx.send_blocking(WorkerMsg::Data(dv));
                    } else if op == OP_SEND_CHAN {
                        let _ = inbox_tx.send_blocking(WorkerMsg::Channel(recv));
                    } else if op == OP_SEND_BLOCK {
                        match data.as_ref().map(portable_block_from_wire) {
                            Some(Ok(pb)) => {
                                let _ = inbox_tx.send_blocking(WorkerMsg::Block(pb));
                            }
                            other => eprintln!(
                                "qn worker-serve: dropped a malformed shipped block: {other:?}"
                            ),
                        }
                    }
                }
            }
            // Mailbox EOF means the PARENT PROCESS IS GONE, nothing milder: the
            // parent's registry pins a mailbox sender for its whole lifetime, so
            // a dropped handle never closes the write half — only parent death
            // (or `terminate`, where we are being killed anyway) lands here. A
            // dead parent cannot consume the done terminal or anything else, and
            // a lingering orphan pins the parent's inherited stdio pipes open,
            // wedging whatever reads them — so exit NOW, deterministically. (A
            // frame decode error also lands here; a desynced mailbox is equally
            // unrecoverable.)
            std::process::exit(0);
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
                let frame = match msg {
                    WorkerMsg::Data(dv) => call_frame(OP_SEND, Some(dv)),
                    WorkerMsg::Channel(chan) => {
                        let mut f = call_frame(OP_SEND_CHAN, None);
                        if let Msg::Call { recv, .. } = &mut f {
                            *recv = chan;
                        }
                        f
                    }
                    // Refused at the send seams; unreachable in practice.
                    WorkerMsg::Block(_) => {
                        eprintln!("qn: dropped a block on a process-worker lane");
                        continue;
                    }
                };
                if to_mail.send(Some(frame)).is_err() {
                    break;
                }
            }
        });
    }

    // Channel-relay pumps, the parent side's mirror.
    let (chan_tx, chan_out_rx) = async_channel::unbounded::<ChanFrame>();
    let (chan_in_tx, chan_rx) = async_channel::unbounded::<ChanFrame>();
    {
        let mut wsock = match chan_sock.try_clone() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("qn worker-serve: chan socket clone: {e}");
                return 1;
            }
        };
        std::thread::spawn(move || {
            while let Ok(f) = chan_out_rx.recv_blocking() {
                if write_msg_frame(&mut wsock, &chan_frame_to_msg(f)).is_err() {
                    break;
                }
            }
            let _ = wsock.shutdown(std::net::Shutdown::Write);
        });
    }
    {
        let mut rsock = chan_sock;
        std::thread::spawn(move || {
            while let Ok(msg) = read_msg_frame(&mut rsock) {
                if let Some(f) = msg_to_chan_frame(msg)
                    && chan_in_tx.send_blocking(f).is_err()
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
        chan_tx,
        chan_rx,
        process: true,
    };
    let unit_opt = (unit != "@none").then_some(unit);
    let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        match (host_block_payload, service) {
            (Some(payload), Some("@job")) => {
                portable_block_from_wire(&payload).and_then(|pb| run_worker_block(pb, link))
            }
            (Some(payload), _) => portable_block_from_wire(&payload)
                .and_then(|pb| run_worker_hosted_block(unit_opt, pb, lanes, link)),
            (None, Some(class)) => run_worker_service(unit, class, lanes, link),
            (None, None) => run_worker_unit(unit, link),
        }
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
