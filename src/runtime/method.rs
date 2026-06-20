use crate::error::BBError;
use crate::value::{AnyCollect, NativeClassBuilder, ObjectPayload, Value};

use gc_arena::collect::{DynCollect, Trace};
use std::any::Any;
use std::mem::transmute;

#[derive(Debug)]
pub struct NativeMethodState {
    pub selector: String,
    pub block: Value<'static>,
    pub is_extension: bool,
    pub next: Option<Value<'static>>,
}

impl NativeMethodState {
    pub fn new(selector: String, block: Value<'_>, is_extension: bool) -> Self {
        let block_static: Value<'static> = unsafe { transmute(block) };
        Self {
            selector,
            block: block_static,
            is_extension,
            next: None,
        }
    }

    pub fn get_block<'gc>(&self) -> Value<'gc> {
        unsafe { transmute(self.block) }
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
        let block_gc: &Value<'gc> = unsafe { transmute(&self.block) };
        block_gc.dyn_trace(cc);
        if let Some(next) = &self.next {
            let next_gc: &Value<'gc> = unsafe { transmute(next) };
            next_gc.dyn_trace(cc);
        }
    }
}

pub fn build_method_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Method", Some("Object"))
        .instance_method("selector", |vm, mc, args| {
            let selector =
                args[0].with_native_state::<NativeMethodState, _, _>(|m| m.selector.clone())?;
            Ok(vm.new_symbol(mc, selector))
        })
        .instance_method("name", |vm, mc, args| {
            let selector =
                args[0].with_native_state::<NativeMethodState, _, _>(|m| m.selector.clone())?;
            Ok(vm.new_symbol(mc, selector))
        })
        .instance_method("extension?", |vm, mc, args| {
            let is_ext =
                args[0].with_native_state::<NativeMethodState, _, _>(|m| m.is_extension)?;
            Ok(vm.new_bool(mc, is_ext))
        })
        .instance_method("block", |_vm, _mc, args| {
            let block = args[0].with_native_state::<NativeMethodState, _, _>(|m| m.get_block())?;
            Ok(block)
        })
        .instance_method("callOn:", |vm, mc, args| {
            let block_val =
                args[0].with_native_state::<NativeMethodState, _, _>(|m| m.get_block())?;
            let receiver = args[1];
            if let Value::Object(obj) = block_val
                && let ObjectPayload::Block(block) = &obj.borrow().payload
            {
                vm.execute_block(mc, block.clone(), Vec::new(), Some(receiver))
            } else {
                Err(BBError::Other("Method block is not a Block".to_string()))
            }
        })
        .instance_method(
            "==:",
            |vm, mc, args| Ok(vm.new_bool(mc, args[0] == args[1])),
        )
}
