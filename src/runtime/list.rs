use crate::arg;
use crate::error::QuoinError;
use crate::value::{AnyCollect, NativeClassBuilder, ObjectPayload, Value};

use gc_arena::collect::{DynCollect, Trace};
use std::any::Any;
use std::mem::transmute;

#[derive(Debug)]
pub struct NativeListState {
    pub vec: Vec<Value<'static>>,
}

impl NativeListState {
    pub fn new(vec: Vec<Value<'_>>) -> Self {
        let vec_static: Vec<Value<'static>> = unsafe { transmute(vec) };
        Self { vec: vec_static }
    }

    pub fn get_vec<'gc>(&self) -> &[Value<'gc>] {
        unsafe { transmute(&self.vec[..]) }
    }

    pub fn get_vec_mut<'gc>(&mut self) -> &mut Vec<Value<'gc>> {
        unsafe { transmute(&mut self.vec) }
    }
}

impl AnyCollect for NativeListState {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn trace_gc<'gc>(&self, cc: &mut dyn Trace<'gc>) {
        for val in &self.vec {
            let val_gc: &Value<'gc> = unsafe { transmute(val) };
            val_gc.dyn_trace(cc);
        }
    }
}

pub fn build_list_class() -> NativeClassBuilder {
    NativeClassBuilder::new("List", Some("Object"))
        .instance_method("count", |vm, mc, args| {
            let len = args[0]
                .with_native_state::<NativeListState, _, _>(|l| l.get_vec().len())
                .map_err(|e| QuoinError::Other(e))?;
            Ok(vm.new_int(mc, len as i64))
        })
        .instance_method("add:", |_vm, mc, args| {
            args[0]
                .with_native_state_mut::<NativeListState, _, _>(mc, |l| {
                    let vec = l.get_vec_mut();
                    vec.push(args[1]);
                })
                .map_err(|e| QuoinError::Other(e))?;
            Ok(args[0])
        })
        .instance_method("push:", |_vm, mc, args| {
            args[0]
                .with_native_state_mut::<NativeListState, _, _>(mc, |l| {
                    let vec = l.get_vec_mut();
                    vec.insert(0, args[1]);
                })
                .map_err(|e| QuoinError::Other(e))?;
            Ok(args[0])
        })
        // The index is typed, so a non-Integer index matches no variant -> MNU
        // (dispatch enforces the type instead of a hand-rolled TypeError).
        .typed_instance_method("at:", &["Integer"], |vm, mc, args| {
            let idx = arg!(args, Int, 1);
            args[0]
                .with_native_state::<NativeListState, _, _>(|l| {
                    let vec = l.get_vec();
                    if idx >= 0 && idx < vec.len() as i64 {
                        Ok(vec[idx as usize])
                    } else {
                        Ok(vm.new_nil(mc))
                    }
                })
                .map_err(|e| QuoinError::Other(e))?
        })
        // Only the index is typed (`&["Integer"]`); the value (arg 2) is any type.
        .typed_instance_method("at:put:", &["Integer"], |_vm, mc, args| {
            let idx = arg!(args, Int, 1);
            let val = args[2];
            args[0]
                .with_native_state_mut(mc, |l: &mut NativeListState| {
                    let vec = l.get_vec_mut();
                    if idx >= 0 && idx < vec.len() as i64 {
                        vec[idx as usize] = val;
                        Ok(())
                    } else {
                        Err(QuoinError::Other(format!(
                            "Index out of bounds: index is {}, but length is {}",
                            idx,
                            vec.len()
                        )))
                    }
                })
                .map_err(|e| QuoinError::Other(e))??;
            Ok(args[0])
        })
        .typed_instance_method("sliceFrom:", &["Integer"], |vm, mc, args| {
            let idx = arg!(args, Int, 1);
            args[0]
                .with_native_state::<NativeListState, _, _>(|l| {
                    let vec = l.get_vec();
                    let start = idx.max(0) as usize;
                    let sliced = if start < vec.len() {
                        vec[start..].to_vec()
                    } else {
                        Vec::new()
                    };
                    Ok(vm.new_list(mc, sliced))
                })
                .map_err(|e| QuoinError::Other(e))?
        })
        .instance_method("s", |vm, mc, args| {
            let len = args[0]
                .with_native_state::<NativeListState, _, _>(|l| l.get_vec().len())
                .map_err(|e| QuoinError::Other(e))?;

            let mut parts = Vec::new();
            for i in 0..len {
                let val = args[0]
                    .with_native_state::<NativeListState, _, _>(|l| l.get_vec().get(i).copied())
                    .map_err(|e| QuoinError::Other(e))?
                    .ok_or_else(|| QuoinError::Other("Index out of bounds".to_string()))?;

                let result = vm.call_method(mc, val, "s", vec![])?;
                let part = if let Value::Object(obj) = result {
                    if let ObjectPayload::String(s) = &obj.borrow().payload {
                        s.to_string()
                    } else {
                        format!("{}", result)
                    }
                } else {
                    format!("{}", result)
                };
                parts.push(part);
            }

            Ok(vm.new_string(mc, format!("#({})", parts.join(" "))))
        })
        .instance_method("==:", |vm, mc, args| {
            let lhs_len = args[0]
                .with_native_state::<NativeListState, _, _>(|l| l.get_vec().len())
                .map_err(|e| QuoinError::Other(e))?;
            let rhs_len_res =
                args[1].with_native_state::<NativeListState, _, _>(|l| l.get_vec().len());
            let rhs_len = match rhs_len_res {
                Ok(len) => len,
                Err(_) => return Ok(vm.new_bool(mc, false)),
            };

            if lhs_len != rhs_len {
                return Ok(vm.new_bool(mc, false));
            }

            for i in 0..lhs_len {
                let lhs_val = args[0]
                    .with_native_state::<NativeListState, _, _>(|l| l.get_vec().get(i).copied())
                    .map_err(|e| QuoinError::Other(e))?
                    .ok_or_else(|| QuoinError::Other("Index out of bounds".to_string()))?;
                let rhs_val = args[1]
                    .with_native_state::<NativeListState, _, _>(|l| l.get_vec().get(i).copied())
                    .map_err(|e| QuoinError::Other(e))?
                    .ok_or_else(|| QuoinError::Other("Index out of bounds".to_string()))?;

                let eq_res = vm.call_method(mc, lhs_val, "==:", vec![rhs_val])?.is_true();
                if !eq_res {
                    return Ok(vm.new_bool(mc, false));
                }
            }

            Ok(vm.new_bool(mc, true))
        })
        .instance_method("bind:", |vm, mc, args| {
            let block = arg!(args, Block, 1);
            let block_args = args[0].with_native_state(|l: &NativeListState| {
                l.get_vec()
                    .iter()
                    .take(block.param_names.len())
                    .map(|v| unsafe { transmute(*v) })
                    .collect::<Vec<_>>()
            })?;

            vm.execute_block(mc, block, block_args, None)
        })
        .instance_method("sort", |vm, mc, args| {
            let len = args[0]
                .with_native_state::<NativeListState, _, _>(|l| l.get_vec().len())
                .map_err(|e| QuoinError::Other(e))?;

            for i in 1..len {
                let mut j = i;
                while j > 0 {
                    let active_args = vm.active_native_args.last().unwrap();
                    let (val_prev, val_curr) = active_args[0]
                        .with_native_state::<NativeListState, _, _>(|l| {
                            (l.get_vec()[j - 1], l.get_vec()[j])
                        })
                        .map_err(|e| QuoinError::Other(e))?;

                    let gt_res = if val_prev.is_nil() {
                        !val_curr.is_nil()
                    } else if val_curr.is_nil() {
                        false
                    } else {
                        vm.call_method(mc, val_prev, ">:", vec![val_curr])?.is_true()
                    };

                    if gt_res {
                        let active_args = vm.active_native_args.last().unwrap();
                        active_args[0]
                            .with_native_state_mut(mc, |l: &mut NativeListState| {
                                l.get_vec_mut().swap(j - 1, j);
                            })
                            .map_err(|e| QuoinError::Other(e))?;
                        j -= 1;
                    } else {
                        break;
                    }
                }
            }

            let active_args = vm.active_native_args.last().unwrap();
            Ok(active_args[0])
        })
        .instance_method("sort:", |vm, mc, args| {
            let block_gc = arg!(args, Block, 1);
            let len = args[0]
                .with_native_state::<NativeListState, _, _>(|l| l.get_vec().len())
                .map_err(|e| QuoinError::Other(e))?;

            let arity = block_gc.param_names.len();
            if arity == 1 {
                for i in 1..len {
                    let mut j = i;
                    while j > 0 {
                        let active_args = vm.active_native_args.last().unwrap();
                        let val_prev = active_args[0]
                            .with_native_state::<NativeListState, _, _>(|l| l.get_vec()[j - 1])
                            .map_err(|e| QuoinError::Other(e))?;

                        let key_lhs = vm.call_method(
                            mc,
                            active_args[1],
                            "valueWithArgs:",
                            vec![vm.new_list(mc, vec![val_prev])],
                        )?;
                        vm.push(key_lhs);

                        let active_args = vm.active_native_args.last().unwrap();
                        let val_curr = active_args[0]
                            .with_native_state::<NativeListState, _, _>(|l| l.get_vec()[j])
                            .map_err(|e| QuoinError::Other(e))?;

                        let key_rhs = vm.call_method(
                            mc,
                            active_args[1],
                            "valueWithArgs:",
                            vec![vm.new_list(mc, vec![val_curr])],
                        )?;
                        let key_lhs = vm.pop()?;

                        let gt_res = if key_lhs.is_nil() {
                            !key_rhs.is_nil()
                        } else if key_rhs.is_nil() {
                            false
                        } else {
                            vm.call_method(mc, key_lhs, ">:", vec![key_rhs])?.is_true()
                        };

                        if gt_res {
                            let active_args = vm.active_native_args.last().unwrap();
                            active_args[0]
                                .with_native_state_mut(mc, |l: &mut NativeListState| {
                                    l.get_vec_mut().swap(j - 1, j);
                                })
                                .map_err(|e| QuoinError::Other(e))?;
                            j -= 1;
                        } else {
                            break;
                        }
                    }
                }
            } else {
                for i in 1..len {
                    let mut j = i;
                    while j > 0 {
                        let active_args = vm.active_native_args.last().unwrap();
                        let (val_prev, val_curr) = active_args[0]
                            .with_native_state::<NativeListState, _, _>(|l| {
                                (l.get_vec()[j - 1], l.get_vec()[j])
                            })
                            .map_err(|e| QuoinError::Other(e))?;

                        let res = vm.call_method(
                            mc,
                            active_args[1],
                            "valueWithArgs:",
                            vec![vm.new_list(mc, vec![val_prev, val_curr])],
                        )?;

                        if !res.is_true() {
                            let active_args = vm.active_native_args.last().unwrap();
                            active_args[0]
                                .with_native_state_mut(mc, |l: &mut NativeListState| {
                                    l.get_vec_mut().swap(j - 1, j);
                                })
                                .map_err(|e| QuoinError::Other(e))?;
                            j -= 1;
                        } else {
                            break;
                        }
                    }
                }
            }

            let active_args = vm.active_native_args.last().unwrap();
            Ok(active_args[0])
        })
}
