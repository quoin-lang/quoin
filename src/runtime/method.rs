use crate::error::QuoinError;
use crate::value::{AnyCollect, NativeClassBuilder, NativeFunc, Value};

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
        /// Declared checker return type (Fork-1b native half), e.g. `Some("String")`. Compile-time
        /// only — never consulted at dispatch; surfaced via `introspect` for the type checker.
        ret_type: Option<String>,
    },
    /// An extension-backed method (Phase 3): the selector dispatches over the socket to `ext`
    /// (the owning `Extension` instance, kept GC-rooted via the method table). Whether the send
    /// is class-side (a constructor) or instance-side is derived from the receiver at dispatch.
    ExtDispatch {
        ext: Value<'static>,
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
        ret_type: Option<String>,
    ) -> Self {
        Self {
            selector,
            body: MethodBody::Native {
                func,
                param_types,
                ret_type,
            },
            is_extension: false,
            next: None,
        }
    }

    /// An extension-backed method variant (Phase 3): `selector` dispatches over the socket to the
    /// owning `Extension` instance `ext`.
    pub fn new_ext(selector: String, ext: Value<'_>) -> Self {
        let ext_static: Value<'static> = unsafe { transmute(ext) };
        Self {
            selector,
            body: MethodBody::ExtDispatch { ext: ext_static },
            is_extension: true,
            next: None,
        }
    }

    /// The owning `Extension` instance for an extension-backed method, or `None` otherwise.
    pub fn ext_dispatch<'gc>(&self) -> Option<Value<'gc>> {
        match &self.body {
            MethodBody::ExtDispatch { ext } => Some(unsafe { transmute(*ext) }),
            _ => None,
        }
    }

    /// The wrapped user `Block` value, or `None` for a native or extension-backed method body.
    pub fn get_block<'gc>(&self) -> Option<Value<'gc>> {
        match &self.body {
            MethodBody::UserBlock(block) => Some(unsafe { transmute(*block) }),
            MethodBody::Native { .. } | MethodBody::ExtDispatch { .. } => None,
        }
    }

    /// The next variant in this selector's multimethod chain, or `None` at the end.
    pub fn get_next<'gc>(&self) -> Option<Value<'gc>> {
        self.next.map(|n| unsafe { transmute(n) })
    }

    /// The native function, or `None` for a user block or extension-backed body.
    pub fn native_func(&self) -> Option<NativeFunc> {
        match &self.body {
            MethodBody::Native { func, .. } => Some(*func),
            MethodBody::UserBlock(_) | MethodBody::ExtDispatch { .. } => None,
        }
    }

    /// Declared parameter types for a native method, or `None` for a user block or
    /// an untyped (legacy) native method — both of which the scorer handles
    /// elsewhere (a user block scores from its `Block`, an untyped native is a
    /// fallback).
    pub fn native_param_types(&self) -> Option<Vec<String>> {
        match &self.body {
            MethodBody::Native { param_types, .. } => param_types.clone(),
            MethodBody::UserBlock(_) | MethodBody::ExtDispatch { .. } => None,
        }
    }

    /// Declared checker return type for a native method (Fork-1b), or `None` for a user block,
    /// an extension body, or a native method that didn't declare one via `.returns(..)`.
    pub fn native_ret_type(&self) -> Option<String> {
        match &self.body {
            MethodBody::Native { ret_type, .. } => ret_type.clone(),
            MethodBody::UserBlock(_) | MethodBody::ExtDispatch { .. } => None,
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
        // Trace whichever `Value` the body holds: a user block, or the `Extension` instance an
        // extension-backed method dispatches to (its GC root lives here, in the class method table).
        match &self.body {
            MethodBody::UserBlock(block) => {
                let block_gc: &Value<'gc> = unsafe { transmute(block) };
                block_gc.dyn_trace(cc);
            }
            MethodBody::ExtDispatch { ext } => {
                let ext_gc: &Value<'gc> = unsafe { transmute(ext) };
                ext_gc.dyn_trace(cc);
            }
            // MethodBody::Native holds only a fn pointer — nothing to trace.
            MethodBody::Native { .. } => {}
        }
        if let Some(next) = &self.next {
            let next_gc: &Value<'gc> = unsafe { transmute(next) };
            next_gc.dyn_trace(cc);
        }
    }
}

pub fn build_method_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Method", Some("Object"))
        .sdk_instance_method("selector", |host, receiver, _args| {
            let selector =
                receiver.with_native_state::<NativeMethodState, _, _>(|m| m.selector.clone())?;
            Ok(host.new_symbol(selector))
        })
        .sdk_instance_method("name", |host, receiver, _args| {
            let selector =
                receiver.with_native_state::<NativeMethodState, _, _>(|m| m.selector.clone())?;
            Ok(host.new_symbol(selector))
        })
        .sdk_instance_method("extension?", |host, receiver, _args| {
            let is_ext =
                receiver.with_native_state::<NativeMethodState, _, _>(|m| m.is_extension)?;
            Ok(host.new_bool(is_ext))
        })
        .sdk_instance_method("block", |host, receiver, _args| {
            // A native method has no user block; report it as nil.
            let block = receiver.with_native_state::<NativeMethodState, _, _>(|m| m.get_block())?;
            Ok(block.unwrap_or_else(|| host.new_nil()))
        })
        .sdk_instance_method("callOn:", |host, receiver, args| {
            let block_val =
                receiver.with_native_state::<NativeMethodState, _, _>(|m| m.get_block())?;
            let receiver = args[0];
            match block_val {
                // `execute_block` validates that the value is a block.
                Some(block) => host.execute_block(block, Vec::new(), Some(receiver)),
                None => Err(QuoinError::Other("Method block is not a Block".to_string())),
            }
        })
        .sdk_instance_method("==:", |host, receiver, args| {
            Ok(host.new_bool(receiver == args[0]))
        })
}
