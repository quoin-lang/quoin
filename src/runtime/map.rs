use crate::arg;
use crate::error::BBError;
use crate::value::{AnyCollect, NativeClassBuilder, Value};
use crate::vm::VmStatus;

use gc_arena::collect::{DynCollect, Trace};
use gc_arena::{Gc, lock::RefLock};
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

    pub fn set_key<'gc>(&mut self, key: Value<'gc>) {
        let key_static: Value<'static> = unsafe { std::mem::transmute(key) };
        self.key = key_static;
    }

    pub fn set_value<'gc>(&mut self, value: Value<'gc>) {
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
        .class_method("new:", |vm, mc, args| {
            let class_obj = match args[0] {
                Value::Class(c) => c,
                _ => {
                    return Err(BBError::TypeError {
                        expected: "Class".to_string(),
                        got: args[0].type_name().to_string(),
                        msg: "new: expects Class receiver".to_string(),
                    });
                }
            };
            let block = if let Value::Object(obj) = args[1]
                && let crate::value::ObjectPayload::Block(b) = &obj.borrow().payload
            {
                *b
            } else {
                return Err(BBError::TypeError {
                    expected: "Block".to_string(),
                    got: args[1].type_name().to_string(),
                    msg: "new: expects a Block".to_string(),
                });
            };

            let initial_frame_count = vm.frames.len();
            vm.start_block(mc, block, Vec::new(), None, None);

            let env_ref = vm.frames.last().unwrap().env;

            while vm.frames.len() > initial_frame_count {
                match vm.step(mc)? {
                    VmStatus::Running => {}
                    VmStatus::Finished(_) => break,
                    VmStatus::Yeeted(val) => {
                        return Err(BBError::Other(format!(
                            "Uncaught exception during block execution: {}",
                            val
                        )));
                    }
                }
            }

            // Pop the block's return value to clean up the stack
            let _block_ret = vm.pop().map_err(|e| BBError::Other(e))?;

            let env_borrow = env_ref.borrow();
            let key = env_borrow
                .vars
                .get("key")
                .copied()
                .unwrap_or_else(|| vm.new_nil(mc));
            let value = env_borrow
                .vars
                .get("value")
                .copied()
                .unwrap_or_else(|| vm.new_nil(mc));

            let state = NativeKeyValuePairState::new(key, value);
            let boxed_state: Box<dyn AnyCollect> = Box::new(state);
            let obj = vm.new_object(mc, class_obj);
            obj.borrow_mut(mc).payload =
                crate::value::ObjectPayload::NativeState(crate::gc!(mc, RefLock::new(boxed_state)));

            Ok(Value::Object(obj))
        })
        .instance_method("init:", |_vm, mc, args| {
            let key = args[1];
            let value = args[2];
            args[0].with_native_state_mut(mc, |kvp: &mut NativeKeyValuePairState| {
                kvp.set_key(key);
                kvp.set_value(value);
            })?;
            Ok(args[0])
        })
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
                && let crate::value::ObjectPayload::String(s) = &obj.borrow().payload
            {
                s.to_string()
            } else {
                format!("{}", key_s_val)
            };

            let val_s_val = vm.call_method(mc, value, "s", vec![])?;
            let val_s = if let Value::Object(obj) = val_s_val
                && let crate::value::ObjectPayload::String(s) = &obj.borrow().payload
            {
                s.to_string()
            } else {
                format!("{}", val_s_val)
            };

            Ok(vm.new_string(mc, format!("{}:{}", key_s, val_s)))
        })
}
