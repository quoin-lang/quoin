//! The `Worker` class — C2 v1 isolates (docs/internal/CONCURRENCY_ARCH.md §5).
//!
//! Parent side: `Worker.spawn:'unit.qn'` boots a fresh VM on its own OS
//! thread and returns a handle; `handle.send:` deep-copies a data value in,
//! `handle.receive` parks until the worker sends one back, `handle.join`
//! parks until the unit finishes (raising the worker's error, catchable).
//! Worker side (class-side, inside the spawned unit): `Worker.receive` /
//! `Worker.send:` are the mirror lanes and `Worker.worker?` says which side
//! you're on.
//!
//! Receives and joins park through `await_io` like any IO wait — a worker
//! handle IS a parked task, so `Async.gather:`/`timeout:do:`/cancellation
//! compose over worker waits unchanged (the §10 L2 property).
//!
//! What crosses: the extension wire's data taxonomy via the same walkers
//! (numbers, strings, booleans, nil, Bytes, lists, maps, big numerics).
//! Blocks, symbols, instances, and resources refuse with the wire's errors.
//! A receive on an exited worker's drained outbox answers nil.

use std::any::Any;

use gc_arena::Collect;
use gc_arena::collect::Trace;
use quoin_ext_proto::DataValue as WireData;

use crate::error::QuoinError;
use crate::io_backend::{IoRequest, IoResult};
use crate::runtime::extension::{value_to_wire, wire_to_value};
use crate::value::ObjectPayload;
use crate::value::{AnyCollect, NativeClassBuilder, Value};
use crate::worker::{
    PortableBlock, WorkerMsg, note_message, rebuild_portable_value, snapshot_block, spawn_worker,
    spawn_worker_block,
};

/// Parent-side handle state: the three channel ends. Plain `Send` data —
/// nothing here touches the arena.
#[derive(Debug)]
pub struct NativeWorkerHandle {
    backing: &'static str,
    /// Registry slot, for `label:` restamps (`VM.ps` rows are the audience).
    reg_idx: usize,
    /// Process backing only: the grip `terminate` kills through.
    grip: Option<crate::worker::ChildGrip>,
    inbox_tx: async_channel::Sender<WorkerMsg>,
    outbox_rx: async_channel::Receiver<WorkerMsg>,
    done_rx: async_channel::Receiver<Result<WireData, crate::worker::WorkerExit>>,
    /// This peer's index in `vm.io.lives` (SUPERVISION.md slice 1) — the
    /// lifecycle sink `events` and `terminate` reach.
    life_idx: usize,
    /// This link's index in `vm.io.chan_links` (§6 channel relay).
    chan_link: usize,
    /// `join` consumes the done lane (its channel holds exactly one value);
    /// a second join is a clear error rather than a confusing hang.
    joined: std::cell::Cell<bool>,
}

impl AnyCollect for NativeWorkerHandle {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn trace_gc<'gc>(&self, _cc: &mut dyn Trace<'gc>) {}
}

unsafe impl<'gc> Collect<'gc> for NativeWorkerHandle {
    const NEEDS_TRACE: bool = false;
}

/// What `snapshot_block` needs from a block value: its template and capture env.
pub(crate) type BlockParts<'gc> = (
    std::sync::Arc<crate::instruction::StaticBlock>,
    Option<gc_arena::Gc<'gc, gc_arena::lock::RefLock<crate::value::EnvFrame<'gc>>>>,
);

/// The template + capture env of a block value, `None` for anything else —
/// the "is this a block, and what ships" probe shared by the lane send seam
/// and the hosted-dispatch argument encoder.
pub(crate) fn block_parts<'gc>(v: Value<'gc>) -> Option<BlockParts<'gc>> {
    if let Value::Object(obj) = v {
        let borrowed = obj.borrow();
        if let ObjectPayload::Block(b) = &borrowed.payload {
            return Some((b.template.clone(), b.parent_env));
        }
    }
    None
}

/// Copy a guest value into a cross-worker message. A BLOCK value ships as
/// a portable block (template + capture snapshot — same rules as
/// `Worker.start:`); everything else takes the wire walkers, whose
/// taxonomy still refuses symbols/instances/resources — and blocks nested
/// INSIDE data structures.
fn to_message<'gc>(v: Value<'gc>, allow_blocks: bool) -> Result<WorkerMsg, QuoinError> {
    if let Some((template, parent_env)) = block_parts(v) {
        if !allow_blocks {
            return Err(QuoinError::Other(
                "blocks cannot cross a process boundary (templates are \
                 in-process references) — send data, or use thread backing"
                    .into(),
            ));
        }
        let pb = snapshot_block(template, parent_env, 0)?;
        note_message();
        return Ok(WorkerMsg::Block(pb));
    }
    let dv = value_to_wire(v, None)?;
    note_message();
    Ok(WorkerMsg::Data(dv))
}

/// Decode a received cross-worker message into a live value: data through
/// the wire walkers, blocks rebuilt over their capture snapshots, shipped
/// channels wrapped as relay endpoints on `link` (§6).
fn from_message<'gc>(
    vm: &mut crate::vm::VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    msg: &WorkerMsg,
    link: usize,
) -> Result<Value<'gc>, QuoinError> {
    match msg {
        WorkerMsg::Data(dv) => wire_to_value(vm, mc, dv, None),
        WorkerMsg::Block(pb) => rebuild_portable_value(vm, mc, pb).map_err(QuoinError::Other),
        WorkerMsg::Channel(chan) => {
            crate::runtime::channel_relay::relay_endpoint(vm, mc, link, *chan)
        }
    }
}

/// Receive a spawned block's N spawn-time `args:` — the first N mailbox
/// messages — as PLAIN data. Every park happens in here, and any relay agent
/// is ensured before returning, so [`materialize_spawn_args`] can then build
/// GC values with no yield in sight (the Arg::Chan yield-safety shape: this
/// pair must stay split — the receive parks, and values must not exist on a
/// frame that parks).
fn receive_spawn_arg_msgs<'gc>(
    vm: &mut crate::vm::VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    n_args: usize,
) -> Result<Vec<WorkerMsg>, QuoinError> {
    let mut arg_msgs: Vec<WorkerMsg> = Vec::with_capacity(n_args);
    if n_args > 0 {
        let rx = match &vm.worker_link {
            Some(link) => link.inbox_rx.clone(),
            None => {
                return Err(QuoinError::Other(
                    "spawn args: only runs inside a worker".into(),
                ));
            }
        };
        for i in 0..n_args {
            match vm.await_io(IoRequest::WorkerRecv(rx.clone()))? {
                IoResult::WorkerMsg(Some(msg)) => arg_msgs.push(msg),
                IoResult::WorkerMsg(None) => {
                    return Err(QuoinError::Other(format!(
                        "spawn args: the parent closed before sending argument {} of {n_args}",
                        i + 1
                    )));
                }
                other => {
                    return Err(QuoinError::Other(format!(
                        "spawn args: unexpected result {other:?}"
                    )));
                }
            }
        }
    }
    if arg_msgs.iter().any(|m| matches!(m, WorkerMsg::Channel(_))) {
        let chan_link = vm.parent_chan_link.unwrap_or(0);
        crate::runtime::channel_relay::ensure_relay_agent(vm, mc, chan_link)?;
    }
    Ok(arg_msgs)
}

/// The non-yielding half of the spawn-args pair: materialize received
/// messages as live values (data via the wire walkers, blocks rebuilt,
/// channels wrapped raw — the agent is already ensured).
fn materialize_spawn_args<'gc>(
    vm: &mut crate::vm::VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    arg_msgs: &[WorkerMsg],
) -> Result<Vec<Value<'gc>>, QuoinError> {
    let chan_link = vm.parent_chan_link.unwrap_or(0);
    let mut arg_vals: Vec<Value<'_>> = Vec::with_capacity(arg_msgs.len());
    for msg in arg_msgs {
        let v = match msg {
            WorkerMsg::Data(dv) => wire_to_value(vm, mc, dv, None)?,
            WorkerMsg::Block(apb) => {
                rebuild_portable_value(vm, mc, apb).map_err(QuoinError::Other)?
            }
            WorkerMsg::Channel(chan) => {
                crate::runtime::channel_relay::relay_endpoint_raw(vm, mc, chan_link, *chan)?
            }
        };
        arg_vals.push(v);
    }
    Ok(arg_vals)
}

/// The shared body of the `start:` family: snapshot the block, spawn its
/// worker on the chosen backing (a process job crosses as source +
/// captures), ship the spawn-time `args:` by the [`spawn_arg`] rules against
/// a pre-registered chan link, and wrap the handle. Arity is checked here,
/// before anything ships.
fn start_block_worker<'gc>(
    vm: &mut crate::vm::VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    receiver: Value<'gc>,
    block: Value<'gc>,
    args_list: Option<Value<'gc>>,
    backing: &'static str,
) -> Result<Value<'gc>, QuoinError> {
    let Some((template, parent_env)) = block_parts(block) else {
        return Err(QuoinError::Other("Worker.start: expects a Block".into()));
    };
    let arg_values = match args_list {
        Some(list) => crate::runtime::extension::extract_args(list)?,
        None => Vec::new(),
    };
    if template.param_syms.len() != arg_values.len() {
        return Err(QuoinError::Other(if args_list.is_none() {
            format!(
                "Worker.start: the block takes {} parameter(s) — pass them with \
                 Worker.start:args: (or send data through the lanes)",
                template.param_syms.len()
            )
        } else {
            format!(
                "Worker.start:args: the block takes {} parameter(s) but args: has {}",
                template.param_syms.len(),
                arg_values.len()
            )
        }));
    }
    let pb = snapshot_block(template, parent_env, 0)
        .map_err(|e| QuoinError::Other(format!("Worker.start: {e}")))?;
    let process = backing == "process";
    let (ch, pid, grip) = if process {
        let payload = crate::worker::portable_block_to_wire(&pb)
            .map_err(|e| QuoinError::Other(format!("Worker.start: {e}")))?;
        let (ch, pid, grip) =
            crate::worker::spawn_worker_process(None, crate::worker::ProcessBody::Job(payload), 1)
                .map_err(QuoinError::Other)?;
        (ch, Some(pid), Some(grip))
    } else {
        (spawn_worker_block(pb), None, None)
    };
    let chan_link = crate::runtime::channel_relay::register_chan_link(
        vm,
        ch.chan_tx.clone(),
        ch.chan_rx.clone(),
    );
    for (i, v) in arg_values.into_iter().enumerate() {
        let msg = crate::runtime::worker_service::spawn_arg(vm, mc, v, chan_link, process)
            .map_err(|e| QuoinError::Other(format!("Worker.start:args: element {}: {e}", i + 1)))?;
        let _ = ch.inbox_tx.try_send(msg);
    }
    wrap_handle(
        vm,
        mc,
        receiver,
        "<block>",
        backing,
        pid,
        grip,
        ch,
        Some(chan_link),
    )
}

/// Answer the peer's lifecycle events Channel (SUPERVISION.md slice 1),
/// creating it — and its pump task — on the first ask via the qnlib
/// `LifecycleBoot` helper (native code mints tasks and channels only through
/// a Quoin block). Later asks answer the SAME channel: one consumer stream
/// per peer; `vm.life_channels` is both the cache and the GC root.
pub(crate) fn life_events_channel<'gc>(
    vm: &mut crate::vm::VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    life_idx: usize,
) -> Result<Value<'gc>, QuoinError> {
    if let Some(Some(ch)) = vm.life_channels.get(life_idx) {
        return Ok(*ch);
    }
    let boot = crate::runtime::extension::resolve_global(vm, "LifecycleBoot").ok_or_else(|| {
        QuoinError::Other("lifecycle events: LifecycleBoot is not installed (qnlib)".into())
    })?;
    let idx_val = vm.new_int(mc, life_idx as i64);
    let ch = vm.call_method(mc, boot, "start:", vec![idx_val])?;
    if vm.life_channels.len() <= life_idx {
        vm.life_channels.resize(life_idx + 1, None);
    }
    vm.life_channels[life_idx] = Some(ch);
    Ok(ch)
}

/// Wrap freshly spawned lanes in a Worker-class handle instance.
#[allow(clippy::too_many_arguments)] // handle-wrapping helper threads the worker/channel context
fn wrap_handle<'gc>(
    vm: &mut crate::vm::VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    receiver: Value<'gc>,
    unit: &str,
    backing: &'static str,
    pid: Option<u32>,
    grip: Option<crate::worker::ChildGrip>,
    ch: crate::worker::WorkerChannels,
    chan_link: Option<usize>,
) -> Result<Value<'gc>, QuoinError> {
    let Value::Class(class) = receiver else {
        return Err(QuoinError::Other("Worker: bad receiver".into()));
    };
    let reg_idx = vm.worker_registry.len();
    let life_idx = {
        let lives = vm.io.lives.clone();
        let mut lives = lives.borrow_mut();
        lives.push(ch.life.clone());
        lives.len() - 1
    };
    vm.worker_registry.push(crate::worker::WorkerReg {
        unit: unit.to_string(),
        label: unit.to_string(),
        backing,
        pid,
        inbox_tx: ch.inbox_tx.clone(),
        outbox_rx: ch.outbox_rx.clone(),
        control_tx: ch.control_tx.clone(),
    });
    // Pre-registered when spawn-time args had to ship against the link first.
    let chan_link = chan_link.unwrap_or_else(|| {
        crate::runtime::channel_relay::register_chan_link(
            vm,
            ch.chan_tx.clone(),
            ch.chan_rx.clone(),
        )
    });
    Ok(vm.new_native_state(
        mc,
        class,
        NativeWorkerHandle {
            backing,
            reg_idx,
            grip,
            inbox_tx: ch.inbox_tx,
            outbox_rx: ch.outbox_rx,
            done_rx: ch.done_rx,
            life_idx,
            chan_link,
            joined: std::cell::Cell::new(false),
        },
    ))
}

/// One hosted-object dispatch (`Worker.hostServe:`'s per-request body,
/// docs/internal/ACTOR_OBJECTS.md §2): decode the `Call`'s arguments, perform the
/// send, and classify the result into its terminal — portable data COPIES
/// (`CallReturnData`); a non-portable object is HOSTED (table insert →
/// `CallReturnResource`, the parent wraps it as a sub-proxy) — objects are
/// addressed, values are copied; a raise becomes `CallReturnError` carrying the
/// worker's rendered stack segment (`ex.remoteStack` at the call site).
/// `blocks` is the request's shipped-block sidecar (§3a): each pair rebuilds
/// as a live closure in THIS arena and takes its argument position, so the
/// hosted method calls it locally — one boundary crossing however many times
/// it runs.
fn dispatch_hosted<'gc>(
    vm: &mut crate::vm::VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    op: &str,
    class_name: &str,
    recv: u64,
    method_args: &[quoin_ext_proto::Arg],
    blocks: &[(usize, PortableBlock)],
) -> quoin_ext_proto::Msg {
    use quoin_ext_proto::{Arg, Msg};
    let err = |message: String| Msg::CallReturnError {
        message,
        remote_stack: String::new(),
    };
    // The relay-agent boot runs Quoin (a task spawn) and can YIELD — do it
    // before any unrooted GC value lives on this frame (`target`, `argv`);
    // the in-loop endpoint construction below is the non-yielding half.
    if method_args.iter().any(|a| matches!(a, Arg::Chan(_))) {
        let link = vm.parent_chan_link.unwrap_or(0);
        if let Err(e) = crate::runtime::channel_relay::ensure_relay_agent(vm, mc, link) {
            return err(format!("hosted call '{op}': {e}"));
        }
    }
    // recv 0 is the reserved class-side id: the send targets the hosted
    // CLASS itself (`Pool.classMethod` through the installed class).
    let target = if recv == 0 {
        match crate::runtime::extension::resolve_global(vm, class_name) {
            Some(v) => v,
            None => {
                return err(format!(
                    "hosted call '{op}': no class named '{class_name}' in the worker"
                ));
            }
        }
    } else {
        match vm.hosted_get(recv) {
            Some(v) => v,
            None => return err(format!("hosted call '{op}': no live hosted object {recv}")),
        }
    };
    let mut argv = Vec::with_capacity(method_args.len());
    for (i, a) in method_args.iter().enumerate() {
        if let Some((_, pb)) = blocks.iter().find(|(pos, _)| *pos == i) {
            match rebuild_portable_value(vm, mc, pb) {
                Ok(v) => argv.push(v),
                Err(e) => return err(format!("hosted call '{op}': argument {}: {e}", i + 1)),
            }
            continue;
        }
        let v = match a {
            Arg::Data(dv) => match wire_to_value(vm, mc, dv, None) {
                Ok(v) => v,
                Err(e) => return err(format!("hosted call '{op}': argument {}: {e}", i + 1)),
            },
            // A proxy of THIS worker passed back in: resolve to the live object.
            Arg::Resource(id) => match vm.hosted_get(*id) {
                Some(v) => v,
                None => {
                    return err(format!(
                        "hosted call '{op}': argument {} references no live hosted object {id}",
                        i + 1
                    ));
                }
            },
            // A shipped channel (§6): wrap the owner's id as a live relay
            // endpoint on this worker's parent link (agent already ensured
            // above — this half cannot yield).
            Arg::Chan(chan) => {
                let link = vm.parent_chan_link.unwrap_or(0);
                match crate::runtime::channel_relay::relay_endpoint_raw(vm, mc, link, *chan) {
                    Ok(v) => v,
                    Err(e) => {
                        return err(format!("hosted call '{op}': argument {}: {e}", i + 1));
                    }
                }
            }
            // A parent-held block (the §3a handle fallback): wrap it so
            // invocations round-trip on the conversation.
            Arg::Handle(h) => match host_block_value(vm, mc, *h) {
                Ok(v) => v,
                Err(e) => return err(format!("hosted call '{op}': argument {}: {e}", i + 1)),
            },
            Arg::Array(_) => {
                return err(format!(
                    "hosted call '{op}': argument {} has an unsupported kind for a hosted call",
                    i + 1
                ));
            }
        };
        argv.push(v);
    }
    match vm.call_method_mnu(mc, target, op, argv) {
        Ok(v) => match value_to_wire(v, None) {
            Ok(dv) => Msg::CallReturnData { value: dv },
            Err(wire_err) => {
                // A CHANNEL return ships as a live endpoint (§6) — checked
                // before the generic hosting path would wrap it as an inert
                // sub-proxy.
                if crate::runtime::channel_relay::is_channel_value(v) {
                    let link = vm.parent_chan_link.unwrap_or(0);
                    return match crate::runtime::channel_relay::ship_for_crossing(vm, mc, v, link) {
                        Ok(chan) => Msg::CallReturnChannel { chan },
                        Err(e) => err(format!("hosted call '{op}': {e}")),
                    };
                }
                if let Value::Object(obj) = v {
                    let class_name = obj.borrow().class_name().to_string();
                    let class_val = Value::Class(obj.borrow().class);
                    let id = vm.hosted_insert(v);
                    // First sighting of this class crossing the boundary: the
                    // terminal carries its manifest so the parent can install
                    // a real class (§2); later returns carry only the name.
                    if vm.hosted_announced.insert(class_name.clone()) {
                        let (instance_selectors, class_selectors) = manifest_selectors(class_val);
                        Msg::CallReturnResourceDecl {
                            resource: id,
                            class_name,
                            instance_selectors,
                            class_selectors,
                        }
                    } else {
                        Msg::CallReturnResource {
                            resource: id,
                            class_name,
                        }
                    }
                } else {
                    err(format!(
                        "hosted call '{op}': the return value is neither portable data nor \
                         a hostable object: {wire_err}"
                    ))
                }
            }
        },
        Err(e) => error_terminal(vm, &e, "worker"),
    }
}

/// A hosted class's selector manifest (ACTOR_OBJECTS.md §2): instance and
/// class-side selectors from the class, its mixins, and its ancestors —
/// stopping at `Object`, whose protocol (`s`, `pp`, `==`…) stays LOCAL on the
/// proxy, exactly as the MNU-era hook behaved. Sorted: manifests are wire
/// bytes, and wire bytes never come from hash iteration.
pub(crate) fn manifest_selectors<'gc>(class_val: Value<'gc>) -> (Vec<String>, Vec<String>) {
    use std::collections::HashSet;
    let mut instance: HashSet<String> = HashSet::new();
    let mut class_side: HashSet<String> = HashSet::new();
    let mut queue: Vec<gc_arena::Gc<'gc, gc_arena::lock::RefLock<crate::value::Class<'gc>>>> =
        Vec::new();
    if let Value::Class(c) = class_val {
        queue.push(c);
    }
    let mut walked = 0usize;
    while let Some(c) = queue.pop() {
        walked += 1;
        if walked > 256 {
            break; // defensive: a hierarchy cycle would be a VM bug
        }
        let b = c.borrow();
        if b.name.to_string() == "Object" {
            continue;
        }
        for sym in b.instance_methods.keys() {
            instance.insert(sym.as_str().to_string());
        }
        for sym in b.class_methods.keys() {
            class_side.insert(sym.as_str().to_string());
        }
        for m in &b.mixin_classes {
            queue.push(*m);
        }
        if let Some(p) = b.parent {
            queue.push(p);
        }
    }
    let mut instance: Vec<String> = instance.into_iter().collect();
    let mut class_side: Vec<String> = class_side.into_iter().collect();
    instance.sort();
    class_side.sort();
    (instance, class_side)
}

/// Render an error as a `CallReturnError` terminal: a Quoin-level throw parks
/// its value in `exceptions.active` (the `Thrown` marker itself just says
/// "thrown exception"), so render THAT as the message; the stack segment is
/// labeled with the side it unwound on ("worker" / "parent").
pub(crate) fn error_terminal<'gc>(
    vm: &crate::vm::VmState<'gc>,
    e: &QuoinError,
    side: &str,
) -> quoin_ext_proto::Msg {
    let message = if matches!(e.innermost(), QuoinError::Thrown) {
        match vm.exceptions.active {
            Some(v) => format!("{v}"),
            None => e.to_string(),
        }
    } else {
        e.to_string()
    };
    quoin_ext_proto::Msg::CallReturnError {
        message,
        remote_stack: crate::runtime::extension::quoin_stack_segment_labeled(e, side),
    }
}

/// Worker-side wrapper for a parent-held block that crossed as `Arg::Handle`
/// (the §3a handle fallback): invocations round-trip to the parent as host-op
/// `Call`s on the block's handle, riding the current conversation.
#[derive(Debug)]
pub struct NativeHostBlock {
    handle: u64,
}

impl AnyCollect for NativeHostBlock {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn trace_gc<'gc>(&self, _cc: &mut dyn Trace<'gc>) {}
}

unsafe impl<'gc> Collect<'gc> for NativeHostBlock {
    const NEEDS_TRACE: bool = false;
}

/// Wrap a received block handle as a live `HostBlock` instance.
fn host_block_value<'gc>(
    vm: &mut crate::vm::VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    handle: u64,
) -> Result<Value<'gc>, String> {
    let class = crate::runtime::extension::resolve_global(vm, "HostBlock")
        .ok_or_else(|| "the HostBlock class is not installed".to_string())?;
    let Value::Class(class) = class else {
        return Err("HostBlock is not a class".to_string());
    };
    Ok(vm.new_native_state(mc, class, NativeHostBlock { handle }))
}

/// The most deeply worker code may nest host-op conversations back into the
/// parent (a parent block calling a hosted method that invokes a parent block,
/// recursively). Mirrors the extension cap: each level is a live call frame on
/// both sides.
const MAX_CONV_DEPTH: u32 = 16;

/// Invoke a parent-held block from worker code: send a host-op `Call` up the
/// current conversation and pump frames until its `CallReturn*` — servicing
/// any NESTED parent→worker call that arrives in between (the LIFO
/// conversation shape, §5.1 rule 3).
fn invoke_parent_block<'gc>(
    vm: &mut crate::vm::VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    receiver: Value<'gc>,
    op: &str,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, QuoinError> {
    let handle = receiver
        .with_native_state::<NativeHostBlock, _, _>(|s| s.handle)
        .map_err(QuoinError::Other)?;
    let task = vm.sched.current_task.0;
    let conv = match vm.worker_convs.get_mut(&task) {
        Some(c) => {
            if c.depth >= MAX_CONV_DEPTH {
                return Err(QuoinError::Other(format!(
                    "host block '{op}': conversations nested {MAX_CONV_DEPTH} levels deep — \
                     mutual parent<->worker recursion? (each level is a live call frame on \
                     both sides)"
                )));
            }
            c.depth += 1;
            c.clone()
        }
        None => {
            return Err(QuoinError::Other(format!(
                "host block '{op}': a parent block can only be invoked while serving a \
                 hosted call (there is no open conversation to the parent)"
            )));
        }
    };
    let result = invoke_parent_block_inner(vm, mc, &conv, handle, op, args);
    if let Some(c) = vm.worker_convs.get_mut(&task) {
        c.depth = c.depth.saturating_sub(1);
    }
    result
}

fn invoke_parent_block_inner<'gc>(
    vm: &mut crate::vm::VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    conv: &crate::worker::ConvHandles,
    handle: u64,
    op: &str,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, QuoinError> {
    use quoin_ext_proto::{Arg, Msg};
    let mut method_args = Vec::with_capacity(args.len());
    for (i, a) in args.iter().enumerate() {
        method_args.push(Arg::Data(value_to_wire(*a, None).map_err(|e| {
            QuoinError::Other(format!(
                "host block '{op}': argument {} cannot cross back to the parent: {e}",
                i + 1
            ))
        })?));
    }
    let frame = Msg::Call {
        op: op.to_string(),
        arg: String::new(),
        handles: Vec::new(),
        resources: Vec::new(),
        releases: Vec::new(),
        arrays: Vec::new(),
        data: None,
        class_name: String::new(),
        recv: handle,
        method_args,
    };
    if conv.reply_tx.try_send(frame).is_err() {
        return Err(QuoinError::Other(format!(
            "host block '{op}': the caller abandoned the conversation"
        )));
    }
    loop {
        let msg = match vm.await_io(IoRequest::FrameRecv(conv.hostops_rx.clone()))? {
            IoResult::FrameMsg(Some(msg)) => *msg,
            IoResult::FrameMsg(None) => {
                return Err(QuoinError::Other(format!(
                    "host block '{op}': the caller abandoned the conversation"
                )));
            }
            other => {
                return Err(QuoinError::Other(format!(
                    "host block '{op}': unexpected result {other:?}"
                )));
            }
        };
        match msg {
            Msg::CallReturnData { value } => return wire_to_value(vm, mc, &value, None),
            // A parent block answered with a PARENT-owned channel: wrap it.
            Msg::CallReturnChannel { chan } => {
                let link = vm.parent_chan_link.unwrap_or(0);
                return crate::runtime::channel_relay::relay_endpoint(vm, mc, link, chan);
            }
            Msg::CallReturnError {
                message,
                remote_stack,
            } => {
                return Err(QuoinError::ExtensionError {
                    message,
                    remote_stack: crate::runtime::extension::truncate_blob(remote_stack),
                });
            }
            // A nested parent→worker call riding the bound conversation while
            // the parent block runs: serve it and keep waiting (LIFO).
            Msg::Call {
                op: nested_op,
                class_name: nested_class,
                recv,
                method_args,
                releases,
                ..
            } => {
                for rid in &releases {
                    vm.hosted_release(*rid);
                }
                let reply =
                    dispatch_hosted(vm, mc, &nested_op, &nested_class, recv, &method_args, &[]);
                if conv.reply_tx.try_send(reply).is_err() {
                    return Err(QuoinError::Other(format!(
                        "host block '{op}': the caller abandoned the conversation"
                    )));
                }
            }
            other => {
                return Err(QuoinError::Other(format!(
                    "host block '{op}': unexpected frame {other:?} in the conversation"
                )));
            }
        }
    }
}

/// The worker-side class for parent-held blocks (§3a handle fallback). Not
/// user-constructible; instances arrive as block arguments to hosted methods.
pub fn build_host_block_class() -> NativeClassBuilder {
    NativeClassBuilder::new("HostBlock", Some("Object"))
        .construct_with("passed as a block argument to a hosted method")
        .class_doc(
            "A block that lives in the PARENT VM, received by a hosted method whose \
             caller passed a block that could not ship (it captures live state, or the \
             service is process-backed). Invoking it round-trips to the parent -- the \
             block runs THERE, seeing its captures live -- one boundary crossing per \
             invocation. A shipped (portable) block runs worker-side on a capture \
             snapshot instead; see the WorkerService docs.",
        )
        .instance_method("value", |vm, mc, receiver, _args| {
            invoke_parent_block(vm, mc, receiver, "value", &[])
        })
        .doc("Invoke the parent block with no arguments (one round trip).")
        .instance_method("value:", |vm, mc, receiver, args| {
            invoke_parent_block(vm, mc, receiver, "value:", &args)
        })
        .doc("Invoke the parent block with one argument (one round trip).")
        .instance_method("valueWithArgs:", |vm, mc, receiver, args| {
            invoke_parent_block(vm, mc, receiver, "valueWithArgs:", &args)
        })
        .doc("Invoke the parent block with a List of arguments (one round trip).")
}

/// Root a freshly created host object (id 1) and send the ready message
/// carrying its class's MANIFEST (§2): the parent installs a real class from
/// it, so proxy dispatch is ordinary method lookup, no VM hook.
fn announce_root<'gc>(
    vm: &mut crate::vm::VmState<'gc>,
    root: Value<'gc>,
    class_val: Value<'gc>,
    class_name: &str,
    outbox_tx: &async_channel::Sender<WorkerMsg>,
) {
    vm.hosted_insert(root);
    let (instance, class_side) = manifest_selectors(class_val);
    vm.hosted_announced.insert(class_name.to_string());
    let _ = outbox_tx.try_send(WorkerMsg::Data(WireData::Map(vec![
        ("ready".to_string(), WireData::Bool(true)),
        (
            "className".to_string(),
            WireData::Str(class_name.to_string()),
        ),
        (
            "instance".to_string(),
            WireData::List(instance.into_iter().map(WireData::Str).collect()),
        ),
        (
            "classSide".to_string(),
            WireData::List(class_side.into_iter().map(WireData::Str).collect()),
        ),
    ])));
}

pub fn build_worker_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Worker", Some("Object"))
        .construct_with("use Worker.spawn: / Worker.start:")
        // ---- worker side: the hosted-object serve loop (ACTOR_OBJECTS.md §2/§5.1).
        // Invoked by the synthesized lines a hosting spawn appends to the unit
        // (`Worker.hostBlockRoot` then a gather of `Worker.hostServeLane`
        // thunks, one fiber per lane); not really a user-facing surface.
        .class_method("hostBlockRoot", |vm, mc, _receiver, _args| {
            let outbox_tx = match &vm.worker_link {
                Some(link) => link.outbox_tx.clone(),
                None => {
                    return Err(QuoinError::Other(
                        "Worker.hostBlockRoot: only runs inside a worker".into(),
                    ));
                }
            };
            let Some(pb) = vm.pending_host_block.take() else {
                return Err(QuoinError::Other(
                    "Worker.hostBlockRoot: no host block was shipped".into(),
                ));
            };
            // The block's parameters arrive as the first N mailbox messages
            // (spawn-time `args:` — the parent sends them before waiting for
            // ready). Receive them as plain data FIRST — every park happens
            // in there — then materialize with no yield in sight.
            let arg_msgs = receive_spawn_arg_msgs(vm, mc, pb.template.param_syms.len())?;
            // Rebuild AFTER the unit (if any) loaded, so the block's global
            // references resolve against it; run it; host what it answers.
            let block_val = rebuild_portable_value(vm, mc, &pb).map_err(QuoinError::Other)?;
            let root = if arg_msgs.is_empty() {
                vm.call_method_mnu(mc, block_val, "value", Vec::new())?
            } else {
                let arg_vals = materialize_spawn_args(vm, mc, &arg_msgs)?;
                let list = vm.new_list(mc, arg_vals);
                vm.call_method_mnu(mc, block_val, "valueWithArgs:", vec![list])?
            };
            let Value::Object(obj) = root else {
                return Err(QuoinError::Other(format!(
                    "Worker.host:with: the block must answer an object to host, got {root}"
                )));
            };
            let class_val = Value::Class(obj.borrow().class);
            let class_name = obj.borrow().class_name().to_string();
            announce_root(vm, root, class_val, &class_name, &outbox_tx);
            Ok(Value::Nil)
        })
        .doc(
            "Worker side of a block-form host (used by the synthesized unit): run the \
             shipped block, root the object it answers, and report ready with its \
             class's manifest. Not meant to be called directly.",
        )
        .class_method("jobRoot", |vm, mc, _receiver, _args| {
            // The parameterized-job bootstrap (`Worker.start:args:`): receive
            // the spawn args, rebuild the shipped block, run it — its value is
            // the program's value, which `join` carries home.
            let Some(pb) = vm.pending_host_block.take() else {
                return Err(QuoinError::Other(
                    "Worker.jobRoot: no job block was shipped".into(),
                ));
            };
            let arg_msgs = receive_spawn_arg_msgs(vm, mc, pb.template.param_syms.len())?;
            let block_val = rebuild_portable_value(vm, mc, &pb).map_err(QuoinError::Other)?;
            if arg_msgs.is_empty() {
                vm.call_method_mnu(mc, block_val, "value", Vec::new())
            } else {
                let arg_vals = materialize_spawn_args(vm, mc, &arg_msgs)?;
                let list = vm.new_list(mc, arg_vals);
                vm.call_method_mnu(mc, block_val, "valueWithArgs:", vec![list])
            }
        })
        .doc(
            "Worker side of a parameterized job (used by the synthesized job unit): \
             receive the spawn args, run the shipped block, answer its value. Not \
             meant to be called directly.",
        )
        .class_method("hostServeLane", |vm, mc, _receiver, _args| {
            let dispatch_rx = match &vm.worker_link {
                Some(link) => link.dispatch_rx.clone(),
                None => {
                    return Err(QuoinError::Other(
                        "Worker.hostServeLane: only runs inside a worker".into(),
                    ));
                }
            };
            loop {
                let req = match vm.await_io(IoRequest::DispatchRecv(dispatch_rx.clone()))? {
                    IoResult::DispatchMsg(Some(req)) => req,
                    // Lane closed: the parent side is gone; end the serve loop.
                    IoResult::DispatchMsg(None) => break,
                    other => {
                        return Err(QuoinError::Other(format!(
                            "Worker.hostServeLane: unexpected result {other:?}"
                        )));
                    }
                };
                let blocks = req.blocks;
                let quoin_ext_proto::Msg::Call {
                    op,
                    class_name,
                    recv,
                    method_args,
                    releases,
                    ..
                } = req.frame
                else {
                    let _ = req.reply.try_send(quoin_ext_proto::Msg::CallReturnError {
                        message: "Worker.hostServeLane: expected a Call frame".to_string(),
                        remote_stack: String::new(),
                    });
                    continue;
                };
                // Dropped-proxy releases ride every call, batched (the reap pattern).
                for rid in &releases {
                    vm.hosted_release(*rid);
                }
                if op == crate::worker::OP_STOP {
                    // One stop per lane: ack and end THIS fiber; the parent
                    // sends as many stops as there are lanes.
                    let _ = req.reply.try_send(quoin_ext_proto::Msg::CallReturnData {
                        value: WireData::Null,
                    });
                    break;
                }
                note_message();
                // Open the conversation for this dispatch: hosted code that
                // invokes a `HostBlock` finds its way back to the parent here.
                // Keyed by task — each lane fiber has its own conversation.
                let task = vm.sched.current_task.0;
                vm.worker_convs.insert(
                    task,
                    crate::worker::ConvHandles {
                        reply_tx: req.reply.clone(),
                        hostops_rx: req.hostops.clone(),
                        depth: 0,
                    },
                );
                let started = std::time::Instant::now();
                let reply = dispatch_hosted(vm, mc, &op, &class_name, recv, &method_args, &blocks);
                req.handler_micros.store(
                    started.elapsed().as_micros() as u64,
                    std::sync::atomic::Ordering::Relaxed,
                );
                vm.worker_convs.remove(&task);
                let _ = req.reply.try_send(reply);
            }
            // No `hosted.clear()` here: lane fibers share the table, and the
            // worker exits (dropping the arena) right after the last lane.
            Ok(Value::Nil)
        })
        .doc(
            "Worker side of a hosted service (used by the synthesized service unit): \
             serve peer-protocol dispatches from the shared lane, one at a time, until \
             the stop op or the lane closing ends this fiber. The service spawns one \
             per lane. Not meant to be called directly.",
        )
        .class_doc(
            "An isolate: a fresh VM on its own OS thread (or child process) with message \
             lanes to its parent. Parent side: `Worker.spawn:'unit.qn'` answers a handle -- \
             `send:` / `receive` exchange values, `join` parks until the unit finishes. \
             Worker side, inside the spawned unit: class-side `Worker.receive` / \
             `Worker.send:` are the mirror lanes, and `Worker.worker?` says which side you \
             are on. Messages deep-copy plain data (numbers, strings, booleans, nil, Bytes, \
             Lists, Maps); symbols, instances, and resources refuse -- and blocks cross \
             only as a whole thread-backed message. See docs/internal/CONCURRENCY_ARCH.md.\n\n\
             ```\n\
             var w = Worker.spawn:'jobs/indexer.qn';\n\
             w.send:#( 'index' 'docs/' );\n\
             var answer = w.receive;\n\
             w.join\n\
             ```",
        )
        // ---- parent side (class-side spawn, instance-side lanes) ----
        .class_method("spawn:", |vm, mc, receiver, args| {
            let path = args[0]
                .as_string()
                .ok_or_else(|| QuoinError::Other("Worker.spawn: expects a String path".into()))?;
            let reg = path.clone();
            wrap_handle(
                vm,
                mc,
                receiver,
                &reg,
                "thread",
                None,
                None,
                spawn_worker(path),
                None,
            )
        })
        .doc(
            "Boot a fresh VM running the unit at the String path on its own OS thread and \
             answer its handle immediately. The unit runs to completion; `join` observes \
             it. `Worker.spawn:(VM.unit)` runs another copy of the current program.",
        )
        // §13.2: backing is a spawn-time choice — thread (default) or a
        // child qn process bridged by the pump.
        .class_method("spawn:backing:", |vm, mc, receiver, args| {
            let path = args[0]
                .as_string()
                .ok_or_else(|| QuoinError::Other("Worker.spawn: expects a String path".into()))?;
            let backing = args[1]
                .as_string()
                .ok_or_else(|| QuoinError::Other("backing: expects a String".into()))?;
            match backing.as_str() {
                "thread" => {
                    let reg = path.clone();
                    wrap_handle(
                        vm,
                        mc,
                        receiver,
                        &reg,
                        "thread",
                        None,
                        None,
                        spawn_worker(path),
                        None,
                    )
                }
                "process" => {
                    let reg = path.clone();
                    let (ch, pid, grip) = crate::worker::spawn_worker_process(
                        Some(path),
                        crate::worker::ProcessBody::Plain,
                        1,
                    )
                    .map_err(QuoinError::Other)?;
                    wrap_handle(
                        vm,
                        mc,
                        receiver,
                        &reg,
                        "process",
                        Some(pid),
                        Some(grip),
                        ch,
                        None,
                    )
                }
                other => Err(QuoinError::Other(format!(
                    "Worker.spawn: unknown backing '{other}' (thread|process)"
                ))),
            }
        })
        .doc(
            "As `spawn:`, choosing the backing at spawn time: 'thread' (the default) or \
             'process' -- a child qn process bridged over the extension wire, whose \
             messages carry data only (blocks cannot cross a process boundary).",
        )
        // Portable blocks are in-process by nature; the explicit form
        // documents WHY rather than silently doing the wrong thing.
        .class_method("start:backing:", |vm, mc, receiver, args| {
            let backing = crate::runtime::worker_service::backing_arg(args[1])?;
            start_block_worker(vm, mc, receiver, args[0], None, backing)
        })
        .doc(
            "As `start:`, choosing the backing: 'thread' (the default) or 'process' -- \
             a child qn process; the block crosses as its SOURCE TEXT plus a snapshot \
             of its captures, so it must come from source (not assembled at runtime).",
        )
        .class_method("start:args:", |vm, mc, receiver, args| {
            start_block_worker(vm, mc, receiver, args[0], Some(args[1]), "thread")
        })
        .doc(
            "As `start:`, passing ARGUMENTS to the block's parameters: portable values \
             snapshot, a Channel becomes a live endpoint in the worker, a portable \
             block crosses as a callable; anything else refuses loudly. Arity is \
             checked before anything ships.\n\n\
             ```\n\
             var jobs = Channel.buffered:16;\n\
             var w = Worker.start:{ |ch| { ch.receive } .whileNotNil:{ |j| j.run } } args:#( jobs )\n\
             ```",
        )
        .class_method("start:args:backing:", |vm, mc, receiver, args| {
            let backing = crate::runtime::worker_service::backing_arg(args[2])?;
            start_block_worker(vm, mc, receiver, args[0], Some(args[1]), backing)
        })
        .doc("As `start:args:`, choosing the backing: 'thread' (the default) or 'process'.")
        // Portable blocks (docs/internal/CONCURRENCY_ARCH.md §10): ship the block's
        // template by reference plus a deep-copied SNAPSHOT of its free
        // reads; join returns the block's value. The portability scan
        // refuses write-captures, ^^, self/@fields, guarded blocks, and
        // class/method definition — loudly, at submit time.
        .class_method("start:", |vm, mc, receiver, args| {
            start_block_worker(vm, mc, receiver, args[0], None, "thread")
        })
        .doc(
            "Spawn a thread-backed worker from a portable BLOCK instead of a unit file: the \
             block's template ships by reference plus a deep-copied snapshot of its free \
             reads, and `join` answers the block's value (unlike a unit worker's nil). The \
             portability scan refuses write-captures, `^^`, `self`/`@fields`, guarded \
             blocks, and class/method definition -- loudly, at submit time. The block takes \
             no parameters; send it data through the lanes.\n\n\
             ```\n\
             var h = Worker.start:{ 21 * 2 };\n\
             h.join    \"* -> 42\n\
             ```",
        )
        .instance_method("send:", |vm, mc, receiver, args| {
            let (tx, backing, chan_link) = receiver
                .with_native_state::<NativeWorkerHandle, _, _>(|h| {
                    (h.inbox_tx.clone(), h.backing, h.chan_link)
                })
                .map_err(QuoinError::Other)?;
            let dv = if crate::runtime::channel_relay::is_channel_value(args[0]) {
                let chan =
                    crate::runtime::channel_relay::ship_for_crossing(vm, mc, args[0], chan_link)?;
                note_message();
                WorkerMsg::Channel(chan)
            } else {
                to_message(args[0], backing == "thread")?
            };
            tx.try_send(dv)
                .map_err(|_| QuoinError::Other("Worker.send: the worker has exited".into()))?;
            Ok(vm.new_nil(mc))
        })
        .doc(
            "Send a value into the worker's inbox (deep-copied; a thread-backed worker also \
             accepts a portable block, and a Channel crosses as a live endpoint -- the \
             worker's sends and receives on it relay back to this side). Raises if the \
             worker has exited. Answers nil.",
        )
        .instance_method("receive", |vm, mc, receiver, _args| {
            let (rx, chan_link) = receiver
                .with_native_state::<NativeWorkerHandle, _, _>(|h| {
                    (h.outbox_rx.clone(), h.chan_link)
                })
                .map_err(QuoinError::Other)?;
            match vm.await_io(IoRequest::WorkerRecv(rx))? {
                IoResult::WorkerMsg(Some(msg)) => from_message(vm, mc, &msg, chan_link),
                IoResult::WorkerMsg(None) => Ok(vm.new_nil(mc)),
                other => Err(QuoinError::Other(format!(
                    "Worker.receive: unexpected result {other:?}"
                ))),
            }
        })
        .doc(
            "Park until the worker sends a value back (its class-side `Worker.send:`); nil \
             once the worker has exited and its outbox is drained. Parks like any I/O wait, \
             so it composes with `Async.gather:` / `timeout:do:` / cancellation.",
        )
        // handle.label:'name' — restamp the registry row (VM.ps/psTree show
        // it; the Plan layer marks ownership and orphans this way).
        .instance_method("label:", |vm, _mc, receiver, args| {
            let label = args[0]
                .as_string()
                .ok_or_else(|| QuoinError::Other("label: expects a String".into()))?;
            let idx = receiver
                .with_native_state::<NativeWorkerHandle, _, _>(|h| h.reg_idx)
                .map_err(QuoinError::Other)?;
            if let Some(reg) = vm.worker_registry.get_mut(idx) {
                reg.label = label;
            }
            Ok(receiver)
        })
        .doc(
            "Restamp the worker's row in `VM.ps` / `VM.psTree` with a human-readable name; \
             answers the handle. The Plan layer marks ownership and orphans this way.",
        )
        // handle.terminate — REAL cancellation, process backing only (a
        // thread worker cannot be killed; orphan it instead). Idempotent;
        // join afterwards reports the exit as an error.
        .instance_method("terminate", |vm, _mc, receiver, _args| {
            let (grip, backing) = receiver
                .with_native_state::<NativeWorkerHandle, _, _>(|h| (h.grip.clone(), h.backing))
                .map_err(QuoinError::Other)?;
            let Some(grip) = grip else {
                return Err(QuoinError::Other(format!(
                    "terminate: only process-backed workers can be killed \
                     (this one is {backing}-backed) — orphan or join it instead"
                )));
            };
            // An explicit kill is an instruction, not a failure
            // (SUPERVISION.md §2): mark the peer STOPPED before the mailbox
            // reader can observe the corpse and call it died. `join` still
            // reports the death honestly (the done lane is untouched).
            let life_idx = receiver
                .with_native_state::<NativeWorkerHandle, _, _>(|h| h.life_idx)
                .map_err(QuoinError::Other)?;
            if let Some(sink) = vm.io.lives.borrow().get(life_idx) {
                sink.emit_stopped("terminated");
            }
            if let Some(c) = grip.lock().expect("child grip").as_mut() {
                let _ = c.kill();
            }
            Ok(Value::Nil)
        })
        .doc(
            "Kill a process-backed worker -- REAL cancellation, idempotent; a later `join` \
             reports the exit as an error. A thread-backed worker cannot be killed: this \
             raises, so orphan or join it instead.",
        )
        .instance_method("join", |vm, mc, receiver, _args| {
            let rx = receiver
                .with_native_state::<NativeWorkerHandle, _, _>(|h| {
                    if h.joined.get() {
                        None
                    } else {
                        h.joined.set(true);
                        Some(h.done_rx.clone())
                    }
                })
                .map_err(QuoinError::Other)?;
            let Some(rx) = rx else {
                return Err(QuoinError::Other("Worker.join: already joined".into()));
            };
            match vm.await_io(IoRequest::WorkerJoin(rx))? {
                IoResult::WorkerDone(Ok(dv)) => wire_to_value(vm, mc, &dv, None),
                // The body ran and reported — an ordinary catchable error.
                IoResult::WorkerDone(Err(crate::worker::WorkerExit::Failed(msg))) => {
                    Err(QuoinError::Other(msg))
                }
                // The isolate is gone (process vanished, thread panicked):
                // the typed death error (SUPERVISION.md §2), naming the
                // worker by its registry label.
                IoResult::WorkerDone(Err(crate::worker::WorkerExit::Died { reason, detail })) => {
                    let peer = receiver
                        .with_native_state::<NativeWorkerHandle, _, _>(|h| h.reg_idx)
                        .ok()
                        .and_then(|i| vm.worker_registry.get(i))
                        .map(|r| r.label.clone())
                        .unwrap_or_else(|| "worker".to_string());
                    Err(QuoinError::peer_died(peer, reason, detail))
                }
                other => Err(QuoinError::Other(format!(
                    "Worker.join: unexpected result {other:?}"
                ))),
            }
        })
        .doc(
            "Park until the worker finishes. A `Worker.start:` block worker answers the \
             block's value (copied); a unit worker answers nil. Raises the worker's error, \
             catchably, if it failed. A handle can be joined once -- a second join raises.",
        )
        .instance_method("events", |vm, mc, receiver, _args| {
            let life_idx = receiver
                .with_native_state::<NativeWorkerHandle, _, _>(|h| h.life_idx)
                .map_err(QuoinError::Other)?;
            life_events_channel(vm, mc, life_idx)
        })
        .doc(
            "This worker's lifecycle events, as a Channel of Maps -- 'kind' is 'spawned', \
             'stopped' (a clean finish, a reported failure, `terminate`), or 'died' (the \
             isolate VANISHED -- with 'reason' and 'message'; SUPERVISION.md). The channel \
             closes after the terminal event. Events are kept from spawn time, so a late \
             consumer still sees the history; asking twice answers the same channel.",
        )
        // ---- lifecycle plumbing (SUPERVISION.md slice 1, internal) ----
        .class_method("lifeNext:", |vm, mc, _receiver, args| {
            let idx = args[0]
                .as_i64()
                .ok_or_else(|| QuoinError::Other("Worker.lifeNext: expects a peer index".into()))?
                as usize;
            let rx = vm
                .io
                .lives
                .borrow()
                .get(idx)
                .map(|sink| sink.rx.clone())
                .ok_or_else(|| QuoinError::Other("Worker.lifeNext: unknown peer".into()))?;
            match vm.await_io(IoRequest::WorkerRecv(rx))? {
                // Build the record map directly rather than through the wire
                // walkers: `reason` is a SYMBOL on every other surface
                // (`PeerDiedError.reason`, `VM.peers`) and wire data cannot
                // carry symbols, so the staging carries its name and the
                // boundary re-mints it here.
                IoResult::WorkerMsg(Some(WorkerMsg::Data(WireData::Map(fields)))) => {
                    let entries: Vec<(String, Value)> = fields
                        .iter()
                        .map(|(k, v)| {
                            let text = match v {
                                WireData::Str(t) => t.clone(),
                                other => format!("{other:?}"),
                            };
                            let val = if k == "reason" {
                                vm.new_symbol(mc, text)
                            } else {
                                vm.new_string(mc, text)
                            };
                            (k.clone(), val)
                        })
                        .collect();
                    Ok(vm.new_map(mc, entries))
                }
                // Staging closed (the terminal event was delivered): end of stream.
                IoResult::WorkerMsg(_) => Ok(Value::Nil),
                other => Err(QuoinError::Other(format!(
                    "Worker.lifeNext: unexpected result {other:?}"
                ))),
            }
        })
        .doc(
            "Internal (SUPERVISION.md slice 1): park for the next staged lifecycle event of \
             peer N, nil when the stream ends. The `LifecycleBoot` pump's wait -- the \
             `events` channels are fed through this; not a user-facing surface.",
        )
        .class_method("lifeWatch:", |vm, _mc, _receiver, args| {
            let idx = args[0]
                .as_i64()
                .ok_or_else(|| QuoinError::Other("Worker.lifeWatch: expects a peer index".into()))?
                as usize;
            let sink = vm
                .io
                .lives
                .borrow()
                .get(idx)
                .cloned()
                .ok_or_else(|| QuoinError::Other("Worker.lifeWatch: unknown peer".into()))?;
            let Some(pid) = sink.pid else {
                return Ok(Value::Nil);
            };
            if sink.is_terminal() {
                return Ok(Value::Nil);
            }
            let _ = vm.await_io(IoRequest::ChildExit { pid })?;
            // First terminal wins: a death the lazy path (or a stop) already
            // observed makes this a no-op.
            sink.emit_died(
                crate::error::PeerDeathReason::Exited,
                "process exited (observed by the exit watch)",
            );
            Ok(Value::Nil)
        })
        .doc(
            "Internal (SUPERVISION.md slice 1): park until peer N's child process exits \
             (kqueue/pidfd -- observation, never a reap) and record the death. Armed once \
             per extension by the first `events` ask; not a user-facing surface.",
        )
        .class_method("superviseService:", |vm, mc, _receiver, args| {
            let svc = *args.first().ok_or_else(|| {
                QuoinError::Other("Worker.superviseService: expects a service proxy".into())
            })?;
            crate::runtime::worker_service::supervise_service_loop(vm, mc, svc)
        })
        .doc(
            "Internal (SUPERVISION.md slice 3): the per-service supervisor loop -- parks on \
             the peer's terminal, runs the policy's restart cycle, gives up when the budget \
             is spent. Spawned once per `serviceSupervise:` by the `SuperviseBoot` helper; \
             not a user-facing surface.",
        )
        .class_method("superviseExtension:", |vm, mc, _receiver, args| {
            let ext = *args.first().ok_or_else(|| {
                QuoinError::Other("Worker.superviseExtension: expects an Extension".into())
            })?;
            crate::runtime::extension::supervise_extension_loop(vm, mc, ext)
        })
        .doc(
            "Internal (SUPERVISION.md slice 3): the per-extension supervisor loop -- the \
             `superviseService:` twin over `Extension.restart`'s machinery, re-arming the \
             exit watch for each new incarnation. Not a user-facing surface.",
        )
        // ---- worker side (class-side lanes, live only inside a worker) ----
        .class_method("receive", |vm, mc, _receiver, _args| {
            let Some(link) = vm.worker_link.as_ref() else {
                return Err(QuoinError::Other(
                    "Worker.receive: not inside a worker (spawn one with Worker.spawn:)".into(),
                ));
            };
            let rx = link.inbox_rx.clone();
            let chan_link = vm.parent_chan_link.unwrap_or(0);
            match vm.await_io(IoRequest::WorkerRecv(rx))? {
                IoResult::WorkerMsg(Some(msg)) => from_message(vm, mc, &msg, chan_link),
                IoResult::WorkerMsg(None) => Ok(vm.new_nil(mc)),
                other => Err(QuoinError::Other(format!(
                    "Worker.receive: unexpected result {other:?}"
                ))),
            }
        })
        .doc(
            "Worker side (inside a spawned unit): park for the next value from the parent; \
             nil once the parent's lane is closed and drained. Raises when not inside a \
             worker.",
        )
        .class_method("send:", |vm, mc, _receiver, args| {
            let Some(link) = vm.worker_link.as_ref() else {
                return Err(QuoinError::Other("Worker.send: not inside a worker".into()));
            };
            let tx = link.outbox_tx.clone();
            let allow_blocks = !link.process;
            let dv = if crate::runtime::channel_relay::is_channel_value(args[0]) {
                let chan_link = vm.parent_chan_link.ok_or_else(|| {
                    QuoinError::Other("Worker.send: no relay link to the parent".into())
                })?;
                let chan =
                    crate::runtime::channel_relay::ship_for_crossing(vm, mc, args[0], chan_link)?;
                note_message();
                WorkerMsg::Channel(chan)
            } else {
                to_message(args[0], allow_blocks)?
            };
            tx.try_send(dv)
                .map_err(|_| QuoinError::Other("Worker.send: the parent has gone away".into()))?;
            Ok(vm.new_nil(mc))
        })
        .doc(
            "Worker side (inside a spawned unit): send a value to the parent's `receive` \
             lane (deep-copied). Raises when not inside a worker, or when the parent has \
             gone away. Answers nil.",
        )
        // ---- hosting (ACTOR_OBJECTS.md §2): worker-resident objects behind
        // real installed proxy classes.
        .class_method("host:with:", |vm, mc, receiver, args| {
            let path = crate::runtime::worker_service::string_arg(args[0], "the unit path")?;
            crate::runtime::worker_service::host_block(
                vm,
                mc,
                receiver,
                Some(path),
                args[1],
                1,
                "thread",
                None,
            )
        })
        .doc(
            "Host with an INIT BLOCK: spawn a worker running the unit, ship the \
             portable block, run it there, and host the object it answers -- real \
             constructor arguments for hosted objects. On 'process' backing the \
             block crosses as its SOURCE TEXT plus a snapshot of its captures, so \
             it must come from source (not assembled at runtime).\n\n\
             ```\n\
             var pool = Worker.host:'db.qn' with:{ Pool.new:{ size = 8 } }\n\
             ```",
        )
        .class_method("host:with:lanes:", |vm, mc, receiver, args| {
            let path = crate::runtime::worker_service::string_arg(args[0], "the unit path")?;
            let lanes = crate::runtime::worker_service::lanes_arg(args[2])?;
            crate::runtime::worker_service::host_block(
                vm,
                mc,
                receiver,
                Some(path),
                args[1],
                lanes,
                "thread",
                None,
            )
        })
        .doc("As `host:with:` with N concurrent lanes.")
        .class_method("host:with:backing:", |vm, mc, receiver, args| {
            let path = crate::runtime::worker_service::string_arg(args[0], "the unit path")?;
            let backing = crate::runtime::worker_service::backing_arg(args[2])?;
            crate::runtime::worker_service::host_block(
                vm,
                mc,
                receiver,
                Some(path),
                args[1],
                1,
                backing,
                None,
            )
        })
        .doc(
            "As `host:with:`, choosing the backing: 'thread' (the default) or \
             'process' -- a child qn process; the block ships as source + captures.",
        )
        .class_method("host:with:backing:lanes:", |vm, mc, receiver, args| {
            let path = crate::runtime::worker_service::string_arg(args[0], "the unit path")?;
            let backing = crate::runtime::worker_service::backing_arg(args[2])?;
            let lanes = crate::runtime::worker_service::lanes_arg(args[3])?;
            crate::runtime::worker_service::host_block(
                vm,
                mc,
                receiver,
                Some(path),
                args[1],
                lanes,
                backing,
                None,
            )
        })
        .doc("As `host:with:backing:` with N concurrent lanes.")
        .class_method("with:", |vm, mc, receiver, args| {
            crate::runtime::worker_service::host_block(vm, mc, receiver, None, args[0], 1, "thread", None)
        })
        .doc(
            "Host the object a portable block answers, with no unit -- `host:with:` \
             without the host: the block runs in a fresh worker that booted qnlib \
             only, so it can reach stdlib classes but not the parent's definitions \
             (put those in a unit and use `host:with:`).\n\n\
             ```\n\
             var clock = Worker.with:{ Timer.new }\n\
             ```",
        )
        .class_method("with:lanes:", |vm, mc, receiver, args| {
            let lanes = crate::runtime::worker_service::lanes_arg(args[1])?;
            crate::runtime::worker_service::host_block(
                vm, mc, receiver, None, args[0], lanes, "thread", None,
            )
        })
        .doc("As `with:` with N concurrent lanes.")
        .class_method("with:backing:", |vm, mc, receiver, args| {
            let backing = crate::runtime::worker_service::backing_arg(args[1])?;
            crate::runtime::worker_service::host_block(vm, mc, receiver, None, args[0], 1, backing, None)
        })
        .doc(
            "As `with:`, choosing the backing: 'thread' (the default) or 'process' -- \
             a child qn process booting bare qnlib; the block ships as source + \
             captures.",
        )
        .class_method("with:backing:lanes:", |vm, mc, receiver, args| {
            let backing = crate::runtime::worker_service::backing_arg(args[1])?;
            let lanes = crate::runtime::worker_service::lanes_arg(args[2])?;
            crate::runtime::worker_service::host_block(
                vm, mc, receiver, None, args[0], lanes, backing, None,
            )
        })
        .doc("As `with:backing:` with N concurrent lanes.")
        .class_method("host:with:args:", |vm, mc, receiver, args| {
            let path = crate::runtime::worker_service::string_arg(args[0], "the unit path")?;
            crate::runtime::worker_service::host_block(
                vm,
                mc,
                receiver,
                Some(path),
                args[1],
                1,
                "thread",
                Some(args[2]),
            )
        })
        .doc(
            "As `host:with:`, passing ARGUMENTS to the block's parameters: each args: \
             element crosses by the spawn rules -- portable values snapshot, a Channel \
             becomes a live endpoint in the worker (the way to hand a hosted object a \
             lane to talk on), a portable block crosses as a callable; anything else \
             refuses loudly before the worker sees it. Arity is checked here, before \
             anything ships.\n\n\
             ```\n\
             var results = Channel.buffered:16;\n\
             var pool = Worker.host:'db.qn' with:{ |out| Pool.new.reportTo:out } args:#( results )\n\
             ```",
        )
        .class_method("host:with:args:lanes:", |vm, mc, receiver, args| {
            let path = crate::runtime::worker_service::string_arg(args[0], "the unit path")?;
            let lanes = crate::runtime::worker_service::lanes_arg(args[3])?;
            crate::runtime::worker_service::host_block(
                vm,
                mc,
                receiver,
                Some(path),
                args[1],
                lanes,
                "thread",
                Some(args[2]),
            )
        })
        .doc("As `host:with:args:` with N concurrent lanes.")
        .class_method("host:with:args:backing:", |vm, mc, receiver, args| {
            let path = crate::runtime::worker_service::string_arg(args[0], "the unit path")?;
            let backing = crate::runtime::worker_service::backing_arg(args[3])?;
            crate::runtime::worker_service::host_block(
                vm,
                mc,
                receiver,
                Some(path),
                args[1],
                1,
                backing,
                Some(args[2]),
            )
        })
        .doc("As `host:with:args:`, choosing the backing: 'thread' (the default) or 'process'.")
        .class_method("host:with:args:backing:lanes:", |vm, mc, receiver, args| {
            let path = crate::runtime::worker_service::string_arg(args[0], "the unit path")?;
            let backing = crate::runtime::worker_service::backing_arg(args[3])?;
            let lanes = crate::runtime::worker_service::lanes_arg(args[4])?;
            crate::runtime::worker_service::host_block(
                vm,
                mc,
                receiver,
                Some(path),
                args[1],
                lanes,
                backing,
                Some(args[2]),
            )
        })
        .doc("As `host:with:args:backing:` with N concurrent lanes.")
        .class_method("with:args:", |vm, mc, receiver, args| {
            crate::runtime::worker_service::host_block(
                vm,
                mc,
                receiver,
                None,
                args[0],
                1,
                "thread",
                Some(args[1]),
            )
        })
        .doc(
            "As `with:`, passing ARGUMENTS to the block's parameters (see \
             `host:with:args:` for the crossing rules).\n\n\
             ```\n\
             var out = Channel.buffered:8;\n\
             var agg = Worker.with:{ |ch| Aggregator.new.drain:ch } args:#( out )\n\
             ```",
        )
        .class_method("with:args:lanes:", |vm, mc, receiver, args| {
            let lanes = crate::runtime::worker_service::lanes_arg(args[2])?;
            crate::runtime::worker_service::host_block(
                vm,
                mc,
                receiver,
                None,
                args[0],
                lanes,
                "thread",
                Some(args[1]),
            )
        })
        .doc("As `with:args:` with N concurrent lanes.")
        .class_method("with:args:backing:", |vm, mc, receiver, args| {
            let backing = crate::runtime::worker_service::backing_arg(args[2])?;
            crate::runtime::worker_service::host_block(
                vm,
                mc,
                receiver,
                None,
                args[0],
                1,
                backing,
                Some(args[1]),
            )
        })
        .doc("As `with:args:`, choosing the backing: 'thread' (the default) or 'process'.")
        .class_method("with:args:backing:lanes:", |vm, mc, receiver, args| {
            let backing = crate::runtime::worker_service::backing_arg(args[2])?;
            let lanes = crate::runtime::worker_service::lanes_arg(args[3])?;
            crate::runtime::worker_service::host_block(
                vm,
                mc,
                receiver,
                None,
                args[0],
                lanes,
                backing,
                Some(args[1]),
            )
        })
        .doc("As `with:args:backing:` with N concurrent lanes.")
        .class_method("worker?", |vm, mc, _receiver, _args| {
            Ok(vm.new_bool(mc, vm.worker_link.is_some()))
        })
        .doc(
            "True inside a spawned worker, false in the main program -- how a unit that can \
             run both ways tells which side it is on.",
        )
}

/// Value → String helper used by `spawn:` (mirrors `arg!` string extraction
/// without the macro's class plumbing).
trait AsStringArg {
    fn as_string(&self) -> Option<String>;
}

impl<'gc> AsStringArg for Value<'gc> {
    fn as_string(&self) -> Option<String> {
        match self {
            Value::Object(obj) => match &obj.borrow().payload {
                crate::value::ObjectPayload::String(s) => Some((**s).clone()),
                _ => None,
            },
            _ => None,
        }
    }
}
