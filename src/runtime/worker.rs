//! The `Worker` class — C2 v1 isolates (docs/CONCURRENCY_ARCH.md §5).
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
    WorkerMsg, note_message, rebuild_portable_value, snapshot_block, spawn_worker,
    spawn_worker_block,
};

/// Parent-side handle state: the three channel ends. Plain `Send` data —
/// nothing here touches the arena.
#[derive(Debug)]
pub struct NativeWorkerHandle {
    inbox_tx: async_channel::Sender<WorkerMsg>,
    outbox_rx: async_channel::Receiver<WorkerMsg>,
    done_rx: async_channel::Receiver<Result<WireData, String>>,
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

/// Copy a guest value into a cross-worker message. A BLOCK value ships as
/// a portable block (template + capture snapshot — same rules as
/// `Worker.start:`); everything else takes the wire walkers, whose
/// taxonomy still refuses symbols/instances/resources — and blocks nested
/// INSIDE data structures.
fn to_message<'gc>(v: Value<'gc>) -> Result<WorkerMsg, QuoinError> {
    if let Value::Object(obj) = v {
        let block_parts = {
            let borrowed = obj.borrow();
            if let ObjectPayload::Block(b) = &borrowed.payload {
                Some((b.template.clone(), b.parent_env))
            } else {
                None
            }
        };
        if let Some((template, parent_env)) = block_parts {
            let pb = snapshot_block(template, parent_env, 0)?;
            note_message();
            return Ok(WorkerMsg::Block(pb));
        }
    }
    let dv = value_to_wire(v, None)?;
    note_message();
    Ok(WorkerMsg::Data(dv))
}

/// Decode a received cross-worker message into a live value: data through
/// the wire walkers, blocks rebuilt over their capture snapshots.
fn from_message<'gc>(
    vm: &mut crate::vm::VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    msg: &WorkerMsg,
) -> Result<Value<'gc>, QuoinError> {
    match msg {
        WorkerMsg::Data(dv) => wire_to_value(vm, mc, dv, None),
        WorkerMsg::Block(pb) => rebuild_portable_value(vm, mc, pb).map_err(QuoinError::Other),
    }
}

/// Wrap freshly spawned lanes in a Worker-class handle instance.
fn wrap_handle<'gc>(
    vm: &mut crate::vm::VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    receiver: Value<'gc>,
    ch: crate::worker::WorkerChannels,
) -> Result<Value<'gc>, QuoinError> {
    let Value::Class(class) = receiver else {
        return Err(QuoinError::Other("Worker: bad receiver".into()));
    };
    Ok(vm.new_native_state(
        mc,
        class,
        NativeWorkerHandle {
            inbox_tx: ch.inbox_tx,
            outbox_rx: ch.outbox_rx,
            done_rx: ch.done_rx,
            joined: std::cell::Cell::new(false),
        },
    ))
}

pub fn build_worker_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Worker", Some("Object"))
        // ---- parent side (class-side spawn, instance-side lanes) ----
        .class_method("spawn:", |vm, mc, receiver, args| {
            let path = args[0]
                .as_string()
                .ok_or_else(|| QuoinError::Other("Worker.spawn: expects a String path".into()))?;
            wrap_handle(vm, mc, receiver, spawn_worker(path))
        })
        // Portable blocks (docs/CONCURRENCY_ARCH.md §10): ship the block's
        // template by reference plus a deep-copied SNAPSHOT of its free
        // reads; join returns the block's value. The portability scan
        // refuses write-captures, ^^, self/@fields, guarded blocks, and
        // class/method definition — loudly, at submit time.
        .class_method("start:", |vm, mc, receiver, args| {
            let Value::Object(obj) = args[0] else {
                return Err(QuoinError::Other("Worker.start: expects a Block".into()));
            };
            let (template, parent_env) = {
                let borrowed = obj.borrow();
                match &borrowed.payload {
                    ObjectPayload::Block(b) => (b.template.clone(), b.parent_env),
                    _ => {
                        return Err(QuoinError::Other("Worker.start: expects a Block".into()));
                    }
                }
            };
            if !template.param_syms.is_empty() {
                return Err(QuoinError::Other(
                    "Worker.start: the block takes no parameters (send it data through \
                     the lanes instead)"
                        .into(),
                ));
            }
            let pb = snapshot_block(template, parent_env, 0)
                .map_err(|e| QuoinError::Other(format!("Worker.start: {e}")))?;
            wrap_handle(vm, mc, receiver, spawn_worker_block(pb))
        })
        .instance_method("send:", |vm, _mc, receiver, args| {
            let dv = to_message(args[0])?;
            let tx = receiver
                .with_native_state::<NativeWorkerHandle, _, _>(|h| h.inbox_tx.clone())
                .map_err(QuoinError::Other)?;
            tx.try_send(dv)
                .map_err(|_| QuoinError::Other("Worker.send: the worker has exited".into()))?;
            Ok(vm.new_nil(_mc))
        })
        .instance_method("receive", |vm, mc, receiver, _args| {
            let rx = receiver
                .with_native_state::<NativeWorkerHandle, _, _>(|h| h.outbox_rx.clone())
                .map_err(QuoinError::Other)?;
            match vm.await_io(IoRequest::WorkerRecv(rx))? {
                IoResult::WorkerMsg(Some(msg)) => from_message(vm, mc, &msg),
                IoResult::WorkerMsg(None) => Ok(vm.new_nil(mc)),
                other => Err(QuoinError::Other(format!(
                    "Worker.receive: unexpected result {other:?}"
                ))),
            }
        })
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
                IoResult::WorkerDone(Err(msg)) => Err(QuoinError::Other(msg)),
                other => Err(QuoinError::Other(format!(
                    "Worker.join: unexpected result {other:?}"
                ))),
            }
        })
        // ---- worker side (class-side lanes, live only inside a worker) ----
        .class_method("receive", |vm, mc, _receiver, _args| {
            let Some(link) = vm.worker_link.as_ref() else {
                return Err(QuoinError::Other(
                    "Worker.receive: not inside a worker (spawn one with Worker.spawn:)".into(),
                ));
            };
            let rx = link.inbox_rx.clone();
            match vm.await_io(IoRequest::WorkerRecv(rx))? {
                IoResult::WorkerMsg(Some(msg)) => from_message(vm, mc, &msg),
                IoResult::WorkerMsg(None) => Ok(vm.new_nil(mc)),
                other => Err(QuoinError::Other(format!(
                    "Worker.receive: unexpected result {other:?}"
                ))),
            }
        })
        .class_method("send:", |vm, mc, _receiver, args| {
            let Some(link) = vm.worker_link.as_ref() else {
                return Err(QuoinError::Other("Worker.send: not inside a worker".into()));
            };
            let tx = link.outbox_tx.clone();
            let dv = to_message(args[0])?;
            tx.try_send(dv)
                .map_err(|_| QuoinError::Other("Worker.send: the parent has gone away".into()))?;
            Ok(vm.new_nil(mc))
        })
        .class_method("worker?", |vm, mc, _receiver, _args| {
            Ok(vm.new_bool(mc, vm.worker_link.is_some()))
        })
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
