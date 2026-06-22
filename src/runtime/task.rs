use crate::error::QuoinError;
use crate::value::{AnyCollect, NativeClassBuilder, ObjectPayload, Value};
use crate::vm::TaskId;

use gc_arena::collect::{DynCollect, Trace};
use std::any::Any;
use std::fmt;
use std::mem::transmute;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TaskStatus {
    /// Spawned and still running or parked.
    Running,
    /// The block returned normally; `result` holds its value.
    Done,
    /// The block raised an uncaught exception; `error` holds the exception value.
    Failed,
    /// Cancelled via `handle.cancel` (Stage 2b-ii).
    Cancelled,
}

/// Native backing state for a `Task` handle (the object `Task.spawn:` returns).
///
/// Holds the detached task's id plus its *outcome*. While the task runs, the
/// scheduler's `Task` slot roots this handle (via `Task::handle`); on completion
/// `complete_detached` writes the outcome here and frees the slot, after which the
/// handle — and its result — live by normal QN reachability. The `TaskId` is only
/// ever dereferenced while `status == Running`, so a freed/reused slot is never
/// touched through a finished handle. GC values are stored `'static` and laundered
/// back via `transmute`, with a hand-written `trace_gc` (as the other native states).
pub struct NativeTaskHandle {
    id: TaskId,
    status: TaskStatus,
    result: Option<Value<'static>>,
    error: Option<Value<'static>>,
}

impl NativeTaskHandle {
    pub fn new(id: TaskId) -> Self {
        Self {
            id,
            status: TaskStatus::Running,
            result: None,
            error: None,
        }
    }

    pub fn id(&self) -> TaskId {
        self.id
    }

    pub fn status(&self) -> TaskStatus {
        self.status
    }

    pub fn set_done<'gc>(&mut self, value: Value<'gc>) {
        self.status = TaskStatus::Done;
        self.result = Some(unsafe { transmute::<Value<'gc>, Value<'static>>(value) });
    }

    pub fn set_failed<'gc>(&mut self, error: Value<'gc>) {
        self.status = TaskStatus::Failed;
        self.error = Some(unsafe { transmute::<Value<'gc>, Value<'static>>(error) });
    }

    pub fn result<'gc>(&self) -> Option<Value<'gc>> {
        self.result
            .map(|v| unsafe { transmute::<Value<'static>, Value<'gc>>(v) })
    }

    pub fn error<'gc>(&self) -> Option<Value<'gc>> {
        self.error
            .map(|v| unsafe { transmute::<Value<'static>, Value<'gc>>(v) })
    }
}

impl fmt::Debug for NativeTaskHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "NativeTaskHandle{{id:{} status:{:?}}}",
            self.id.0, self.status
        )
    }
}

impl AnyCollect for NativeTaskHandle {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn trace_gc<'gc>(&self, cc: &mut dyn Trace<'gc>) {
        if let Some(v) = &self.result {
            let v_gc: &Value<'gc> = unsafe { transmute(v) };
            v_gc.dyn_trace(cc);
        }
        if let Some(v) = &self.error {
            let v_gc: &Value<'gc> = unsafe { transmute(v) };
            v_gc.dyn_trace(cc);
        }
    }
}

/// Read `(status, id)` off a `Task` handle receiver.
fn handle_state(receiver: Value<'_>) -> Result<(TaskStatus, TaskId), QuoinError> {
    receiver
        .with_native_state::<NativeTaskHandle, _, _>(|h| (h.status(), h.id()))
        .map_err(QuoinError::Other)
}

pub fn build_task_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Task", Some("Object"))
        // Task.spawn:aBlock -> spawn a detached task running the block; returns a
        // handle. The spawner keeps running (fire-and-forget). See docs/ASYNC_ARCH.md.
        .class_method("spawn:", |vm, _mc, _receiver, args| {
            let block_gc = match args.first() {
                Some(Value::Object(obj)) => match &obj.borrow().payload {
                    ObjectPayload::Block(b) => *b,
                    _ => return Err(spawn_type_error(args.first())),
                },
                _ => return Err(spawn_type_error(args.first())),
            };
            Ok(vm.spawn_detached(_mc, block_gc))
        })
        // Task.running -> a snapshot list of the handles of all still-running detached
        // tasks. The basis for a user-written structured join-all.
        .class_method("running", |vm, mc, _receiver, _args| {
            let handles: Vec<Value> = vm
                .sched
                .tasks
                .iter()
                .filter_map(|t| t.as_ref().and_then(|t| t.handle))
                .collect();
            Ok(vm.new_list(mc, handles))
        })
        // handle.join -> the task's result, re-raising its exception if it failed (a
        // catchable throw). Parks the caller if the task is still running.
        .instance_method("join", |vm, mc, receiver, _args| {
            let (status, id) = handle_state(receiver)?;
            match status {
                TaskStatus::Running => {
                    if id == vm.sched.current_task {
                        return Err(QuoinError::Other("a task cannot join itself".to_string()));
                    }
                    vm.await_join(id)
                }
                TaskStatus::Done => {
                    let r = receiver
                        .with_native_state::<NativeTaskHandle, _, _>(|h| h.result())
                        .map_err(QuoinError::Other)?;
                    Ok(r.unwrap_or_else(|| vm.new_nil(mc)))
                }
                TaskStatus::Failed => {
                    let e = receiver
                        .with_native_state::<NativeTaskHandle, _, _>(|h| h.error())
                        .map_err(QuoinError::Other)?;
                    vm.active_exception = e;
                    Err(QuoinError::Thrown)
                }
                TaskStatus::Cancelled => Err(QuoinError::Other(
                    "join of a cancelled task is not yet supported".to_string(),
                )),
            }
        })
        // handle.status -> running | done | failed | cancelled
        .instance_method("status", |vm, mc, receiver, _args| {
            let name = match handle_state(receiver)?.0 {
                TaskStatus::Running => "running",
                TaskStatus::Done => "done",
                TaskStatus::Failed => "failed",
                TaskStatus::Cancelled => "cancelled",
            };
            Ok(vm.new_string(mc, name.to_string()))
        })
        .instance_method("done?", |vm, mc, receiver, _args| {
            Ok(vm.new_bool(mc, handle_state(receiver)?.0 == TaskStatus::Done))
        })
}

fn spawn_type_error(got: Option<&Value>) -> QuoinError {
    QuoinError::TypeError {
        expected: "Block".to_string(),
        got: got
            .map(|v| v.type_name().to_string())
            .unwrap_or_else(|| "None".to_string()),
        msg: "Task.spawn: expects a Block".to_string(),
    }
}
