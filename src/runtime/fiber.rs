use crate::error::QuoinError;
use crate::fiber::{run_vm_loop, Fiber};
use crate::gc;
use crate::value::{AnyCollect, NativeClassBuilder, Value};
use crate::vm::Frame;

use gc_arena::collect::{DynCollect, Trace};
use gc_arena::Gc;
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
    native_args: Vec<Vec<Value<'static>>>,
    /// Final return value once the fiber completes normally.
    result: Option<Value<'static>>,
    /// The error value once the fiber fails.
    error: Option<Value<'static>>,
    /// This coroutine's `Yielder`, stored as a raw address. The scheduler loads
    /// it into `VmState.yielder` before resuming this fiber. Not GC data.
    yielder: Option<usize>,
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
            result: None,
            error: None,
            yielder: None,
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
    pub fn take_context<'gc>(
        &mut self,
    ) -> (Vec<Value<'gc>>, Vec<Frame<'gc>>, Vec<Vec<Value<'gc>>>) {
        unsafe {
            (
                transmute::<Vec<Value<'static>>, Vec<Value<'gc>>>(std::mem::take(&mut self.stack)),
                transmute::<Vec<Frame<'static>>, Vec<Frame<'gc>>>(std::mem::take(&mut self.frames)),
                transmute::<Vec<Vec<Value<'static>>>, Vec<Vec<Value<'gc>>>>(std::mem::take(
                    &mut self.native_args,
                )),
            )
        }
    }

    /// Store a context, overwriting whatever was saved.
    pub fn set_context<'gc>(
        &mut self,
        stack: Vec<Value<'gc>>,
        frames: Vec<Frame<'gc>>,
        native_args: Vec<Vec<Value<'gc>>>,
    ) {
        unsafe {
            self.stack = transmute::<Vec<Value<'gc>>, Vec<Value<'static>>>(stack);
            self.frames = transmute::<Vec<Frame<'gc>>, Vec<Frame<'static>>>(frames);
            self.native_args =
                transmute::<Vec<Vec<Value<'gc>>>, Vec<Vec<Value<'static>>>>(native_args);
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
        for inner in &self.native_args {
            for val in inner {
                let val_gc: &Value<'gc> = unsafe { transmute(val) };
                val_gc.dyn_trace(cc);
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
        // Fiber.new:aBlock -> a fresh, unstarted fiber wrapping the block.
        .class_method("new:", |vm, mc, args| {
            let block_val = args[1];
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
            let coro = Fiber::new(|yielder, ctx| run_vm_loop(yielder, ctx));
            let coro_gc = gc!(mc, coro);
            let state = NativeFiberState::new(coro_gc, block_val);
            let class = vm.get_builtin_class("Fiber");
            Ok(vm.new_native_state(mc, class, state))
        })
        // Fiber.yield:value / Fiber.yield -> suspend the running fiber.
        .class_method("yield:", |vm, mc, args| vm.fiber_yield(mc, args[1]))
        .class_method("yield", |vm, mc, _args| {
            let nil = vm.new_nil(mc);
            vm.fiber_yield(mc, nil)
        })
        // Fiber.current -> the running fiber, or nil from the main program.
        .class_method("current", |vm, mc, _args| {
            Ok(vm.current_fiber.unwrap_or_else(|| vm.new_nil(mc)))
        })
        // f.resume / f.resume:value -> run until the next yield or completion.
        .instance_method("resume", |vm, mc, args| {
            let nil = vm.new_nil(mc);
            vm.fiber_resume(mc, args[0], nil)
        })
        .instance_method("resume:", |vm, mc, args| {
            vm.fiber_resume(mc, args[0], args[1])
        })
        .instance_method("done?", |vm, mc, args| {
            Ok(vm.new_bool(mc, status_of(args[0])? == FiberStatus::Done))
        })
        .instance_method("failed?", |vm, mc, args| {
            Ok(vm.new_bool(mc, status_of(args[0])? == FiberStatus::Failed))
        })
        .instance_method("alive?", |vm, mc, args| {
            Ok(vm.new_bool(mc, !status_of(args[0])?.is_terminated()))
        })
        .instance_method("status", |vm, mc, args| {
            let name = match status_of(args[0])? {
                FiberStatus::Created => "created",
                FiberStatus::Suspended => "suspended",
                FiberStatus::Running => "running",
                FiberStatus::Done => "done",
                FiberStatus::Failed => "failed",
            };
            Ok(vm.new_string(mc, name.to_string()))
        })
        // The fiber's final return value (nil unless it completed normally).
        .instance_method("result", |vm, mc, args| {
            let r = args[0]
                .with_native_state::<NativeFiberState, _, _>(|s| s.result())
                .map_err(QuoinError::Other)?;
            Ok(r.unwrap_or_else(|| vm.new_nil(mc)))
        })
        // The error value if the fiber failed (nil otherwise).
        .instance_method("error", |vm, mc, args| {
            let e = args[0]
                .with_native_state::<NativeFiberState, _, _>(|s| s.error())
                .map_err(QuoinError::Other)?;
            Ok(e.unwrap_or_else(|| vm.new_nil(mc)))
        })
}
