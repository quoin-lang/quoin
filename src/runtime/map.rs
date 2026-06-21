use crate::arg;
use crate::error::QuoinError;
use crate::value::{AnyCollect, NativeClassBuilder, ObjectPayload, Value};
use crate::vm::VmStatus;

use gc_arena::Gc;
use gc_arena::collect::{DynCollect, Trace};
use gc_arena::lock::RefLock;
use std::any::Any;
use std::collections::HashMap;
use std::mem::transmute;

#[derive(Debug)]
pub struct NativeMapState {
    pub map: HashMap<String, Value<'static>>,
}

impl NativeMapState {
    pub fn new(map: HashMap<String, Value<'_>>) -> Self {
        let map_static: HashMap<String, Value<'static>> = unsafe { transmute(map) };
        Self { map: map_static }
    }

    pub fn get_map<'gc>(&self) -> &HashMap<String, Value<'gc>> {
        unsafe { transmute(&self.map) }
    }

    pub fn get_map_mut<'gc>(&mut self) -> &mut HashMap<String, Value<'gc>> {
        unsafe { transmute(&mut self.map) }
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
            let val_gc: &Value<'gc> = unsafe { transmute(val) };
            val_gc.dyn_trace(cc);
        }
    }
}

pub fn build_map_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Map", Some("Object"))
        //
        .instance_method("containsKey?:", |vm, mc, receiver, args| {
            let key = arg!(args, String, 0).to_string();
            let b = receiver.with_native_state(|m: &NativeMapState| m.map.contains_key(&key))?;
            Ok(vm.new_bool(mc, b))
        })
        .instance_method("at:", |vm, mc, receiver, args| {
            let key = arg!(args, String, 0).to_string();
            let value =
                receiver.with_native_state(|m: &NativeMapState| m.get_map().get(&key).copied())?;
            Ok(if let Some(v) = value {
                v
            } else {
                vm.new_nil(mc)
            })
        })
        .instance_method("at:put:", |_vm, mc, receiver, args| {
            let key = arg!(args, String, 0).to_string();
            let val = args[1];
            receiver.with_native_state_mut(mc, |m: &mut NativeMapState| {
                m.get_map_mut().insert(key, val)
            })?;
            Ok(receiver)
        })
        .instance_method("count", |vm, mc, receiver, _args| {
            Ok(vm.new_int(
                mc,
                receiver.with_native_state(|m: &NativeMapState| m.get_map().len())? as i64,
            ))
        })
        .instance_method("keys", |vm, mc, receiver, _args| {
            let keys_vec = receiver.with_native_state(|m: &NativeMapState| {
                m.get_map()
                    .keys()
                    .map(|v| vm.new_string(mc, v.clone()))
                    .collect::<Vec<_>>()
            })?;
            Ok(vm.new_list(mc, keys_vec))
        })
        .instance_method("values", |vm, mc, receiver, _args| {
            let values_vec = receiver.with_native_state(|m: &NativeMapState| {
                m.get_map().values().map(|v| *v).collect::<Vec<_>>()
            })?;
            Ok(vm.new_list(mc, values_vec))
        })
        .instance_method("==:", |vm, mc, receiver, args| {
            let lhs_map =
                receiver.with_native_state::<NativeMapState, _, _>(|m| m.get_map().clone())?;
            let rhs_map_res =
                args[0].with_native_state::<NativeMapState, _, _>(|m| m.get_map().clone());
            let rhs_map = match rhs_map_res {
                Ok(m) => m,
                Err(_) => return Ok(vm.new_bool(mc, false)),
            };

            if lhs_map.len() != rhs_map.len() {
                return Ok(vm.new_bool(mc, false));
            }

            let keys: Vec<String> = lhs_map.keys().cloned().collect();
            for key in keys {
                let lhs_val = receiver
                    .with_native_state::<NativeMapState, _, _>(|m| m.get_map().get(&key).copied())
                    .map_err(|e| QuoinError::Other(e))?
                    .ok_or_else(|| QuoinError::Other("Key missing in lhs".to_string()))?;

                let rhs_val = args[0]
                    .with_native_state::<NativeMapState, _, _>(|m| m.get_map().get(&key).copied())
                    .map_err(|e| QuoinError::Other(e))?
                    .ok_or_else(|| QuoinError::Other("Key missing in rhs".to_string()))?;

                let eq_res = vm.call_method(mc, lhs_val, "==:", vec![rhs_val])?.is_true();
                if !eq_res {
                    return Ok(vm.new_bool(mc, false));
                }
            }

            Ok(vm.new_bool(mc, true))
        })
}

#[derive(Debug)]
pub struct NativeKeyValuePairState {
    pub key: Value<'static>,
    pub value: Value<'static>,
}

impl NativeKeyValuePairState {
    pub fn new(key: Value<'_>, value: Value<'_>) -> Self {
        let key_static: Value<'static> = unsafe { transmute(key) };
        let value_static: Value<'static> = unsafe { transmute(value) };
        Self {
            key: key_static,
            value: value_static,
        }
    }

    pub fn get_key<'gc>(&self) -> Value<'gc> {
        unsafe { transmute(self.key) }
    }

    pub fn get_value<'gc>(&self) -> Value<'gc> {
        unsafe { transmute(self.value) }
    }

    pub fn set_key(&mut self, key: Value) {
        let key_static: Value<'static> = unsafe { transmute(key) };
        self.key = key_static;
    }

    pub fn set_value(&mut self, value: Value) {
        let value_static: Value<'static> = unsafe { transmute(value) };
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
        let key_gc: &Value<'gc> = unsafe { transmute(&self.key) };
        key_gc.dyn_trace(cc);
        let value_gc: &Value<'gc> = unsafe { transmute(&self.value) };
        value_gc.dyn_trace(cc);
    }
}

pub fn build_key_value_pair_class() -> NativeClassBuilder {
    NativeClassBuilder::new("KeyValuePair", Some("Object"))
        .class_method("new:", |vm, mc, receiver, args| {
            if !matches!(receiver, Value::Class(_)) {
                return Err(QuoinError::TypeError {
                    expected: "Class".to_string(),
                    got: receiver.type_name().to_string(),
                    msg: "new: expects Class receiver".to_string(),
                });
            }
            let block = if let Value::Object(obj) = args[0]
                && let ObjectPayload::Block(b) = &obj.borrow().payload
            {
                *b
            } else {
                return Err(QuoinError::TypeError {
                    expected: "Block".to_string(),
                    got: args[0].type_name().to_string(),
                    msg: "new: expects a Block".to_string(),
                });
            };

            let initial_frame_count = vm.frames.len();
            vm.start_block(mc, block, Vec::new(), None, None);

            while vm.frames.len() > initial_frame_count {
                match vm.step_internal(mc) {
                    Ok(VmStatus::Running) => {}
                    Ok(VmStatus::Finished(_)) => break,
                    Ok(VmStatus::Yeeted(val)) => {
                        return Err(QuoinError::Other(format!(
                            "Uncaught exception during block execution: {}",
                            val
                        )));
                    }
                    Err(QuoinError::NonLocalReturn) => {
                        if vm.frames.len() > initial_frame_count {
                            continue;
                        } else if vm.frames.len() == initial_frame_count {
                            break;
                        } else {
                            return Err(QuoinError::NonLocalReturn);
                        }
                    }
                    Err(e) => return Err(e),
                }
            }

            // Pop the block's return value to clean up the stack
            let _block_ret = vm.pop().map_err(|e| QuoinError::Other(e))?;

            // Retrieve environment from the last popped frame recorded in VmState
            let env_ref = vm.last_popped_env.ok_or_else(|| {
                QuoinError::Other("Missing environment from block execution".to_string())
            })?;
            let env_borrow = env_ref.borrow();
            let key = env_borrow
                .lookup_str("key")
                .unwrap_or_else(|| vm.new_nil(mc));
            let value = env_borrow
                .lookup_str("value")
                .unwrap_or_else(|| vm.new_nil(mc));

            let state = NativeKeyValuePairState::new(key, value);
            let boxed_state: Box<dyn AnyCollect> = Box::new(state);
            let active_class_val = vm.active_native_args.last().unwrap().receiver;
            let class_obj = match active_class_val {
                Value::Class(c) => c,
                _ => {
                    return Err(QuoinError::TypeError {
                        expected: "Class".to_string(),
                        got: active_class_val.type_name().to_string(),
                        msg: "new: expects Class receiver".to_string(),
                    });
                }
            };
            let obj = vm.new_object(mc, class_obj);
            obj.borrow_mut(mc).payload =
                ObjectPayload::NativeState(crate::gc!(mc, RefLock::new(boxed_state)));

            Ok(Value::Object(obj))
        })
        .instance_method("init:", |_vm, mc, receiver, args| {
            let key = args[0];
            let value = args[1];
            receiver.with_native_state_mut(mc, |kvp: &mut NativeKeyValuePairState| {
                kvp.set_key(key);
                kvp.set_value(value);
            })?;
            Ok(receiver)
        })
        .instance_method("key", |_vm, _mc, receiver, _args| {
            let key = receiver.with_native_state(|kvp: &NativeKeyValuePairState| kvp.get_key())?;
            Ok(key)
        })
        .instance_method("value", |_vm, _mc, receiver, _args| {
            let value =
                receiver.with_native_state(|kvp: &NativeKeyValuePairState| kvp.get_value())?;
            Ok(value)
        })
        .instance_method("s", |vm, mc, receiver, _args| {
            let key =
                receiver.with_native_state::<NativeKeyValuePairState, _, _>(|kvp| kvp.get_key())?;

            let key_s_val = vm.call_method(mc, key, "s", vec![])?;
            let key_s = if let Value::Object(obj) = key_s_val
                && let ObjectPayload::String(s) = &obj.borrow().payload
            {
                s.to_string()
            } else {
                format!("{}", key_s_val)
            };

            let active_receiver = vm.active_native_args.last().unwrap().receiver;
            let value = active_receiver
                .with_native_state::<NativeKeyValuePairState, _, _>(|kvp| kvp.get_value())?;

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
        .instance_method("==:", |vm, mc, receiver, args| {
            let lhs_key =
                receiver.with_native_state::<NativeKeyValuePairState, _, _>(|kvp| kvp.get_key())?;
            let rhs_key_res =
                args[0].with_native_state::<NativeKeyValuePairState, _, _>(|kvp| kvp.get_key());
            let rhs_key = match rhs_key_res {
                Ok(k) => k,
                Err(_) => return Ok(vm.new_bool(mc, false)),
            };

            let keys_eq = vm.call_method(mc, lhs_key, "==:", vec![rhs_key])?.is_true();
            if !keys_eq {
                return Ok(vm.new_bool(mc, false));
            }

            let active_lhs = vm.active_native_args.last().unwrap().receiver;
            let active_rhs = vm.active_native_args.last().unwrap().args[0];

            let lhs_val = active_lhs
                .with_native_state::<NativeKeyValuePairState, _, _>(|kvp| kvp.get_value())?;
            let rhs_val = active_rhs
                .with_native_state::<NativeKeyValuePairState, _, _>(|kvp| kvp.get_value())?;

            let vals_eq = vm.call_method(mc, lhs_val, "==:", vec![rhs_val])?.is_true();
            Ok(vm.new_bool(mc, vals_eq))
        })
}
