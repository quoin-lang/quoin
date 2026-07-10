//! `WorkerService` — L4 of the concurrency stack (docs/CONCURRENCY_ARCH.md
//! §10): host a class in a dedicated worker isolate and get a PROXY whose
//! ordinary method sends become RPC over the worker lanes. Sticky state,
//! serialized access — an actor, effectively.
//!
//! ```text
//! var index = WorkerService.host:'search/index.qn' class:'SearchIndex';
//! index.add:doc;
//! var hits = index.query:'quoin';
//! ```
//!
//! The proxy forwards through the dispatch MNU seam: a selector the proxy's
//! own class doesn't define (everything except `serviceStop`) builds a call
//! frame, ships it, and parks for the reply — so callers compose with
//! `Async.gather:`/`timeout:do:` like any parked wait, and the hook costs
//! nothing on the hot path (it sits on the lookup-miss branch).
//!
//! SERIALIZATION: the service processes one call at a time by construction
//! (one loop), and callers serialize on a one-token internal channel — take
//! the token (parks fairly on the existing machinery), send, receive the
//! reply, return the token. Without it, two concurrent callers' replies
//! could cross on the shared outbox lane.
//!
//! Arguments and returns follow the wire taxonomy (data crosses,
//! blocks/instances refuse — same as extension calls). Errors in the hosted
//! method — including MessageNotUnderstood from a bad selector — come back
//! as the reply's 'err' and raise catchably at the call site.
//!
//! BACKING is a spawn-time option (`host:class:backing:`): 'thread' is v1;
//! 'process' is designed (the extension wire, verbatim) but not yet
//! implemented — it errors clearly rather than pretending.

use std::any::Any;

use gc_arena::Collect;
use gc_arena::collect::Trace;
use quoin_ext_proto::DataValue as WireData;

use crate::error::QuoinError;
use crate::io_backend::{IoRequest, IoResult};
use crate::runtime::extension::{value_to_wire, wire_to_value};
use crate::symbol::Symbol;
use crate::value::{AnyCollect, NativeClassBuilder, Value};
use crate::vm::VmState;
use crate::worker::{WorkerMsg, note_message, spawn_worker_service};

/// Proxy-side state: the service worker's lanes plus the one-token call
/// serializer. Plain `Send` data.
#[derive(Debug)]
pub struct NativeServiceState {
    inbox_tx: async_channel::Sender<WorkerMsg>,
    outbox_rx: async_channel::Receiver<WorkerMsg>,
    done_rx: async_channel::Receiver<Result<WireData, String>>,
    token_tx: async_channel::Sender<WorkerMsg>,
    token_rx: async_channel::Receiver<WorkerMsg>,
    stopped: std::cell::Cell<bool>,
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

/// The dispatch MNU-seam hook (see `exec_send` / `call_method_cached_inner`):
/// `None` means "not a service proxy — raise the MNU as usual".
pub(crate) fn try_service_call<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    receiver: Value<'gc>,
    selector: Symbol,
    args: &[Value<'gc>],
) -> Option<Result<Value<'gc>, QuoinError>> {
    let lanes = receiver
        .with_native_state::<NativeServiceState, _, _>(|s| {
            (
                s.inbox_tx.clone(),
                s.outbox_rx.clone(),
                s.token_tx.clone(),
                s.token_rx.clone(),
                s.stopped.get(),
            )
        })
        .ok()?;
    Some(service_call(vm, mc, lanes, selector, args))
}

type Lanes = (
    async_channel::Sender<WorkerMsg>,
    async_channel::Receiver<WorkerMsg>,
    async_channel::Sender<WorkerMsg>,
    async_channel::Receiver<WorkerMsg>,
    bool,
);

fn service_call<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    (inbox_tx, outbox_rx, token_tx, token_rx, stopped): Lanes,
    selector: Symbol,
    args: &[Value<'gc>],
) -> Result<Value<'gc>, QuoinError> {
    if stopped {
        return Err(QuoinError::Other(format!(
            "service call '{}': the service is stopped",
            selector.as_str()
        )));
    }
    // Encode BEFORE taking the token: a refused argument shouldn't occupy
    // the service.
    let mut wire_args = Vec::with_capacity(args.len());
    for (i, a) in args.iter().enumerate() {
        wire_args.push(value_to_wire(*a, None).map_err(|e| {
            QuoinError::Other(format!(
                "service call '{}': argument {} is not portable: {e}",
                selector.as_str(),
                i + 1
            ))
        })?);
    }
    let frame = WireData::Map(vec![
        (
            "sel".to_string(),
            WireData::Str(selector.as_str().to_string()),
        ),
        ("args".to_string(), WireData::List(wire_args)),
    ]);

    // Serialize: take the token (parks fairly behind other callers).
    match vm.await_io(IoRequest::WorkerRecv(token_rx))? {
        IoResult::WorkerMsg(Some(_)) => {}
        _ => {
            return Err(QuoinError::Other(format!(
                "service call '{}': the service is stopped",
                selector.as_str()
            )));
        }
    }
    note_message();
    if inbox_tx.try_send(WorkerMsg::Data(frame)).is_err() {
        let _ = token_tx.try_send(WorkerMsg::Data(WireData::Null));
        return Err(QuoinError::Other(format!(
            "service call '{}': the service has exited",
            selector.as_str()
        )));
    }
    let reply = vm.await_io(IoRequest::WorkerRecv(outbox_rx))?;
    let _ = token_tx.try_send(WorkerMsg::Data(WireData::Null));
    let WireData::Map(pairs) = (match reply {
        IoResult::WorkerMsg(Some(WorkerMsg::Data(dv))) => dv,
        IoResult::WorkerMsg(_) => {
            return Err(QuoinError::Other(format!(
                "service call '{}': the service exited mid-call",
                selector.as_str()
            )));
        }
        other => {
            return Err(QuoinError::Other(format!(
                "service call '{}': unexpected result {other:?}",
                selector.as_str()
            )));
        }
    }) else {
        return Err(QuoinError::Other(format!(
            "service call '{}': malformed reply",
            selector.as_str()
        )));
    };
    for (k, v) in &pairs {
        match k.as_str() {
            "ret" => return wire_to_value(vm, mc, v, None),
            "err" => {
                if let WireData::Str(msg) = v {
                    return Err(QuoinError::Other(msg.clone()));
                }
            }
            _ => {}
        }
    }
    Err(QuoinError::Other(format!(
        "service call '{}': malformed reply",
        selector.as_str()
    )))
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
                crate::worker::spawn_worker_process(path.clone(), Some(class_name))
                    .map_err(QuoinError::Other)?;
            (ch, Some(pid))
        }
        _ => (spawn_worker_service(path.clone(), class_name), None),
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
    // Handshake: the loop's first act is a 'ready' message; a closed lane
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
            inbox_tx: ch.inbox_tx,
            outbox_rx: ch.outbox_rx,
            done_rx: ch.done_rx,
            token_tx,
            token_rx,
            stopped: std::cell::Cell::new(false),
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
             method sends become RPC over the worker lanes: sticky state with serialized \
             access -- an actor, effectively. Arguments and returns follow the worker data \
             taxonomy (plain data crosses; blocks and instances refuse), errors in the \
             hosted method raise catchably at the call site, and one call runs at a time \
             (concurrent callers queue fairly).\n\n\
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
             forwards as RPC and parks for the reply, so calls compose with `Async.gather:` \
             / `timeout:do:` like any parked wait.",
        )
        // Backing is a spawn-time choice by DESIGN (docs/CONCURRENCY_ARCH.md
        // §10): 'thread' is this v1; 'process' — the sanctioned escape from
        // the macOS cluster ceiling for compute-heavy services — is the
        // extension wire verbatim and lands separately. Reserve the surface,
        // refuse loudly.
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
        // Stop the service: waits for in-flight calls (takes the token),
        // sends the nil end-of-service message, and joins the worker.
        // Subsequent calls raise 'the service is stopped'.
        .instance_method("serviceStop", |vm, mc, receiver, _args| {
            let lanes = receiver
                .with_native_state::<NativeServiceState, _, _>(|s| {
                    s.stopped.set(true);
                    (s.inbox_tx.clone(), s.token_rx.clone(), s.done_rx.clone())
                })
                .map_err(QuoinError::Other)?;
            let (inbox_tx, token_rx, done_rx) = lanes;
            // Drain the token so in-flight calls finish first; ignore a
            // closed token lane (double stop).
            let _ = vm.await_io(IoRequest::WorkerRecv(token_rx))?;
            let _ = inbox_tx.try_send(WorkerMsg::Data(WireData::Null));
            match vm.await_io(IoRequest::WorkerJoin(done_rx))? {
                IoResult::WorkerDone(Ok(_)) => Ok(vm.new_nil(mc)),
                IoResult::WorkerDone(Err(msg)) => Err(QuoinError::Other(msg)),
                other => Err(QuoinError::Other(format!(
                    "serviceStop: unexpected result {other:?}"
                ))),
            }
        })
        .doc(
            "Stop the service: wait for in-flight calls to finish, send the end-of-service \
             message, and join the worker. Further calls raise 'the service is stopped'. \
             Answers nil.",
        )
}
