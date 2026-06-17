use crate::arg;
use crate::value::{AnyCollect, NativeClassBuilder, ObjectPayload, OpaqueState, Value};

use gc_arena::collect::{DynCollect, Trace};
use std::any::Any;
use std::collections::HashMap;

#[derive(Debug)]
pub struct NativeMapState {
    pub map: HashMap<String, Value<'static>>,
    pub iter: Option<std::vec::IntoIter<String>>,
}

impl NativeMapState {
    pub fn new(map: HashMap<String, Value<'_>>) -> Self {
        let map_static: HashMap<String, Value<'static>> = unsafe { std::mem::transmute(map) };
        Self {
            map: map_static,
            iter: None,
        }
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
        .instance_method("at:put:", |_vm, mc, args| {
            let key = arg!(args, String, 1).to_string();
            let val = args[2];
            args[0].with_native_state_mut(mc, |m: &mut NativeMapState| {
                m.get_map_mut().insert(key, val)
            })?;
            Ok(args[0])
        })
        .instance_method("count", |vm, mc, args| {
            Ok(vm.new_int(
                mc,
                args[0].with_native_state(|m: &NativeMapState| m.get_map().len())? as i64,
            ))
        })
        .instance_method("next", |vm, mc, args| {
            let kv_opt = args[0].with_native_state_mut(mc, |m: &mut NativeMapState| {
                if m.iter.is_none() {
                    let keys: Vec<String> = m.map.keys().cloned().collect();
                    m.iter = Some(keys.into_iter());
                }
                if let Some(key) = m.iter.as_mut().unwrap().next() {
                    let val_static = m
                        .map
                        .get(&key)
                        .copied()
                        .unwrap_or_else(|| unsafe { std::mem::transmute(vm.new_nil(mc)) });
                    let val_gc = unsafe { std::mem::transmute(val_static) };
                    Some((key, val_gc))
                } else {
                    None
                }
            })?;

            if let Some((key, val)) = kv_opt {
                let kvp_state = NativeKeyValuePairState::new(vm.new_string(mc, key), val);
                let kvp_class = vm.get_builtin_class("KeyValuePair");
                let obj = vm.new_native_state(mc, kvp_class, OpaqueState(kvp_state));
                Ok(obj)
            } else {
                Ok(vm.new_nil(mc))
            }
        })
        .instance_method("reset", |vm, mc, args| {
            args[0].with_native_state_mut(mc, |m: &mut NativeMapState| {
                let keys: Vec<String> = m.map.keys().cloned().collect();
                m.iter = Some(keys.into_iter());
            })?;
            Ok(vm.new_nil(mc))
        })
}

#[derive(Debug)]
pub struct NativeKeyValuePairState {
    pub key: Value<'static>,
    pub value: Value<'static>,
}

impl NativeKeyValuePairState {
    pub fn new(key: Value<'_>, value: Value<'_>) -> Self {
        let key_static: Value<'static> = unsafe { std::mem::transmute(key) };
        let value_static: Value<'static> = unsafe { std::mem::transmute(value) };
        Self {
            key: key_static,
            value: value_static,
        }
    }

    pub fn get_key<'gc>(&self) -> Value<'gc> {
        unsafe { std::mem::transmute(self.key) }
    }

    pub fn get_value<'gc>(&self) -> Value<'gc> {
        unsafe { std::mem::transmute(self.value) }
    }

    pub fn set_key(&mut self, key: Value) {
        let key_static: Value<'static> = unsafe { std::mem::transmute(key) };
        self.key = key_static;
    }

    pub fn set_value(&mut self, value: Value) {
        let value_static: Value<'static> = unsafe { std::mem::transmute(value) };
        self.value = value_static;
    }
}

impl AnyCollect for NativeKeyValuePairState {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn trace_gc<'gc>(&self, cc: &mut dyn Trace<'gc>) {
        let key_gc: &Value<'gc> = unsafe { std::mem::transmute(&self.key) };
        key_gc.dyn_trace(cc);
        let value_gc: &Value<'gc> = unsafe { std::mem::transmute(&self.value) };
        value_gc.dyn_trace(cc);
    }
}

pub fn build_key_value_pair_class() -> NativeClassBuilder {
    NativeClassBuilder::new("KeyValuePair", Some("Object"))
        .instance_method("key", |_vm, _mc, args| {
            let key = args[0].with_native_state(|kvp: &NativeKeyValuePairState| kvp.get_key())?;
            Ok(key)
        })
        .instance_method("value", |_vm, _mc, args| {
            let value =
                args[0].with_native_state(|kvp: &NativeKeyValuePairState| kvp.get_value())?;
            Ok(value)
        })
        .instance_method("s", |vm, mc, args| {
            let (key, value) =
                args[0].with_native_state::<NativeKeyValuePairState, _, _>(|kvp| {
                    (kvp.get_key(), kvp.get_value())
                })?;

            let key_s_val = vm.call_method(mc, key, "s", vec![])?;
            let key_s = if let Value::Object(obj) = key_s_val
                && let ObjectPayload::String(s) = &obj.borrow().payload
            {
                s.to_string()
            } else {
                format!("{}", key_s_val)
            };

            let val_s_val = vm.call_method(mc, value, "s", vec![])?;
            let val_s = if let Value::Object(obj) = val_s_val
                && let ObjectPayload::String(s) = &obj.borrow().payload
            {
                s.to_string()
            } else {
                format!("{}", val_s_val)
            };

            Ok(vm.new_string(mc, format!("{}:{}", key_s, val_s)))
        })
}
