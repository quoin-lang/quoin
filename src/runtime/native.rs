use crate::error::BBError;
use crate::runtime::regex::NativeRegexState;
use crate::value::{Class, NamespacedName, NativeFunc, ObjectPayload, Value};
use crate::vm::VmState;

use gc_arena::lock::RefLock;
use gc_arena::{Gc, Mutation};

// Native helper: print
pub fn native_print<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, BBError> {
    if args.len() > 1 {
        for (i, arg) in args[1..].iter().enumerate() {
            if i > 0 {
                print!(" ");
            }
            let s = vm.call_method(mc, *arg, "s", vec![])?;
            print!("{}", s);
        }
    }
    println!();
    Ok(vm.new_nil(mc))
}

// Native helper: regex_match
pub fn native_regex_match<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, BBError> {
    if args.len() != 2 {
        return Err(BBError::ArgumentCountMismatch {
            expected: 2,
            got: args.len(),
            msg: "regex_match expects exactly 2 arguments (regex, string)".to_string(),
        });
    }
    let re_val = args[0];
    let str_val = args[1];

    let matched = re_val
        .with_native_state::<NativeRegexState, _, _>(|r| {
            if let Value::Object(obj) = str_val
                && let ObjectPayload::String(s) = &obj.borrow().payload
            {
                Ok(r.regex.is_match(&**s))
            } else {
                Err(BBError::TypeError {
                    expected: "String".to_string(),
                    got: str_val.type_name().to_string(),
                    msg: "regex_match expects a String as the second argument".to_string(),
                })
            }
        })
        .map_err(|e| BBError::Other(e))??;

    Ok(vm.new_bool(mc, matched))
}

fn is_instance_of<'gc>(
    vm: &VmState<'gc>,
    val: Value<'gc>,
    class_obj: Gc<'gc, RefLock<Class<'gc>>>,
) -> bool {
    if let Some(val_class) = vm.get_class_for_lookup(val) {
        let mut curr = Some(val_class);
        while let Some(clz) = curr {
            if Gc::ptr_eq(clz, class_obj) {
                return true;
            }
            for mixin in &clz.borrow().mixin_classes {
                if Gc::ptr_eq(*mixin, class_obj) {
                    return true;
                }
            }
            curr = clz.borrow().parent;
        }
    }
    false
}

// Native helper: match (tilde ~ operator)
pub fn native_match<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, BBError> {
    if args.len() != 2 {
        return Err(BBError::ArgumentCountMismatch {
            expected: 2,
            got: args.len(),
            msg: "match expects 2 arguments".to_string(),
        });
    }

    let has_custom_tilde = if let Some(class_obj) = vm.get_class_for_lookup(args[0]) {
        if let Some(method_val) = vm.lookup_in_class_hierarchy(class_obj, "~:", false) {
            let object_key = NamespacedName::new(Vec::new(), "Object".to_string());
            let default_tilde = if let Some(Value::Class(object_class)) =
                vm.globals.borrow().get(&object_key).copied()
            {
                vm.lookup_in_class_hierarchy(object_class, "~:", false)
            } else {
                None
            };
            Some(method_val) != default_tilde
        } else {
            false
        }
    } else {
        false
    };

    if has_custom_tilde {
        let (lhs, rhs) = (args[0], args[1]);
        return vm.call_method(mc, lhs, "~:", vec![rhs]);
    }

    let active_args = vm.active_native_args.last().unwrap();

    if let Value::Object(obj) = active_args[0]
        && let ObjectPayload::Block(block_gc) = &obj.borrow().payload
    {
        let block_gc = *block_gc;
        let active_args = vm.active_native_args.last().unwrap();
        let rhs = active_args[1];
        let self_val = if block_gc.param_names.len() == 0 {
            Some(rhs)
        } else {
            None
        };
        let block_args = if block_gc.param_names.len() == 0 {
            Vec::new()
        } else {
            vec![rhs]
        };
        let res = vm.execute_block(mc, block_gc, block_args, self_val)?;
        return Ok(res);
    }

    let active_args = vm.active_native_args.last().unwrap();

    if let Value::Object(obj) = active_args[1]
        && let ObjectPayload::Block(block_gc) = &obj.borrow().payload
    {
        let block_gc = *block_gc;
        let active_args = vm.active_native_args.last().unwrap();
        let lhs = active_args[0];
        let self_val = if block_gc.param_names.len() == 0 {
            Some(lhs)
        } else {
            None
        };
        let block_args = if block_gc.param_names.len() == 0 {
            Vec::new()
        } else {
            vec![lhs]
        };
        let res = vm.execute_block(mc, block_gc, block_args, self_val)?;
        return Ok(res);
    }

    let active_args = vm.active_native_args.last().unwrap();
    let (lhs, rhs) = (active_args[0], active_args[1]);

    if let Value::Class(c) = lhs {
        return Ok(vm.new_bool(mc, is_instance_of(vm, rhs, c)));
    }
    if let Value::Class(c) = rhs {
        return Ok(vm.new_bool(mc, is_instance_of(vm, lhs, c)));
    }
    if let Value::ClassMeta(c) = lhs {
        return Ok(vm.new_bool(mc, is_instance_of(vm, rhs, c)));
    }
    if let Value::ClassMeta(c) = rhs {
        return Ok(vm.new_bool(mc, is_instance_of(vm, lhs, c)));
    }

    if let Ok(matched) = lhs.with_native_state::<NativeRegexState, _, _>(|r| {
        if let Value::Object(o2) = rhs
            && let ObjectPayload::String(s) = &o2.borrow().payload
        {
            r.regex.is_match(&**s)
        } else {
            false
        }
    }) {
        return Ok(vm.new_bool(mc, matched));
    }
    if let Ok(_) = rhs.with_native_state::<NativeRegexState, _, _>(|_| ()) {
        if let Value::Object(o2) = lhs
            && let ObjectPayload::String(_) = &o2.borrow().payload
        {
            return native_match(vm, mc, vec![rhs, lhs]);
        }
    }

    let (lhs, rhs) = {
        let active_args = vm.active_native_args.last().unwrap();
        (active_args[0], active_args[1])
    };

    let eq_val = if vm.lookup_method(mc, lhs, "==:", &[rhs])?.is_some() {
        let active_args = vm.active_native_args.last().unwrap();
        vm.call_method(mc, active_args[0], "==:", vec![active_args[1]])?
    } else {
        let active_args = vm.active_native_args.last().unwrap();
        vm.new_bool(mc, active_args[0] == active_args[1])
    };
    Ok(eq_val)
}

pub fn register_native_funcs<'gc>(vm: &mut VmState<'gc>, mc: &Mutation<'gc>) {
    let mut funcs = Vec::new();

    funcs.push((
        "print:".to_string(),
        vm.new_native(mc, NativeFunc(native_print)),
    ));
    funcs.push((
        "print:and:".to_string(),
        vm.new_native(mc, NativeFunc(native_print)),
    ));
    funcs.push((
        "print:and:and:and:".to_string(),
        vm.new_native(mc, NativeFunc(native_print)),
    ));
    funcs.push((
        "regex_match:".to_string(),
        vm.new_native(mc, NativeFunc(native_regex_match)),
    ));

    // Operators. Arithmetic (`+: -: *: /: %:`) and ordering (`<: >: <=: >=:`) are now
    // typed multimethods on the numeric/String classes (with `>:`/`<=:`/`>=:` derived
    // as shared BB on Object), so only `~` (match) remains as a global fallback.
    funcs.push(("~".to_string(), vm.new_native(mc, NativeFunc(native_match))));

    let mut globals = vm.globals.borrow_mut(mc);
    for (name, val) in funcs {
        globals.insert(NamespacedName::new(Vec::new(), name), val);
    }
}
