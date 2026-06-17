use crate::arg;
use crate::error::BBError;
use crate::value::{AnyCollect, NativeClassBuilder, ObjectPayload, Value};

#[derive(Debug)]
pub struct NativeListState {
    pub idx: usize,
    pub vec: Vec<Value<'static>>,
}

impl NativeListState {
    pub fn new(vec: Vec<Value<'_>>) -> Self {
        let vec_static: Vec<Value<'static>> = unsafe { std::mem::transmute(vec) };
        Self {
            idx: 0,
            vec: vec_static,
        }
    }

    pub fn get_vec<'gc>(&self) -> &[Value<'gc>] {
        unsafe { std::mem::transmute(&self.vec[..]) }
    }

    pub fn get_vec_mut<'gc>(&mut self) -> &mut Vec<Value<'gc>> {
        unsafe { std::mem::transmute(&mut self.vec) }
    }
}

impl AnyCollect for NativeListState {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn trace_gc<'gc>(&self, cc: &mut dyn gc_arena::collect::Trace<'gc>) {
        use gc_arena::collect::DynCollect;
        for val in &self.vec {
            let val_gc: &Value<'gc> = unsafe { std::mem::transmute(val) };
            val_gc.dyn_trace(cc);
        }
    }
}

pub fn build_list_class() -> NativeClassBuilder {
    NativeClassBuilder::new("List", Some("Object"))
        .instance_method("next", |vm, mc, args| {
            let val_opt = args[0].with_native_state_mut(mc, |l: &mut NativeListState| {
                if l.idx < l.vec.len() {
                    let val = l.vec[l.idx];
                    l.idx += 1;
                    Some(val)
                } else {
                    None
                }
            })?;

            Ok(if let Some(val) = val_opt {
                unsafe { std::mem::transmute(val) }
            } else {
                vm.new_nil(mc)
            })
        })
        .instance_method("reset", |vm, mc, args| {
            args[0].with_native_state_mut(mc, |l: &mut NativeListState| {
                l.idx = 0;
            })?;
            Ok(vm.new_nil(mc))
        })
        .instance_method("count", |vm, mc, args| {
            let len = args[0]
                .with_native_state::<NativeListState, _, _>(|l| l.get_vec().len())
                .map_err(|e| BBError::Other(e))?;
            Ok(vm.new_int(mc, len as i64))
        })
        .instance_method("add:", |_vm, mc, args| {
            args[0]
                .with_native_state_mut::<NativeListState, _, _>(mc, |l| {
                    let vec = l.get_vec_mut();
                    vec.push(args[1]);
                })
                .map_err(|e| BBError::Other(e))?;
            Ok(args[0])
        })
        .instance_method("push:", |_vm, mc, args| {
            args[0]
                .with_native_state_mut::<NativeListState, _, _>(mc, |l| {
                    let vec = l.get_vec_mut();
                    vec.insert(0, args[1]);
                })
                .map_err(|e| BBError::Other(e))?;
            Ok(args[0])
        })
        .instance_method("at:", |vm, mc, args| {
            let idx = match args[1] {
                Value::Object(obj) => match &obj.borrow().payload {
                    ObjectPayload::Int(i) => *i,
                    _ => {
                        return Err(BBError::TypeError {
                            expected: "Integer".to_string(),
                            got: args[1].type_name().to_string(),
                            msg: "at expects integer index".to_string(),
                        });
                    }
                },
                _ => {
                    return Err(BBError::TypeError {
                        expected: "Integer".to_string(),
                        got: args[1].type_name().to_string(),
                        msg: "at expects integer index".to_string(),
                    });
                }
            };
            args[0]
                .with_native_state::<NativeListState, _, _>(|l| {
                    let vec = l.get_vec();
                    if idx >= 0 && idx < vec.len() as i64 {
                        Ok(vec[idx as usize])
                    } else {
                        Ok(vm.new_nil(mc))
                    }
                })
                .map_err(|e| BBError::Other(e))?
        })
        .instance_method("sliceFrom:", |vm, mc, args| {
            let idx = match args[1] {
                Value::Object(obj) => match &obj.borrow().payload {
                    ObjectPayload::Int(i) => *i,
                    _ => {
                        return Err(BBError::TypeError {
                            expected: "Integer".to_string(),
                            got: args[1].type_name().to_string(),
                            msg: "sliceFrom expects integer index".to_string(),
                        });
                    }
                },
                _ => {
                    return Err(BBError::TypeError {
                        expected: "Integer".to_string(),
                        got: args[1].type_name().to_string(),
                        msg: "sliceFrom expects integer index".to_string(),
                    });
                }
            };
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
                .map_err(|e| BBError::Other(e))?
        })
        .instance_method("s", |vm, mc, args| {
            let parts = args[0]
                .with_native_state::<NativeListState, _, _>(|l| {
                    let mut parts = Vec::new();
                    for val in l.get_vec() {
                        let result = vm.call_method(mc, *val, "s", vec![])?;
                        if let Value::Object(obj) = result {
                            if let ObjectPayload::String(s) = &obj.borrow().payload {
                                parts.push(s.to_string());
                                continue;
                            }
                        }
                        parts.push(format!("{}", result))
                    }
                    Ok::<Vec<String>, BBError>(parts)
                })
                .map_err(|e| BBError::Other(e))??;

            Ok(vm.new_string(mc, format!("#({})", parts.join(" "))))
        })
        .instance_method("==:", |vm, mc, args| {
            let lhs_vec = args[0].with_native_state::<NativeListState, _, _>(|l| l.get_vec().to_vec())?;
            let rhs_vec_res = args[1].with_native_state::<NativeListState, _, _>(|l| l.get_vec().to_vec());
            let rhs_vec = match rhs_vec_res {
                Ok(v) => v,
                Err(_) => return Ok(vm.new_bool(mc, false)),
            };

            if lhs_vec.len() != rhs_vec.len() {
                return Ok(vm.new_bool(mc, false));
            }

            for (i, &lhs_val) in lhs_vec.iter().enumerate() {
                let rhs_val = rhs_vec[i];
                let eq_res = vm.call_method(mc, lhs_val, "==:", vec![rhs_val])?.is_true();
                if !eq_res {
                    return Ok(vm.new_bool(mc, false));
                }
            }

            Ok(vm.new_bool(mc, true))
        })
        .instance_method("sort", |vm, mc, args| {
            let mut vec = args[0].with_native_state::<NativeListState, _, _>(|l| l.get_vec().to_vec()).map_err(|e| BBError::Other(e))?;
            
            for i in 1..vec.len() {
                let mut j = i;
                while j > 0 {
                    let gt_res = if vec[j-1].is_nil() {
                        !vec[j].is_nil()
                    } else if vec[j].is_nil() {
                        false
                    } else {
                        vm.call_method(mc, vec[j-1], ">", vec![vec[j]])?.is_true()
                    };
                    if gt_res {
                        vec.swap(j-1, j);
                        j -= 1;
                    } else {
                        break;
                    }
                }
            }
            
            args[0].with_native_state_mut(mc, |l: &mut NativeListState| {
                *l.get_vec_mut() = vec;
            }).map_err(|e| BBError::Other(e))?;
            
            Ok(args[0])
        })
        .instance_method("sort:", |vm, mc, args| {
            let block_gc = arg!(args, Block, 1);
            let mut vec = args[0].with_native_state::<NativeListState, _, _>(|l| l.get_vec().to_vec()).map_err(|e| BBError::Other(e))?;
            
            let arity = block_gc.param_names.len();
            if arity == 1 {
                for i in 1..vec.len() {
                    let mut j = i;
                    while j > 0 {
                        let key_lhs = vm.call_method(mc, args[1], "valueWithArgs:", vec![vm.new_list(mc, vec![vec[j-1]])])?;
                        let key_rhs = vm.call_method(mc, args[1], "valueWithArgs:", vec![vm.new_list(mc, vec![vec[j]])])?;
                        let gt_res = if key_lhs.is_nil() {
                            !key_rhs.is_nil()
                        } else if key_rhs.is_nil() {
                            false
                        } else {
                            vm.call_method(mc, key_lhs, ">", vec![key_rhs])?.is_true()
                        };
                        if gt_res {
                            vec.swap(j-1, j);
                            j -= 1;
                        } else {
                            break;
                        }
                    }
                }
            } else {
                for i in 1..vec.len() {
                    let mut j = i;
                    while j > 0 {
                        let res = vm.call_method(mc, args[1], "valueWithArgs:", vec![vm.new_list(mc, vec![vec[j-1], vec[j]])])?;
                        if !res.is_true() {
                            vec.swap(j-1, j);
                            j -= 1;
                        } else {
                            break;
                        }
                    }
                }
            }
            
            args[0].with_native_state_mut(mc, |l: &mut NativeListState| {
                *l.get_vec_mut() = vec;
            }).map_err(|e| BBError::Other(e))?;
            
            Ok(args[0])
        })
}
