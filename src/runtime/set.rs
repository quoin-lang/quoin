use crate::error::QuoinError;
use crate::ext_sdk::{Host, HostExt};
use crate::runtime::elem_tag::{ElemTag, check_insert};
use crate::runtime::pretty::{PpShape, PrettyPrint};
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
    /// Checked element type (docs/GENERICS_ARCH.md). `None` = untagged.
    pub elem: Option<ElemTag>,
}

impl NativeSetState {
    pub fn new(vec: Vec<Value<'_>>) -> Self {
        let vec_static: Vec<Value<'static>> = unsafe { transmute(vec) };
        Self {
            vec: vec_static,
            elem: None,
        }
    }

    pub fn get_vec<'gc>(&self) -> &[Value<'gc>] {
        unsafe { transmute(&self.vec[..]) }
    }

    pub fn get_vec_mut<'gc>(&mut self) -> &mut Vec<Value<'gc>> {
        unsafe { transmute(&mut self.vec) }
    }
}

impl PrettyPrint for NativeSetState {
    fn pp_shape<'gc>(&self) -> PpShape<'gc> {
        PpShape::Seq {
            open: "#<",
            close: ">",
            items: self.get_vec().to_vec(),
        }
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
        .sdk_instance_method("count", |host, receiver, _args| {
            let len = receiver
                .with_native_state::<NativeSetState, _, _>(|s| s.get_vec().len())
                .map_err(|e| QuoinError::Other(e))?;
            Ok(host.new_int(len as i64))
        })
        .returns("Integer")
        .sdk_instance_method("add:", |host, receiver, args| {
            let tag = receiver
                .with_native_state::<NativeSetState, _, _>(|s| s.elem)
                .map_err(QuoinError::Other)?;
            check_insert(tag, "Set", &args[0], None, |v, n| {
                host.value_matches_type(*v, n)
            })?;
            set_add(host, receiver, args[0])?;
            Ok(receiver)
        })
        // --- checked generics (docs/GENERICS_ARCH.md §4.2/§6) ---
        .sdk_class_method("of:", |host, _receiver, args| {
            let tag = ElemTag::from_class_value(&args[0]).ok_or_else(|| QuoinError::TypeError {
                expected: "Class".to_string(),
                got: args[0].type_name().to_string(),
                msg: "Set.of: expects an element class (e.g. `Set.of:String`)".to_string(),
            })?;
            let v = host.new_native_state(
                host.get_or_create_builtin_class("Set"),
                NativeSetState::new(Vec::new()),
            );
            host.with_native_state_mut(v, |s: &mut NativeSetState| s.elem = Some(tag));
            Ok(v)
        })
        .sdk_instance_method("ensure:", |host, receiver, args| {
            let tag = ElemTag::from_class_value(&args[0]).ok_or_else(|| QuoinError::TypeError {
                expected: "Class".to_string(),
                got: args[0].type_name().to_string(),
                msg: "ensure: expects an element class (e.g. `s.ensure:String`)".to_string(),
            })?;
            let vec: Vec<Value> = receiver
                .with_native_state::<NativeSetState, _, _>(|s| s.get_vec().to_vec())
                .map_err(QuoinError::Other)?;
            for (i, v) in vec.iter().enumerate() {
                check_insert(Some(tag), "Set", v, Some(i as i64), |v, n| {
                    host.value_matches_type(*v, n)
                })?;
            }
            let v = host.new_native_state(
                host.get_or_create_builtin_class("Set"),
                NativeSetState::new(vec),
            );
            host.with_native_state_mut(v, |s: &mut NativeSetState| s.elem = Some(tag));
            Ok(v)
        })
        .sdk_instance_method("elementType", |host, receiver, _args| {
            let tag = receiver
                .with_native_state::<NativeSetState, _, _>(|s| s.elem)
                .map_err(QuoinError::Other)?;
            Ok(match tag {
                Some(t) => host.new_symbol(t.name().to_string()),
                None => host.new_nil(),
            })
        })
        .sdk_instance_method("remove:", |host, receiver, args| {
            set_remove(host, receiver, args[0])?;
            Ok(receiver)
        })
        .sdk_instance_method("contains?:", |host, receiver, args| {
            let found = set_contains(host, receiver, args[0])?;
            Ok(host.new_bool(found))
        })
        .returns("Boolean")
        .sdk_instance_method("each:", |host, receiver, args| {
            let len = receiver
                .with_native_state::<NativeSetState, _, _>(|s| s.get_vec().len())
                .map_err(|e| QuoinError::Other(e))?;
            for i in 0..len {
                let elem = receiver
                    .with_native_state::<NativeSetState, _, _>(|s| s.get_vec().get(i).copied())
                    .map_err(|e| QuoinError::Other(e))?;
                if let Some(elem) = elem {
                    host.call_method(args[0], "valueWithSelfOrArg:", vec![elem])?;
                }
            }
            Ok(receiver)
        })
        .sdk_instance_method("s", |host, receiver, _args| {
            let len = receiver
                .with_native_state::<NativeSetState, _, _>(|s| s.get_vec().len())
                .map_err(|e| QuoinError::Other(e))?;

            let mut parts = Vec::new();
            for i in 0..len {
                let val = receiver
                    .with_native_state::<NativeSetState, _, _>(|s| s.get_vec().get(i).copied())
                    .map_err(|e| QuoinError::Other(e))?
                    .ok_or_else(|| QuoinError::Other("Index out of bounds".to_string()))?;

                let result = host.call_method(val, "s", vec![])?;
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

            Ok(host.new_string(format!("#<{}>", parts.join(" "))))
        })
        .sdk_instance_method("==:", |host, receiver, args| {
            let lhs_len = receiver
                .with_native_state::<NativeSetState, _, _>(|s| s.get_vec().len())
                .map_err(|e| QuoinError::Other(e))?;
            let rhs_len =
                match args[0].with_native_state::<NativeSetState, _, _>(|s| s.get_vec().len()) {
                    Ok(len) => len,
                    Err(_) => return Ok(host.new_bool(false)),
                };

            if lhs_len != rhs_len {
                return Ok(host.new_bool(false));
            }

            for i in 0..lhs_len {
                let elem = receiver
                    .with_native_state::<NativeSetState, _, _>(|s| s.get_vec()[i])
                    .map_err(|e| QuoinError::Other(e))?;
                if !set_contains(host, args[0], elem)? {
                    return Ok(host.new_bool(false));
                }
            }

            Ok(host.new_bool(true))
        })
}

/// Whether `set_val` already holds an element equal (by Quoin `==:`) to `value`.
/// Membership is O(n) — `NativeSetState` is a simple reference impl, not hashed.
fn set_contains<'gc>(
    host: &mut dyn Host<'gc>,
    set_val: Value<'gc>,
    value: Value<'gc>,
) -> Result<bool, QuoinError> {
    let len = set_val
        .with_native_state::<NativeSetState, _, _>(|s| s.get_vec().len())
        .map_err(QuoinError::Other)?;
    for i in 0..len {
        let elem = set_val
            .with_native_state::<NativeSetState, _, _>(|s| s.get_vec()[i])
            .map_err(QuoinError::Other)?;
        if host.call_method(elem, "==:", vec![value])?.is_true() {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Insert `value` unless an equal element is already present; returns whether a new
/// element was added.
fn set_add<'gc>(
    host: &mut dyn Host<'gc>,
    set_val: Value<'gc>,
    value: Value<'gc>,
) -> Result<bool, QuoinError> {
    if set_contains(host, set_val, value)? {
        Ok(false)
    } else {
        host.with_native_state_mut::<NativeSetState, _>(set_val, |s| s.get_vec_mut().push(value))
            .map_err(QuoinError::Other)?;
        Ok(true)
    }
}

/// Remove the first element equal (by `==:`) to `value`; returns whether one was removed.
fn set_remove<'gc>(
    host: &mut dyn Host<'gc>,
    set_val: Value<'gc>,
    value: Value<'gc>,
) -> Result<bool, QuoinError> {
    let len = set_val
        .with_native_state::<NativeSetState, _, _>(|s| s.get_vec().len())
        .map_err(QuoinError::Other)?;
    for i in 0..len {
        let elem = set_val
            .with_native_state::<NativeSetState, _, _>(|s| s.get_vec()[i])
            .map_err(QuoinError::Other)?;
        if host.call_method(elem, "==:", vec![value])?.is_true() {
            host.with_native_state_mut::<NativeSetState, _>(set_val, |s| {
                s.get_vec_mut().remove(i);
            })
            .map_err(QuoinError::Other)?;
            return Ok(true);
        }
    }
    Ok(false)
}
