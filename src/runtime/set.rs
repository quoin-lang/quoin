use crate::error::QuoinError;
use crate::value::{AnyCollect, NativeClassBuilder, ObjectPayload, Value};

use gc_arena::collect::{DynCollect, Trace};
use std::any::Any;
use std::mem::transmute;

/// An insertion-ordered set of unique values. Uniqueness is determined by the Quoin
/// `==:` method (so it matches `List#uniq` semantics), which means membership is
/// O(n); this is a simple reference implementation rather than a hashed set.
#[derive(Debug)]
pub struct NativeSetState {
    pub vec: Vec<Value<'static>>,
}

impl NativeSetState {
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

impl AnyCollect for NativeSetState {
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

pub fn build_set_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Set", Some("Object"))
        .instance_method("count", |vm, mc, args| {
            let len = args[0]
                .with_native_state::<NativeSetState, _, _>(|s| s.get_vec().len())
                .map_err(|e| QuoinError::Other(e))?;
            Ok(vm.new_int(mc, len as i64))
        })
        .instance_method("add:", |vm, mc, args| {
            vm.set_add(mc, args[0], args[1])?;
            Ok(args[0])
        })
        .instance_method("remove:", |vm, mc, args| {
            vm.set_remove(mc, args[0], args[1])?;
            Ok(args[0])
        })
        .instance_method("contains?:", |vm, mc, args| {
            let found = vm.set_contains(mc, args[0], args[1])?;
            Ok(vm.new_bool(mc, found))
        })
        .instance_method("each:", |vm, mc, args| {
            let len = args[0]
                .with_native_state::<NativeSetState, _, _>(|s| s.get_vec().len())
                .map_err(|e| QuoinError::Other(e))?;
            for i in 0..len {
                let elem = args[0]
                    .with_native_state::<NativeSetState, _, _>(|s| s.get_vec().get(i).copied())
                    .map_err(|e| QuoinError::Other(e))?;
                if let Some(elem) = elem {
                    vm.call_method(mc, args[1], "valueWithSelfOrArg:", vec![elem])?;
                }
            }
            Ok(args[0])
        })
        .instance_method("s", |vm, mc, args| {
            let len = args[0]
                .with_native_state::<NativeSetState, _, _>(|s| s.get_vec().len())
                .map_err(|e| QuoinError::Other(e))?;

            let mut parts = Vec::new();
            for i in 0..len {
                let val = args[0]
                    .with_native_state::<NativeSetState, _, _>(|s| s.get_vec().get(i).copied())
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

            Ok(vm.new_string(mc, format!("#<{}>", parts.join(" "))))
        })
        .instance_method("==:", |vm, mc, args| {
            let lhs_len = args[0]
                .with_native_state::<NativeSetState, _, _>(|s| s.get_vec().len())
                .map_err(|e| QuoinError::Other(e))?;
            let rhs_len = match args[1].with_native_state::<NativeSetState, _, _>(|s| {
                s.get_vec().len()
            }) {
                Ok(len) => len,
                Err(_) => return Ok(vm.new_bool(mc, false)),
            };

            if lhs_len != rhs_len {
                return Ok(vm.new_bool(mc, false));
            }

            for i in 0..lhs_len {
                let elem = args[0]
                    .with_native_state::<NativeSetState, _, _>(|s| s.get_vec()[i])
                    .map_err(|e| QuoinError::Other(e))?;
                if !vm.set_contains(mc, args[1], elem)? {
                    return Ok(vm.new_bool(mc, false));
                }
            }

            Ok(vm.new_bool(mc, true))
        })
}
