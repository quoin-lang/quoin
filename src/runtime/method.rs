use crate::error::QuoinError;
use crate::value::{AnyCollect, NativeClassBuilder, NativeFunc, ObjectPayload, Value};

use gc_arena::collect::{DynCollect, Trace};
use std::any::Any;
use std::mem::transmute;

/// The body of a method variant: a user-defined `Block`, or a native Rust fn.
/// Unifying these into one chainable node lets native and user methods share the
/// same multimethod dispatch. (Phase 2b will let the `Native` variant also carry a
/// type signature; until then a native method scores as an untyped fallback.)
#[derive(Debug)]
pub enum MethodBody {
    UserBlock(Value<'static>),
    Native {
        func: NativeFunc,
        /// `None` = untyped/legacy native method (scored as a fallback); `Some` =
        /// scored by these declared parameter types like a user method.
        param_types: Option<Vec<String>>,
    },
}

#[derive(Debug)]
pub struct NativeMethodState {
    pub selector: String,
    pub body: MethodBody,
    pub is_extension: bool,
    pub next: Option<Value<'static>>,
}

impl NativeMethodState {
    /// A user-defined method variant wrapping `block`.
    pub fn new(selector: String, block: Value<'_>, is_extension: bool) -> Self {
        let block_static: Value<'static> = unsafe { transmute(block) };
        Self {
            selector,
            body: MethodBody::UserBlock(block_static),
            is_extension,
            next: None,
        }
    }

    /// A native method variant. Chainable and dispatchable like a user method.
    pub fn new_native(
        selector: String,
        func: NativeFunc,
        param_types: Option<Vec<String>>,
    ) -> Self {
        Self {
            selector,
            body: MethodBody::Native { func, param_types },
            is_extension: false,
            next: None,
        }
    }

    /// The wrapped user `Block` value, or `None` for a native method body.
    pub fn get_block<'gc>(&self) -> Option<Value<'gc>> {
        match &self.body {
            MethodBody::UserBlock(block) => Some(unsafe { transmute(*block) }),
            MethodBody::Native { .. } => None,
        }
    }

    /// The native function, or `None` for a user block body.
    pub fn native_func(&self) -> Option<NativeFunc> {
        match &self.body {
            MethodBody::Native { func, .. } => Some(*func),
            MethodBody::UserBlock(_) => None,
        }
    }

    /// Declared parameter types for a native method, or `None` for a user block or
    /// an untyped (legacy) native method — both of which the scorer handles
    /// elsewhere (a user block scores from its `Block`, an untyped native is a
    /// fallback).
    pub fn native_param_types(&self) -> Option<Vec<String>> {
        match &self.body {
            MethodBody::Native { param_types, .. } => param_types.clone(),
            MethodBody::UserBlock(_) => None,
        }
    }
}

impl AnyCollect for NativeMethodState {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn trace_gc<'gc>(&self, cc: &mut dyn Trace<'gc>) {
        if let MethodBody::UserBlock(block) = &self.body {
            let block_gc: &Value<'gc> = unsafe { transmute(block) };
            block_gc.dyn_trace(cc);
        }
        // MethodBody::Native holds only a fn pointer — nothing to trace.
        if let Some(next) = &self.next {
            let next_gc: &Value<'gc> = unsafe { transmute(next) };
            next_gc.dyn_trace(cc);
        }
    }
}

pub fn build_method_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Method", Some("Object"))
        .instance_method("selector", |vm, mc, receiver, _args| {
            let selector =
                receiver.with_native_state::<NativeMethodState, _, _>(|m| m.selector.clone())?;
            Ok(vm.new_symbol(mc, selector))
        })
        .instance_method("name", |vm, mc, receiver, _args| {
            let selector =
                receiver.with_native_state::<NativeMethodState, _, _>(|m| m.selector.clone())?;
            Ok(vm.new_symbol(mc, selector))
        })
        .instance_method("extension?", |vm, mc, receiver, _args| {
            let is_ext =
                receiver.with_native_state::<NativeMethodState, _, _>(|m| m.is_extension)?;
            Ok(vm.new_bool(mc, is_ext))
        })
        .instance_method("block", |vm, mc, receiver, _args| {
            // A native method has no user block; report it as nil.
            let block = receiver.with_native_state::<NativeMethodState, _, _>(|m| m.get_block())?;
            Ok(block.unwrap_or_else(|| vm.new_nil(mc)))
        })
        .instance_method("callOn:", |vm, mc, receiver, args| {
            let block_val =
                receiver.with_native_state::<NativeMethodState, _, _>(|m| m.get_block())?;
            let receiver = args[0];
            if let Some(Value::Object(obj)) = block_val
                && let ObjectPayload::Block(block) = &obj.borrow().payload
            {
                vm.execute_block(mc, block.clone(), Vec::new(), Some(receiver))
            } else {
                Err(QuoinError::Other("Method block is not a Block".to_string()))
            }
        })
        .instance_method("==:", |vm, mc, receiver, args| {
            Ok(vm.new_bool(mc, receiver == args[0]))
        })
}
