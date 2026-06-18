use crate::error::BBError;
use crate::value::{Block, Value};
use crate::vm::VmState;

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
