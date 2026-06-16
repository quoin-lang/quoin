use crate::arg;
use crate::value::{AnyCollect, NativeClassBuilder, Value};

use gc_arena::collect::{DynCollect, Trace};
use std::any::Any;
use std::collections::HashMap;

#[derive(Debug)]
pub struct NativeMapState {
    pub map: HashMap<String, Value<'static>>,
}

impl NativeMapState {
    pub fn new(map: HashMap<String, Value<'_>>) -> Self {
        let map_static: HashMap<String, Value<'static>> = unsafe { std::mem::transmute(map) };
        Self { map: map_static }
    }

    pub fn get_map<'gc>(&self) -> &HashMap<String, Value<'gc>> {
        unsafe { std::mem::transmute(&self.map) }
    }

    pub fn get_map_mut<'gc>(&mut self) -> &mut HashMap<String, Value<'gc>> {
        unsafe { std::mem::transmute(&mut self.map) }
    }
}

impl AnyCollect for NativeMapState {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn trace_gc<'gc>(&self, cc: &mut dyn Trace<'gc>) {
        for val in self.map.values() {
            let val_gc: &Value<'gc> = unsafe { std::mem::transmute(val) };
            val_gc.dyn_trace(cc);
        }
    }
}

pub fn build_map_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Map", Some("Object"))
        //
        .instance_method("containsKey?:", |vm, mc, args| {
            let key = arg!(args, String, 1).to_string();
            let b = args[0].with_native_state(|m: &NativeMapState| m.map.contains_key(&key))?;
            Ok(vm.new_bool(mc, b))
        })
        .instance_method("at:", |vm, mc, args| {
            let key = arg!(args, String, 1).to_string();
            let value =
                args[0].with_native_state(|m: &NativeMapState| m.get_map().get(&key).copied())?;
            Ok(if let Some(v) = value {
                v
            } else {
                vm.new_nil(mc)
            })
        })
        .instance_method("count", |vm, mc, args| {
            Ok(vm.new_int(
                mc,
                args[0].with_native_state(|m: &NativeMapState| m.get_map().len())? as i64,
            ))
        })
}
