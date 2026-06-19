use crate::error::BBError;
use crate::value::{Block, Value};
use crate::vm::{VmState, VmStatus};

use corosensei::stack::DefaultStack;
use gc_arena::{Collect, Gc, Mutation};
use std::cell::RefCell;

pub type VMCoroutine<'gc> = corosensei::ScopedCoroutine<
    'gc,
    VMContext<'gc>,
    YieldReason<'gc>,
    Result<Value<'gc>, BBError>,
    DefaultStack,
>;
pub type VMYielder<'gc> = corosensei::Yielder<VMContext<'gc>, YieldReason<'gc>>;

#[derive(Collect)]
#[collect(no_drop)]
pub enum YieldReason<'gc> {
    CallBlock {
        block: Gc<'gc, Block<'gc>>,
        args: Vec<Value<'gc>>,
    },
    CooperativeYield,
    Return(Value<'gc>),
    /// A guest fiber is resuming another guest fiber. Bubbles to the scheduler,
    /// which switches execution contexts to `fiber`, delivering `arg`.
    ResumeFiber {
        fiber: Value<'gc>,
        arg: Value<'gc>,
    },
    /// The running guest fiber is suspending and handing `value` back to whoever
    /// resumed it. Bubbles to the scheduler.
    YieldFiber {
        value: Value<'gc>,
    },
}

/// The standard VM driver loop, shared by the main program and every guest
/// `Fiber`. Each runs as its own `corosensei` coroutine; this body just steps
/// the VM over the *current* execution context and cooperatively suspends so
/// the scheduler can run the GC. Fiber resume/yield happen deeper in `step`
/// (inside the native `Fiber` methods) and bubble up as `YieldReason`s, so they
/// are transparent here.
pub fn run_vm_loop<'gc>(
    yielder: &VMYielder<'gc>,
    mut ctx: VMContext<'gc>,
) -> Result<Value<'gc>, BBError> {
    let (vm, _mc) = unsafe { ctx.get() };
    vm.yielder = Some(yielder as *const _ as *const ());

    loop {
        let (vm, mc) = unsafe { ctx.get() };
        match vm.step(mc) {
            Ok(VmStatus::Running) => {
                vm.yielder = None;
                ctx = yielder.suspend(YieldReason::CooperativeYield);
                let (vm, _mc) = unsafe { ctx.get() };
                vm.yielder = Some(yielder as *const _ as *const ());
            }
            Ok(VmStatus::Finished(val)) => {
                vm.yielder = None;
                return Ok(val);
            }
            Ok(VmStatus::Yeeted(val)) => {
                vm.yielder = None;
                return Err(BBError::Other(format!("Uncaught exception: {}", val)));
            }
            Err(err) => {
                vm.yielder = None;
                return Err(err);
            }
        }
    }
}

/// A wrapper around raw pointers to VMState and Mutation contexts.
/// This allows passing execution contexts into and out of coroutines
/// without lifetime conflicts.
pub struct VMContext<'gc> {
    pub vm: *mut VmState<'gc>,
    pub mc: *const Mutation<'gc>,
}

impl<'gc> VMContext<'gc> {
    /// # Safety
    /// The caller must ensure that the pointers are valid and that no other
    /// borrows of the VM state or mutation context exist during the call.
    pub unsafe fn get(&self) -> (&mut VmState<'gc>, &Mutation<'gc>) {
        unsafe { (&mut *self.vm, &*self.mc) }
    }
}

pub struct Fiber<'gc> {
    pub coroutine: RefCell<Option<VMCoroutine<'gc>>>,
}

unsafe impl<'gc> Collect<'gc> for Fiber<'gc> {
    const NEEDS_TRACE: bool = false;
}

impl<'gc> Fiber<'gc> {
    pub fn new<F>(f: F) -> Self
    where
        F: FnOnce(&VMYielder<'gc>, VMContext<'gc>) -> Result<Value<'gc>, BBError> + 'gc,
    {
        let coroutine = VMCoroutine::new(move |yielder, ctx| f(yielder, ctx));
        Self {
            coroutine: RefCell::new(Some(coroutine)),
        }
    }
}
