//! `WorkerService` — hosted objects on the peer protocol (docs/internal/ACTOR_OBJECTS.md
//! §2; the L4 of docs/internal/CONCURRENCY_ARCH.md §10, converged): host a class in a
//! dedicated worker isolate and get a PROXY whose ordinary method sends become
//! peer-protocol `Call` frames. Sticky state, serialized access — an actor.
//!
//! ```text
//! var index = WorkerService.host:'search/index.qn' class:'SearchIndex';
//! index.add:doc;
//! var hits = index.query:'quoin';
//! ```
//!
//! The proxy forwards through the dispatch MNU seam: a selector the proxy's own
//! class doesn't define (everything except `serviceStop`) builds a
//! `Call{class_name, op, recv, method_args}` and parks for its `CallReturn*`
//! terminal — so callers compose with `Async.gather:`/`timeout:do:` like any
//! parked wait, and the hook costs nothing on the hot path (it sits on the
//! lookup-miss branch). The hook is TEMPORARY by decision (2026-07-14): once
//! hosted classes declare their selectors, the parent installs a real class
//! (the `install_ext_class` pattern) and this seam goes away — see
//! ACTOR_OBJECTS.md §10.
//!
//! HOSTED RETURNS — the actor-object rule: a method's portable return COPIES
//! back (`CallReturnData`); a non-portable object return is HOSTED in the
//! worker's table and comes back as `CallReturnResource`, which this side wraps
//! as a SUB-PROXY (same worker, its own object id). Sub-proxies are ordinary
//! receivers — including as ARGUMENTS to further calls on the same worker,
//! where they travel as live references (`Arg::Resource`). A dropped proxy's id
//! is reaped and flushed on the next call (`Call.releases`).
//!
//! SERIALIZATION: the service processes one call at a time by construction (one
//! serve loop), and callers serialize on a one-token internal channel — take
//! the token (parks fairly), dispatch, park for the terminal, return the token.
//! The per-object claim machinery (ACTOR_OBJECTS.md §5) replaces this token in
//! the mailboxes+lanes slice.
//!
//! Errors in the hosted method — including MessageNotUnderstood — come back as
//! `CallReturnError` and raise catchably at the call site, carrying the
//! worker's rendered stack as `ex.remoteStack` (the extension error shape).

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gc_arena::Collect;
use gc_arena::collect::Trace;
use quoin_ext_proto::{Arg, DataValue as WireData, Msg};

use crate::error::QuoinError;
use crate::io_backend::{IoRequest, IoResult};
use crate::runtime::extension::{truncate_blob, value_to_wire, wire_to_value};
use crate::runtime::worker::block_parts;
use crate::symbol::Symbol;
use crate::value::{AnyCollect, NativeClassBuilder, Value};
use crate::vm::VmState;
use crate::worker::{
    DispatchReq, OP_STOP, PortableBlock, WorkerMsg, note_message, snapshot_block,
    spawn_worker_service,
};

/// Proxy-side state: the worker's dispatch lane plus this proxy's hosted-object
/// id. Everything worker-wide (lanes, token, stop flag, reap queue) is shared by
/// every proxy of the worker; only `object_id`/`class_name` are per-proxy.
#[derive(Debug)]
pub struct NativeServiceState {
    dispatch_tx: async_channel::Sender<DispatchReq>,
    done_rx: async_channel::Receiver<Result<WireData, String>>,
    /// One-token call serializer (see the module doc).
    token_tx: async_channel::Sender<WorkerMsg>,
    token_rx: async_channel::Receiver<WorkerMsg>,
    /// Worker-wide stop flag — a stopped service refuses calls from every proxy.
    stopped: Rc<Cell<bool>>,
    /// Dropped-proxy ids awaiting flush as `Call.releases` (the reap pattern:
    /// a GC `Drop` can't send a frame).
    reap: Rc<RefCell<Vec<u64>>>,
    /// This proxy's hosted-object id (the root instance is 1).
    object_id: u64,
    /// The hosted object's class name — routes the dispatch worker-side.
    class_name: String,
    /// True for process backing: block arguments refuse at the encode seam
    /// (templates are in-process references; no source-shipping yet —
    /// ACTOR_OBJECTS.md §3a).
    process: bool,
}

impl Drop for NativeServiceState {
    fn drop(&mut self) {
        self.reap.borrow_mut().push(self.object_id);
    }
}

impl AnyCollect for NativeServiceState {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn trace_gc<'gc>(&self, _cc: &mut dyn Trace<'gc>) {}
}

unsafe impl<'gc> Collect<'gc> for NativeServiceState {
    const NEEDS_TRACE: bool = false;
}

/// The per-call snapshot of a proxy's state (cloned out so the native-state
/// borrow ends before any park).
struct CallCtx {
    dispatch_tx: async_channel::Sender<DispatchReq>,
    done_rx: async_channel::Receiver<Result<WireData, String>>,
    token_tx: async_channel::Sender<WorkerMsg>,
    token_rx: async_channel::Receiver<WorkerMsg>,
    stopped: Rc<Cell<bool>>,
    reap: Rc<RefCell<Vec<u64>>>,
    object_id: u64,
    class_name: String,
    process: bool,
}

fn snapshot(s: &NativeServiceState) -> CallCtx {
    CallCtx {
        dispatch_tx: s.dispatch_tx.clone(),
        done_rx: s.done_rx.clone(),
        token_tx: s.token_tx.clone(),
        token_rx: s.token_rx.clone(),
        stopped: s.stopped.clone(),
        reap: s.reap.clone(),
        object_id: s.object_id,
        class_name: s.class_name.clone(),
        process: s.process,
    }
}

/// The dispatch MNU-seam hook (see `exec_send` / `call_method_cached_inner`):
/// `None` means "not a service proxy — raise the MNU as usual".
pub(crate) fn try_service_call<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    receiver: Value<'gc>,
    selector: Symbol,
    args: &[Value<'gc>],
) -> Option<Result<Value<'gc>, QuoinError>> {
    let ctx = receiver
        .with_native_state::<NativeServiceState, _, _>(snapshot)
        .ok()?;
    Some(service_call(vm, mc, receiver, ctx, selector, args))
}

/// A bare `Call` frame for a hosted-object dispatch.
fn hosted_call(ctx: &CallCtx, op: String, method_args: Vec<Arg>, releases: Vec<u64>) -> Msg {
    Msg::Call {
        op,
        arg: String::new(),
        handles: Vec::new(),
        resources: Vec::new(),
        releases,
        arrays: Vec::new(),
        data: None,
        class_name: ctx.class_name.clone(),
        recv: ctx.object_id,
        method_args,
    }
}

fn service_call<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    receiver: Value<'gc>,
    ctx: CallCtx,
    selector: Symbol,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, QuoinError> {
    if ctx.stopped.get() {
        return Err(QuoinError::Other(format!(
            "service call '{}': the service is stopped",
            selector.as_str()
        )));
    }
    // Encode BEFORE taking the token: a refused argument shouldn't occupy the
    // service. A proxy of the SAME worker travels as a live reference; a
    // portable BLOCK ships to a thread peer as a capture snapshot riding the
    // dispatch request out-of-band (§3a — the in-memory lane's
    // richer-than-wire allowance), with a Null placeholder holding its
    // position in the frame.
    let mut method_args = Vec::with_capacity(args.len());
    let mut blocks: Vec<(usize, PortableBlock)> = Vec::new();
    for (i, a) in args.iter().enumerate() {
        let same_worker_id = a
            .with_native_state::<NativeServiceState, _, _>(|s| {
                Rc::ptr_eq(&s.reap, &ctx.reap).then_some(s.object_id)
            })
            .ok()
            .flatten();
        if let Some(id) = same_worker_id {
            method_args.push(Arg::Resource(id));
            continue;
        }
        if let Some((template, parent_env)) = block_parts(*a) {
            if ctx.process {
                return Err(QuoinError::Other(format!(
                    "service call '{}': argument {} is a block — blocks cannot cross \
                     a process boundary (templates are in-process references); use \
                     thread backing",
                    selector.as_str(),
                    i + 1
                )));
            }
            let pb = snapshot_block(template, parent_env, 0).map_err(|e| {
                QuoinError::Other(format!(
                    "service call '{}': argument {}: {e}",
                    selector.as_str(),
                    i + 1
                ))
            })?;
            blocks.push((i, pb));
            method_args.push(Arg::Data(WireData::Null));
            continue;
        }
        method_args.push(Arg::Data(value_to_wire(*a, None).map_err(|e| {
            QuoinError::Other(format!(
                "service call '{}': argument {} is not portable: {e}",
                selector.as_str(),
                i + 1
            ))
        })?));
    }

    // Serialize: take the token (parks fairly behind other callers).
    match vm.await_io(IoRequest::WorkerRecv(ctx.token_rx.clone()))? {
        IoResult::WorkerMsg(Some(_)) => {}
        _ => {
            return Err(QuoinError::Other(format!(
                "service call '{}': the service is stopped",
                selector.as_str()
            )));
        }
    }
    note_message();
    let releases: Vec<u64> = ctx.reap.borrow_mut().drain(..).collect();
    let (reply_tx, reply_rx) = async_channel::bounded::<Msg>(1);
    let frame = hosted_call(&ctx, selector.as_str().to_string(), method_args, releases);
    if ctx
        .dispatch_tx
        .try_send(DispatchReq {
            frame,
            blocks,
            reply: reply_tx,
        })
        .is_err()
    {
        let _ = ctx.token_tx.try_send(WorkerMsg::Data(WireData::Null));
        return Err(QuoinError::Other(format!(
            "service call '{}': the service has exited",
            selector.as_str()
        )));
    }
    let reply = vm.await_io(IoRequest::FrameRecv(reply_rx))?;
    let _ = ctx.token_tx.try_send(WorkerMsg::Data(WireData::Null));
    match reply {
        IoResult::FrameMsg(Some(msg)) => interpret_terminal(vm, mc, receiver, &ctx, selector, *msg),
        IoResult::FrameMsg(None) => Err(QuoinError::Other(format!(
            "service call '{}': the service exited mid-call",
            selector.as_str()
        ))),
        other => Err(QuoinError::Other(format!(
            "service call '{}': unexpected result {other:?}",
            selector.as_str()
        ))),
    }
}

/// Materialize a `CallReturn*` terminal: data through the wire walkers, a hosted
/// resource as a SUB-PROXY of the same worker, an error as the extension error
/// shape (message + `ex.remoteStack`).
fn interpret_terminal<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    receiver: Value<'gc>,
    ctx: &CallCtx,
    selector: Symbol,
    msg: Msg,
) -> Result<Value<'gc>, QuoinError> {
    match msg {
        Msg::CallReturnData { value } => wire_to_value(vm, mc, &value, None),
        Msg::CallReturnResource {
            resource,
            class_name,
        } => {
            let Value::Object(obj) = receiver else {
                return Err(QuoinError::Other(format!(
                    "service call '{}': bad proxy receiver",
                    selector.as_str()
                )));
            };
            let class = obj.borrow().class;
            Ok(vm.new_native_state(
                mc,
                class,
                NativeServiceState {
                    dispatch_tx: ctx.dispatch_tx.clone(),
                    done_rx: ctx.done_rx.clone(),
                    token_tx: ctx.token_tx.clone(),
                    token_rx: ctx.token_rx.clone(),
                    stopped: ctx.stopped.clone(),
                    reap: ctx.reap.clone(),
                    object_id: resource,
                    class_name,
                    process: ctx.process,
                },
            ))
        }
        Msg::CallReturnError {
            message,
            remote_stack,
        } => Err(QuoinError::ExtensionError {
            message,
            remote_stack: truncate_blob(remote_stack),
        }),
        other => Err(QuoinError::Other(format!(
            "service call '{}': unexpected terminal {other:?}",
            selector.as_str()
        ))),
    }
}

fn host<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    receiver: Value<'gc>,
    path: String,
    class_name: String,
    backing: &'static str,
) -> Result<Value<'gc>, QuoinError> {
    let Value::Class(class) = receiver else {
        return Err(QuoinError::Other("WorkerService: bad receiver".into()));
    };
    let (ch, pid) = match backing {
        "process" => {
            let (ch, pid, _grip) =
                crate::worker::spawn_worker_process(path.clone(), Some(class_name.clone()))
                    .map_err(QuoinError::Other)?;
            (ch, Some(pid))
        }
        _ => (spawn_worker_service(path.clone(), class_name.clone()), None),
    };
    vm.worker_registry.push(crate::worker::WorkerReg {
        unit: format!("svc:{path}"),
        label: format!("svc:{path}"),
        backing,
        pid,
        inbox_tx: ch.inbox_tx.clone(),
        outbox_rx: ch.outbox_rx.clone(),
        control_tx: ch.control_tx.clone(),
    });
    // Handshake: the serve loop's first act is a 'ready' message; a closed lane
    // instead means boot/compile/instantiation failed — the done lane says
    // why. Parks, so slow boots don't block other tasks.
    match vm.await_io(IoRequest::WorkerRecv(ch.outbox_rx.clone()))? {
        IoResult::WorkerMsg(Some(_)) => {}
        IoResult::WorkerMsg(None) => {
            let why = match vm.await_io(IoRequest::WorkerJoin(ch.done_rx.clone()))? {
                IoResult::WorkerDone(Err(msg)) => msg,
                _ => "the service exited before reporting ready".to_string(),
            };
            return Err(QuoinError::Other(format!("WorkerService.host: {why}")));
        }
        other => {
            return Err(QuoinError::Other(format!(
                "WorkerService.host: unexpected result {other:?}"
            )));
        }
    }
    let (token_tx, token_rx) = async_channel::bounded(1);
    let _ = token_tx.try_send(WorkerMsg::Data(WireData::Null));
    Ok(vm.new_native_state(
        mc,
        class,
        NativeServiceState {
            dispatch_tx: ch.dispatch_tx,
            done_rx: ch.done_rx,
            token_tx,
            token_rx,
            stopped: Rc::new(Cell::new(false)),
            reap: Rc::new(RefCell::new(Vec::new())),
            object_id: 1,
            class_name,
            process: backing == "process",
        },
    ))
}

fn string_arg<'gc>(v: Value<'gc>, what: &str) -> Result<String, QuoinError> {
    match v {
        Value::Object(obj) => match &obj.borrow().payload {
            crate::value::ObjectPayload::String(s) => Ok((**s).clone()),
            _ => Err(QuoinError::Other(format!(
                "WorkerService: {what} must be a String"
            ))),
        },
        _ => Err(QuoinError::Other(format!(
            "WorkerService: {what} must be a String"
        ))),
    }
}

pub fn build_worker_service_class() -> NativeClassBuilder {
    NativeClassBuilder::new("WorkerService", Some("Object"))
        .construct_with("use WorkerService.host:class:")
        .class_doc(
            "Host a class in a dedicated worker isolate and get a PROXY whose ordinary \
             method sends become peer-protocol calls: sticky state with serialized \
             access -- an actor, effectively. Portable arguments and returns deep-copy; \
             a method that returns a NON-portable object HOSTS it -- the answer is a \
             sub-proxy addressing it, usable like any receiver (including as an argument \
             to further calls on the same service). A portable BLOCK argument ships to a \
             thread-backed service and runs worker-side -- one crossing however many \
             times the method invokes it (unportable blocks, and any block to a \
             process-backed service, refuse with a clear error). Errors in the hosted method raise \
             catchably at the call site, with the worker's stack as `ex.remoteStack`; \
             one call runs at a time (concurrent callers queue fairly).\n\n\
             ```\n\
             var index = WorkerService.host:'search/index.qn' class:'SearchIndex';\n\
             index.add:doc;\n\
             var hits = index.query:'quoin'\n\
             ```",
        )
        .class_method("host:class:", |vm, mc, receiver, args| {
            let path = string_arg(args[0], "the unit path")?;
            let class_name = string_arg(args[1], "the class name")?;
            host(vm, mc, receiver, path, class_name, "thread")
        })
        .doc(
            "Spawn a worker running the unit at the path, instantiate the named class in it \
             (`TheClass.new`), and answer the proxy once the service reports ready. Every \
             selector the proxy doesn't define itself (everything except `serviceStop`) \
             forwards as a call and parks for the reply, so calls compose with \
             `Async.gather:` / `timeout:do:` like any parked wait.",
        )
        .class_method("host:class:backing:", |vm, mc, receiver, args| {
            let path = string_arg(args[0], "the unit path")?;
            let class_name = string_arg(args[1], "the class name")?;
            let backing = string_arg(args[2], "the backing")?;
            match backing.as_str() {
                "thread" => host(vm, mc, receiver, path, class_name, "thread"),
                "process" => host(vm, mc, receiver, path, class_name, "process"),
                other => Err(QuoinError::Other(format!(
                    "WorkerService: unknown backing '{other}' (thread|process)"
                ))),
            }
        })
        .doc(
            "As `host:class:`, choosing the backing at spawn time: 'thread' (the default) \
             or 'process' (a child qn process -- the escape from the in-process thread \
             ceiling for compute-heavy services).",
        )
        // Stop the service: waits for in-flight calls (takes the token), sends
        // the reserved stop op, and joins the worker. Worker-wide: every proxy
        // of the service refuses calls afterwards.
        .instance_method("serviceStop", |vm, mc, receiver, _args| {
            let ctx = receiver
                .with_native_state::<NativeServiceState, _, _>(|s| {
                    s.stopped.set(true);
                    snapshot(s)
                })
                .map_err(QuoinError::Other)?;
            // Drain the token so in-flight calls finish first; ignore a closed
            // token lane (double stop).
            let _ = vm.await_io(IoRequest::WorkerRecv(ctx.token_rx.clone()))?;
            // The reserved stop op ends the serve loop; a dead worker skips
            // straight to the join, which reports why.
            let (reply_tx, reply_rx) = async_channel::bounded::<Msg>(1);
            let frame = hosted_call(&ctx, OP_STOP.to_string(), Vec::new(), Vec::new());
            if ctx
                .dispatch_tx
                .try_send(DispatchReq {
                    frame,
                    blocks: Vec::new(),
                    reply: reply_tx,
                })
                .is_ok()
            {
                let _ = vm.await_io(IoRequest::FrameRecv(reply_rx))?;
            }
            match vm.await_io(IoRequest::WorkerJoin(ctx.done_rx.clone()))? {
                IoResult::WorkerDone(Ok(_)) => Ok(vm.new_nil(mc)),
                IoResult::WorkerDone(Err(msg)) => Err(QuoinError::Other(msg)),
                other => Err(QuoinError::Other(format!(
                    "serviceStop: unexpected result {other:?}"
                ))),
            }
        })
        .doc(
            "Stop the service: wait for in-flight calls to finish, send the stop message, \
             and join the worker. Worker-wide -- further calls through ANY proxy of this \
             service raise 'the service is stopped'. Answers nil.",
        )
}
