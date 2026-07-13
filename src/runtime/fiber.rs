use crate::error::QuoinError;
use crate::fiber::Fiber;
#[cfg(not(target_arch = "wasm32"))]
use crate::fiber::run_vm_loop;
#[cfg(not(target_arch = "wasm32"))]
use crate::gc;
use crate::value::{AnyCollect, NativeCall, NativeClassBuilder, Value};
use crate::vm::{AotTaskState, Frame};
use crate::vm_scheduler::TaskId;

use gc_arena::Gc;
use gc_arena::collect::{DynCollect, Trace};
use std::any::Any;
use std::fmt;
use std::mem::transmute;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FiberStatus {
    /// Constructed via `Fiber.new:` but never resumed.
    Created,
    /// Suspended at a `yield` (or as a resumer waiting for a child).
    Suspended,
    /// Currently executing, or an ancestor of the currently executing fiber.
    Running,
    /// The block returned normally; the fiber can no longer be resumed.
    Done,
    /// The block raised an uncaught error; the fiber can no longer be resumed.
    Failed,
}

impl FiberStatus {
    /// True once the fiber has terminated (whether normally or via an error).
    pub fn is_terminated(self) -> bool {
        matches!(self, FiberStatus::Done | FiberStatus::Failed)
    }
}

/// Native backing state for a guest `Fiber`.
///
/// Holds the fiber's own `corosensei` coroutine (its native stack) plus the
/// guest execution context (`stack`/`frames`/`native_args`) that is swapped into
/// and out of `VmState` by the scheduler. As with the other native states
/// (`NativeListState`, `NativeMethodState`), GC values are stored with their
/// lifetime erased to `'static` and laundered back via `transmute`, with a
/// hand-written `trace_gc` so the collector still sees them.
pub struct NativeFiberState {
    coro: Gc<'static, Fiber<'static>>,
    block: Value<'static>,
    pub status: FiberStatus,
    pub started: bool,
    stack: Vec<Value<'static>>,
    frames: Vec<Frame<'static>>,
    native_args: Vec<NativeCall<'static>>,
    /// The fiber's own AOT execution state (frame marks, enclosing env,
    /// fuel/depth), swapped with `vm.aot` alongside `stack`/`frames` so a
    /// compiled frame suspended across a `Fiber.yield` resumes with ITS
    /// marks — not the resumer's.
    aot: AotTaskState<'static>,
    /// Final return value once the fiber completes normally.
    result: Option<Value<'static>>,
    /// The error value once the fiber fails.
    error: Option<Value<'static>>,
    /// This coroutine's `Yielder`, stored as a raw address. The scheduler loads
    /// it into `VmState.yielder` before resuming this fiber. Not GC data.
    yielder: Option<usize>,
    /// The task this fiber is currently live inside — as its current fiber or an
    /// ancestor on its resume chain — set at `do_resume_switch`, cleared when the
    /// fiber yields or completes. While set, the fiber's real execution context is
    /// live in (or stashed with) that task, not in this state's `stack`/`frames`,
    /// so resuming it from any other task must be refused (`fiber_resume`): it
    /// would load an empty context and re-enter the coroutine at a foreign suspend
    /// point, corrupting both tasks and ultimately aborting the process. Plain
    /// data, not GC state.
    pub owner: Option<TaskId>,
}

impl NativeFiberState {
    pub fn new<'gc>(coro: Gc<'gc, Fiber<'gc>>, block: Value<'gc>) -> Self {
        Self {
            coro: unsafe { transmute::<Gc<'gc, Fiber<'gc>>, Gc<'static, Fiber<'static>>>(coro) },
            block: unsafe { transmute::<Value<'gc>, Value<'static>>(block) },
            status: FiberStatus::Created,
            started: false,
            stack: Vec::new(),
            frames: Vec::new(),
            native_args: Vec::new(),
            aot: AotTaskState::default(),
            result: None,
            error: None,
            yielder: None,
            owner: None,
        }
    }

    pub fn set_yielder(&mut self, ptr: *const ()) {
        self.yielder = Some(ptr as usize);
    }

    pub fn yielder(&self) -> Option<*const ()> {
        self.yielder.map(|u| u as *const ())
    }

    pub fn set_result<'gc>(&mut self, val: Value<'gc>) {
        self.result = Some(unsafe { transmute::<Value<'gc>, Value<'static>>(val) });
    }

    pub fn result<'gc>(&self) -> Option<Value<'gc>> {
        self.result
            .map(|v| unsafe { transmute::<Value<'static>, Value<'gc>>(v) })
    }

    pub fn set_error<'gc>(&mut self, val: Value<'gc>) {
        self.error = Some(unsafe { transmute::<Value<'gc>, Value<'static>>(val) });
    }

    pub fn error<'gc>(&self) -> Option<Value<'gc>> {
        self.error
            .map(|v| unsafe { transmute::<Value<'static>, Value<'gc>>(v) })
    }

    pub fn coro<'gc>(&self) -> Gc<'gc, Fiber<'gc>> {
        unsafe { transmute::<Gc<'static, Fiber<'static>>, Gc<'gc, Fiber<'gc>>>(self.coro) }
    }

    pub fn block<'gc>(&self) -> Value<'gc> {
        unsafe { transmute::<Value<'static>, Value<'gc>>(self.block) }
    }

    /// Move the saved context out, leaving the slots empty.
    #[allow(clippy::type_complexity)]
    pub fn take_context<'gc>(
        &mut self,
    ) -> (
        Vec<Value<'gc>>,
        Vec<Frame<'gc>>,
        Vec<NativeCall<'gc>>,
        AotTaskState<'gc>,
    ) {
        unsafe {
            (
                transmute::<Vec<Value<'static>>, Vec<Value<'gc>>>(std::mem::take(&mut self.stack)),
                transmute::<Vec<Frame<'static>>, Vec<Frame<'gc>>>(std::mem::take(&mut self.frames)),
                transmute::<Vec<NativeCall<'static>>, Vec<NativeCall<'gc>>>(std::mem::take(
                    &mut self.native_args,
                )),
                transmute::<AotTaskState<'static>, AotTaskState<'gc>>(std::mem::take(
                    &mut self.aot,
                )),
            )
        }
    }

    /// Store a context, overwriting whatever was saved.
    pub fn set_context<'gc>(
        &mut self,
        stack: Vec<Value<'gc>>,
        frames: Vec<Frame<'gc>>,
        native_args: Vec<NativeCall<'gc>>,
        aot: AotTaskState<'gc>,
    ) {
        unsafe {
            self.stack = transmute::<Vec<Value<'gc>>, Vec<Value<'static>>>(stack);
            self.frames = transmute::<Vec<Frame<'gc>>, Vec<Frame<'static>>>(frames);
            self.native_args =
                transmute::<Vec<NativeCall<'gc>>, Vec<NativeCall<'static>>>(native_args);
            self.aot = transmute::<AotTaskState<'gc>, AotTaskState<'static>>(aot);
        }
    }
}

impl fmt::Debug for NativeFiberState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "NativeFiberState{{status:{:?} started:{}}}",
            self.status, self.started
        )
    }
}

impl AnyCollect for NativeFiberState {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn trace_gc<'gc>(&self, cc: &mut dyn Trace<'gc>) {
        let coro_gc: &Gc<'gc, Fiber<'gc>> = unsafe { transmute(&self.coro) };
        coro_gc.dyn_trace(cc);
        let block_gc: &Value<'gc> = unsafe { transmute(&self.block) };
        block_gc.dyn_trace(cc);
        for val in &self.stack {
            let val_gc: &Value<'gc> = unsafe { transmute(val) };
            val_gc.dyn_trace(cc);
        }
        for frame in &self.frames {
            let frame_gc: &Frame<'gc> = unsafe { transmute(frame) };
            frame_gc.dyn_trace(cc);
        }
        // The saved AOT slice holds one Gc field (`enclosing_env`); the rest
        // is require_static. Trace the whole struct like a Frame.
        let aot_gc: &AotTaskState<'gc> = unsafe { transmute(&self.aot) };
        aot_gc.dyn_trace(cc);
        for call in &self.native_args {
            let recv_gc: &Value<'gc> = unsafe { transmute(&call.receiver) };
            recv_gc.dyn_trace(cc);
            // A StackWindow variant holds only indices — its values live in
            // (and are traced via) the owning stack Vec.
            if let crate::value::NativeArgs::Owned(vals) = &call.args {
                for val in vals {
                    let val_gc: &Value<'gc> = unsafe { transmute(val) };
                    val_gc.dyn_trace(cc);
                }
            }
        }
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

fn status_of(fiber: Value<'_>) -> Result<FiberStatus, QuoinError> {
    fiber
        .with_native_state::<NativeFiberState, _, _>(|s| s.status)
        .map_err(QuoinError::Other)
}

pub fn build_fiber_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Fiber", Some("Object"))
        .construct_with("use Fiber.new:{ … }")
        .class_doc(
            "A coroutine: a block that can suspend itself with `Fiber.yield:` -- or the \
             `^> value` sugar -- and be resumed by its caller, exchanging values both ways. \
             Cooperative and in-task; for concurrent tasks whose I/O overlaps, use Task / \
             Async instead.\n\n\
             ```\n\
             var f = Fiber.new:{ ^> 1; 2 };\n\
             #( f.resume f.resume f.status )    \"* -> #(1 2 done)\n\
             ```",
        )
        // Fiber.new:aBlock -> a fresh, unstarted fiber wrapping the block.
        .class_method("new:", |vm, mc, _receiver, args| {
            let block_val = args[0];
            match block_val {
                Value::Object(obj)
                    if matches!(obj.borrow().payload, crate::value::ObjectPayload::Block(_)) => {}
                _ => {
                    return Err(QuoinError::TypeError {
                        expected: "Block".to_string(),
                        got: block_val.type_name().to_string(),
                        msg: "Fiber.new: expects a Block".to_string(),
                    });
                }
            }
            // No stack switching on wasm32: a guest fiber could be built (as a husk)
            // but never resumed, so refuse up front with a catchable error instead.
            #[cfg(target_arch = "wasm32")]
            {
                let _ = (vm, mc);
                Err(QuoinError::Other(
                    "Fiber is not supported on this platform".to_string(),
                ))
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                let coro = Fiber::new(|yielder, ctx| run_vm_loop(yielder, ctx));
                let coro_gc = gc!(mc, coro);
                let state = NativeFiberState::new(coro_gc, block_val);
                let class = vm.get_builtin_class("Fiber");
                Ok(vm.new_native_state(mc, class, state))
            }
        })
        .doc(
            "A fresh, unstarted fiber wrapping the zero-parameter block (status 'created'). \
             Nothing runs until the first `resume`.",
        )
        // Fiber.yield:value / Fiber.yield -> suspend the running fiber.
        .class_method("yield:", |vm, mc, _receiver, args| {
            vm.fiber_yield(mc, args[0])
        })
        .doc(
            "Suspend the running fiber, delivering the value as the pending `resume`'s \
             result; the value a later `resume:` passes becomes this expression's value. \
             `^> value` is the yield sugar.\n\n\
             ```\n\
             var f = Fiber.new:{ var got = ^> 1; got * 10 };\n\
             f.resume;     \"* -> 1\n\
             f.resume:5    \"* -> 50\n\
             ```",
        )
        .class_method("yield", |vm, mc, _receiver, _args| {
            let nil = vm.new_nil(mc);
            vm.fiber_yield(mc, nil)
        })
        .doc("As `yield:` with nil.")
        // Fiber.current -> the running fiber, or nil from the main program.
        .class_method("current", |vm, mc, _receiver, _args| {
            Ok(vm.sched.current_fiber.unwrap_or_else(|| vm.new_nil(mc)))
        })
        .doc("The currently running fiber, or nil outside any fiber.")
        // f.resume / f.resume:value -> run until the next yield or completion.
        .instance_method("resume", |vm, mc, receiver, _args| {
            let nil = vm.new_nil(mc);
            vm.fiber_resume(mc, receiver, nil)
        })
        .doc(
            "Run the fiber until its next yield or completion; answers the yielded (or \
             final) value. If the fiber's block raised, the error re-raises here. Resuming \
             a finished fiber -- or one live in another task -- raises a FiberError.",
        )
        .instance_method("resume:", |vm, mc, receiver, args| {
            vm.fiber_resume(mc, receiver, args[0])
        })
        .doc(
            "As `resume`, delivering the value as the result of the `yield` expression the \
             fiber is suspended at.",
        )
        .instance_method("done?", |vm, mc, receiver, _args| {
            Ok(vm.new_bool(mc, status_of(receiver)? == FiberStatus::Done))
        })
        .doc("True once the fiber's block returned normally.")
        .instance_method("failed?", |vm, mc, receiver, _args| {
            Ok(vm.new_bool(mc, status_of(receiver)? == FiberStatus::Failed))
        })
        .doc(
            "True once the fiber's block raised an uncaught error (the error re-raised at \
             the `resume` that observed it; `error` keeps the value).",
        )
        .instance_method("alive?", |vm, mc, receiver, _args| {
            Ok(vm.new_bool(mc, !status_of(receiver)?.is_terminated()))
        })
        .doc("True while the fiber can still be resumed (not yet done or failed).")
        .instance_method("status", |vm, mc, receiver, _args| {
            let name = match status_of(receiver)? {
                FiberStatus::Created => "created",
                FiberStatus::Suspended => "suspended",
                FiberStatus::Running => "running",
                FiberStatus::Done => "done",
                FiberStatus::Failed => "failed",
            };
            Ok(vm.new_string(mc, name.to_string()))
        })
        .doc(
            "One of 'created', 'suspended', 'running', 'done', 'failed'.\n\n\
             ```\n\
             (Fiber.new:{ }).status    \"* -> created\n\
             ```",
        )
        // The fiber's final return value (nil unless it completed normally).
        .instance_method("result", |vm, mc, receiver, _args| {
            let r = receiver
                .with_native_state::<NativeFiberState, _, _>(|s| s.result())
                .map_err(QuoinError::Other)?;
            Ok(r.unwrap_or_else(|| vm.new_nil(mc)))
        })
        .doc("The fiber's final return value; nil unless it completed normally.")
        // The error value if the fiber failed (nil otherwise).
        .instance_method("error", |vm, mc, receiver, _args| {
            let e = receiver
                .with_native_state::<NativeFiberState, _, _>(|s| s.error())
                .map_err(QuoinError::Other)?;
            Ok(e.unwrap_or_else(|| vm.new_nil(mc)))
        })
        .doc("The error value if the fiber failed; nil otherwise.")
}
