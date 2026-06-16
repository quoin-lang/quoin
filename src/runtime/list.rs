use crate::value::{NativeClassBuilder, Value, AnyCollect};

#[derive(Debug)]
pub struct NativeListState {
    pub idx: usize,
    pub vec: Vec<Value<'static>>,
}

impl NativeListState {
    pub fn new(vec: Vec<Value<'_>>) -> Self {
        let vec_static: Vec<Value<'static>> = unsafe { std::mem::transmute(vec) };
        Self { idx: 0, vec: vec_static }
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
        .instance_method("at:", |vm, mc, args| {
            let idx = match args[1] {
                Value::Object(obj) => match &obj.borrow().payload {
                    crate::value::ObjectPayload::Int(i) => *i,
                    _ => {
                        return Err(crate::error::BBError::TypeError {
                            expected: "Integer".to_string(),
                            got: args[1].type_name().to_string(),
                            msg: "at expects integer index".to_string(),
                        });
                    }
                }
                _ => {
                    return Err(crate::error::BBError::TypeError {
                        expected: "Integer".to_string(),
                        got: args[1].type_name().to_string(),
                        msg: "at expects integer index".to_string(),
                    });
                }
            };
            args[0].with_native_state::<NativeListState, _, _>(|l| {
                let vec = l.get_vec();
                if idx >= 0 && idx < vec.len() as i64 {
                    Ok(vec[idx as usize])
                } else {
                    Ok(vm.new_nil(mc))
                }
            })
            .map_err(|e| crate::error::BBError::Other(e))?
        })
        .instance_method("sliceFrom:", |vm, mc, args| {
            let idx = match args[1] {
                Value::Object(obj) => match &obj.borrow().payload {
                    crate::value::ObjectPayload::Int(i) => *i,
                    _ => {
                        return Err(crate::error::BBError::TypeError {
                            expected: "Integer".to_string(),
                            got: args[1].type_name().to_string(),
                            msg: "sliceFrom expects integer index".to_string(),
                        });
                    }
                }
                _ => {
                    return Err(crate::error::BBError::TypeError {
                        expected: "Integer".to_string(),
                        got: args[1].type_name().to_string(),
                        msg: "sliceFrom expects integer index".to_string(),
                    });
                }
            };
            args[0].with_native_state::<NativeListState, _, _>(|l| {
                let vec = l.get_vec();
                let start = idx.max(0) as usize;
                let sliced = if start < vec.len() {
                    vec[start..].to_vec()
                } else {
                    Vec::new()
                };
                Ok(vm.new_list(mc, sliced))
            })
            .map_err(|e| crate::error::BBError::Other(e))?
        })
}
