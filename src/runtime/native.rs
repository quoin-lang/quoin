use crate::error::BBError;
use crate::runtime::list::NativeListState;
use crate::runtime::map::NativeMapState;
use crate::runtime::regex::NativeRegexState;
use crate::value::{Class, NamespacedName, NativeFunc, ObjectPayload, Value};
use crate::vm::VmState;

use gc_arena::Mutation;

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
    class_obj: gc_arena::Gc<'gc, gc_arena::lock::RefLock<Class<'gc>>>,
) -> bool {
    if let Some(val_class) = vm.get_class_for_lookup(val) {
        let mut curr = Some(val_class);
        while let Some(clz) = curr {
            if gc_arena::Gc::ptr_eq(clz, class_obj) {
                return true;
            }
            for mixin in &clz.borrow().mixin_classes {
                if gc_arena::Gc::ptr_eq(*mixin, class_obj) {
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

    let (lhs, rhs) = (args[0], args[1]);

    let has_custom_tilde = if let Some(class_obj) = vm.get_class_for_lookup(lhs) {
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
        return vm.call_method(mc, lhs, "~:", args[1..].to_vec());
    }

    if let Value::Object(obj) = lhs
        && let ObjectPayload::Block(block_gc) = &obj.borrow().payload
    {
        let block_gc = *block_gc;
        let res = if block_gc.param_names.len() == 0 {
            vm.execute_block(mc, block_gc, Vec::new(), Some(rhs))?
        } else {
            vm.execute_block(mc, block_gc, vec![rhs], None)?
        };
        return Ok(res);
    }

    if let Value::Object(obj) = rhs
        && let ObjectPayload::Block(block_gc) = &obj.borrow().payload
    {
        let block_gc = *block_gc;
        let res = if block_gc.param_names.len() == 0 {
            vm.execute_block(mc, block_gc, Vec::new(), Some(lhs))?
        } else {
            vm.execute_block(mc, block_gc, vec![lhs], None)?
        };
        return Ok(res);
    }

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

    let eq_val = if vm.lookup_method(mc, lhs, "==:", &[rhs])?.is_some() {
        vm.call_method(mc, lhs, "==:", vec![rhs])?
    } else {
        vm.new_bool(mc, lhs == rhs)
    };
    Ok(eq_val)
}

// Native helper: add
pub fn native_add<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, BBError> {
    if args.len() != 2 {
        return Err(BBError::ArgumentCountMismatch {
            expected: 2,
            got: args.len(),
            msg: "add expects 2 arguments".to_string(),
        });
    }

    let receiver = args[0];
    if vm.lookup_method(mc, receiver, "+:", &args[1..])?.is_some() {
        return vm.call_method(mc, receiver, "+:", args[1..].to_vec());
    }

    let (p0, p1) = match (&args[0], &args[1]) {
        (Value::Object(o1), Value::Object(o2)) => (&o1.borrow().payload, &o2.borrow().payload),
        _ => {
            return Err(BBError::TypeError {
                expected: "Object".to_string(),
                got: format!("{:?} and {:?}", args[0], args[1]),
                msg: "add expected objects".to_string(),
            });
        }
    };
    match (p0, p1) {
        (ObjectPayload::Int(a), ObjectPayload::Int(b)) => Ok(vm.new_int(mc, a + b)),
        (ObjectPayload::Double(a), ObjectPayload::Double(b)) => Ok(vm.new_double(mc, a + b)),
        (ObjectPayload::Int(a), ObjectPayload::Double(b)) => Ok(vm.new_double(mc, *a as f64 + b)),
        (ObjectPayload::Double(a), ObjectPayload::Int(b)) => Ok(vm.new_double(mc, a + *b as f64)),
        (ObjectPayload::String(a), ObjectPayload::String(b)) => {
            let new_str = format!("{}{}", **a, **b);
            Ok(vm.new_string(mc, new_str))
        }
        _ => Err(BBError::TypeError {
            expected: "numeric or compatible types".to_string(),
            got: format!("{:?} and {:?}", args[0], args[1]),
            msg: format!("Cannot add {:?} and {:?}", args[0], args[1]),
        }),
    }
}

// Native helper: sub
pub fn native_sub<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, BBError> {
    if args.len() != 2 {
        return Err(BBError::ArgumentCountMismatch {
            expected: 2,
            got: args.len(),
            msg: "sub expects 2 arguments".to_string(),
        });
    }

    let receiver = args[0];
    if vm.lookup_method(mc, receiver, "-:", &args[1..])?.is_some() {
        return vm.call_method(mc, receiver, "-:", args[1..].to_vec());
    }

    let (p0, p1) = match (&args[0], &args[1]) {
        (Value::Object(o1), Value::Object(o2)) => (&o1.borrow().payload, &o2.borrow().payload),
        _ => {
            return Err(BBError::TypeError {
                expected: "Object".to_string(),
                got: format!("{:?} and {:?}", args[0], args[1]),
                msg: "sub expected objects".to_string(),
            });
        }
    };
    match (p0, p1) {
        (ObjectPayload::Int(a), ObjectPayload::Int(b)) => Ok(vm.new_int(mc, a - b)),
        (ObjectPayload::Double(a), ObjectPayload::Double(b)) => Ok(vm.new_double(mc, a - b)),
        (ObjectPayload::Int(a), ObjectPayload::Double(b)) => Ok(vm.new_double(mc, *a as f64 - b)),
        (ObjectPayload::Double(a), ObjectPayload::Int(b)) => Ok(vm.new_double(mc, a - *b as f64)),
        _ => Err(BBError::TypeError {
            expected: "numeric types".to_string(),
            got: format!("{:?} and {:?}", args[0], args[1]),
            msg: format!("Cannot subtract {:?} and {:?}", args[0], args[1]),
        }),
    }
}

// Native helper: mul
pub fn native_mul<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, BBError> {
    if args.len() != 2 {
        return Err(BBError::ArgumentCountMismatch {
            expected: 2,
            got: args.len(),
            msg: "mul expects 2 arguments".to_string(),
        });
    }

    let receiver = args[0];
    if vm.lookup_method(mc, receiver, "*:", &args[1..])?.is_some() {
        return vm.call_method(mc, receiver, "*:", args[1..].to_vec());
    }

    let (p0, p1) = match (&args[0], &args[1]) {
        (Value::Object(o1), Value::Object(o2)) => (&o1.borrow().payload, &o2.borrow().payload),
        _ => {
            return Err(BBError::TypeError {
                expected: "Object".to_string(),
                got: format!("{:?} and {:?}", args[0], args[1]),
                msg: "mul expected objects".to_string(),
            });
        }
    };
    match (p0, p1) {
        (ObjectPayload::Int(a), ObjectPayload::Int(b)) => Ok(vm.new_int(mc, a * b)),
        (ObjectPayload::Double(a), ObjectPayload::Double(b)) => Ok(vm.new_double(mc, a * b)),
        (ObjectPayload::Int(a), ObjectPayload::Double(b)) => Ok(vm.new_double(mc, *a as f64 * b)),
        (ObjectPayload::Double(a), ObjectPayload::Int(b)) => Ok(vm.new_double(mc, a * *b as f64)),
        _ => Err(BBError::TypeError {
            expected: "numeric types".to_string(),
            got: format!("{:?} and {:?}", args[0], args[1]),
            msg: format!("Cannot multiply {:?} and {:?}", args[0], args[1]),
        }),
    }
}

// Native helper: div
pub fn native_div<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, BBError> {
    if args.len() != 2 {
        return Err(BBError::ArgumentCountMismatch {
            expected: 2,
            got: args.len(),
            msg: "div expects 2 arguments".to_string(),
        });
    }

    let receiver = args[0];
    if vm.lookup_method(mc, receiver, "/:", &args[1..])?.is_some() {
        return vm.call_method(mc, receiver, "/:", args[1..].to_vec());
    }

    let (p0, p1) = match (&args[0], &args[1]) {
        (Value::Object(o1), Value::Object(o2)) => (&o1.borrow().payload, &o2.borrow().payload),
        _ => {
            return Err(BBError::TypeError {
                expected: "Object".to_string(),
                got: format!("{:?} and {:?}", args[0], args[1]),
                msg: "div expected objects".to_string(),
            });
        }
    };
    match (p0, p1) {
        (ObjectPayload::Int(a), ObjectPayload::Int(b)) => {
            if *b == 0 {
                return Err(BBError::ArithmeticError("Division by zero".to_string()));
            }
            Ok(vm.new_int(mc, a / b))
        }
        (ObjectPayload::Double(a), ObjectPayload::Double(b)) => Ok(vm.new_double(mc, a / b)),
        (ObjectPayload::Int(a), ObjectPayload::Double(b)) => Ok(vm.new_double(mc, *a as f64 / b)),
        (ObjectPayload::Double(a), ObjectPayload::Int(b)) => Ok(vm.new_double(mc, a / *b as f64)),
        _ => Err(BBError::TypeError {
            expected: "numeric types".to_string(),
            got: format!("{:?} and {:?}", args[0], args[1]),
            msg: format!("Cannot divide {:?} and {:?}", args[0], args[1]),
        }),
    }
}

// Native helper: mod
pub fn native_mod<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, BBError> {
    if args.len() != 2 {
        return Err(BBError::ArgumentCountMismatch {
            expected: 2,
            got: args.len(),
            msg: "mod expects 2 arguments".to_string(),
        });
    }

    let receiver = args[0];
    if vm.lookup_method(mc, receiver, "%:", &args[1..])?.is_some() {
        return vm.call_method(mc, receiver, "%:", args[1..].to_vec());
    }

    let (p0, p1) = match (&args[0], &args[1]) {
        (Value::Object(o1), Value::Object(o2)) => (&o1.borrow().payload, &o2.borrow().payload),
        _ => {
            return Err(BBError::TypeError {
                expected: "Object".to_string(),
                got: format!("{:?} and {:?}", args[0], args[1]),
                msg: "mod expected objects".to_string(),
            });
        }
    };
    match (p0, p1) {
        (ObjectPayload::Int(a), ObjectPayload::Int(b)) => {
            if *b == 0 {
                return Err(BBError::ArithmeticError("Division by zero".to_string()));
            }
            Ok(vm.new_int(mc, a % b))
        }
        (ObjectPayload::Double(a), ObjectPayload::Double(b)) => Ok(vm.new_double(mc, a % b)),
        (ObjectPayload::Int(a), ObjectPayload::Double(b)) => Ok(vm.new_double(mc, *a as f64 % b)),
        (ObjectPayload::Double(a), ObjectPayload::Int(b)) => Ok(vm.new_double(mc, a % *b as f64)),
        (ObjectPayload::String(s), other) => {
            let mut format_args = Vec::new();
            match other {
                ObjectPayload::NativeState(state_cell) => {
                    let state_ref = state_cell.borrow();
                    if let Some(list_state) = state_ref.as_any().downcast_ref::<NativeListState>() {
                        for val in list_state.get_vec() {
                            format_args.push(*val);
                        }
                    } else {
                        format_args.push(args[1]);
                    }
                }
                _ => {
                    format_args.push(args[1]);
                }
            }

            let mut result = String::new();
            let s_str = s.to_string();
            let mut chars = s_str.chars().peekable();
            let mut arg_idx = 0;

            while let Some(c) = chars.next() {
                if c == '%' {
                    if let Some(&next_c) = chars.peek() {
                        if next_c.is_digit(10) {
                            let mut num_str = String::new();
                            while let Some(&digit) = chars.peek() {
                                if digit.is_digit(10) {
                                    num_str.push(digit);
                                    chars.next();
                                } else {
                                    break;
                                }
                            }
                            let idx: usize = num_str.parse().unwrap();
                            if idx > 0 && idx <= format_args.len() {
                                let val = format_args[idx - 1];
                                let val_str_val = vm.call_method(mc, val, "s", vec![])?;
                                let val_str = match val_str_val {
                                    Value::Object(o) => match &o.borrow().payload {
                                        ObjectPayload::String(st) => st.to_string(),
                                        _ => format!("{}", val_str_val),
                                    },
                                    _ => format!("{}", val_str_val),
                                };
                                result.push_str(&val_str);
                            }
                        } else if next_c.is_alphabetic()
                            && let Value::Object(obj) = args[1]
                            && let ObjectPayload::NativeState(state_cell) = &obj.borrow().payload
                            && state_cell.borrow().as_any().is::<NativeMapState>()
                        {
                            let key_char = next_c;
                            chars.next(); // Consume the character

                            let mut resolved_val = None;
                            let state_ref = state_cell.borrow();
                            if let Some(map_state) =
                                state_ref.as_any().downcast_ref::<NativeMapState>()
                            {
                                let key_str = key_char.to_string();
                                if let Some(val) = map_state.get_map().get(&key_str).copied() {
                                    resolved_val = Some(val);
                                }
                            }

                            if let Some(val) = resolved_val {
                                let val_str_val = vm.call_method(mc, val, "s", vec![])?;
                                let val_str = match val_str_val {
                                    Value::Object(o) => match &o.borrow().payload {
                                        ObjectPayload::String(st) => st.to_string(),
                                        _ => format!("{}", val_str_val),
                                    },
                                    _ => format!("{}", val_str_val),
                                };
                                result.push_str(&val_str);
                            } else {
                                result.push('%');
                                result.push(key_char);
                            }
                        } else {
                            if arg_idx < format_args.len() {
                                let val = format_args[arg_idx];
                                arg_idx += 1;
                                let val_str_val = vm.call_method(mc, val, "s", vec![])?;
                                let val_str = match val_str_val {
                                    Value::Object(o) => match &o.borrow().payload {
                                        ObjectPayload::String(st) => st.to_string(),
                                        _ => format!("{}", val_str_val),
                                    },
                                    _ => format!("{}", val_str_val),
                                };
                                result.push_str(&val_str);
                            }
                        }
                    } else {
                        if arg_idx < format_args.len() {
                            let val = format_args[arg_idx];
                            arg_idx += 1;
                            let val_str_val = vm.call_method(mc, val, "s", vec![])?;
                            let val_str = match val_str_val {
                                Value::Object(o) => match &o.borrow().payload {
                                    ObjectPayload::String(st) => st.to_string(),
                                    _ => format!("{}", val_str_val),
                                },
                                _ => format!("{}", val_str_val),
                            };
                            result.push_str(&val_str);
                        } else {
                            result.push('%');
                        }
                    }
                } else {
                    result.push(c);
                }
            }
            Ok(vm.new_string(mc, result))
        }
        _ => Err(BBError::TypeError {
            expected: "numeric or string formatting types".to_string(),
            got: format!("{:?} and {:?}", args[0], args[1]),
            msg: format!("Cannot modulo/format {:?} and {:?}", args[0], args[1]),
        }),
    }
}

// Native helper: eq
pub fn native_eq<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, BBError> {
    if args.len() != 2 {
        return Err(BBError::ArgumentCountMismatch {
            expected: 2,
            got: args.len(),
            msg: "eq expects 2 arguments".to_string(),
        });
    }

    let (receiver, other) = (args[0], args[1]);
    if vm.lookup_method(mc, receiver, "==:", &args[1..])?.is_some() {
        return vm.call_method(mc, receiver, "==:", args[1..].to_vec());
    }

    Ok(vm.new_bool(mc, receiver == other))
}

// Native helper: ne
pub fn native_ne<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, BBError> {
    if args.len() != 2 {
        return Err(BBError::ArgumentCountMismatch {
            expected: 2,
            got: args.len(),
            msg: "ne expects 2 arguments".to_string(),
        });
    }
    let eq_result = native_eq(vm, mc, args)?;
    let false_val = vm.new_bool(mc, false);
    Ok(vm.new_bool(mc, eq_result == false_val))
}

// Native helper: lt
pub fn native_lt<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, BBError> {
    if args.len() != 2 {
        return Err(BBError::ArgumentCountMismatch {
            expected: 2,
            got: args.len(),
            msg: "lt expects 2 arguments".to_string(),
        });
    }

    let receiver = args[0];
    if vm.lookup_method(mc, receiver, "<:", &args[1..])?.is_some() {
        return vm.call_method(mc, receiver, "<:", args[1..].to_vec());
    }

    let (p0, p1) = match (&args[0], &args[1]) {
        (Value::Object(o1), Value::Object(o2)) => (&o1.borrow().payload, &o2.borrow().payload),
        _ => {
            return Err(BBError::TypeError {
                expected: "Object".to_string(),
                got: format!("{:?} and {:?}", args[0], args[1]),
                msg: "lt expected objects".to_string(),
            });
        }
    };
    match (p0, p1) {
        (ObjectPayload::Int(a), ObjectPayload::Int(b)) => Ok(vm.new_bool(mc, a < b)),
        (ObjectPayload::Double(a), ObjectPayload::Double(b)) => Ok(vm.new_bool(mc, a < b)),
        (ObjectPayload::Int(a), ObjectPayload::Double(b)) => Ok(vm.new_bool(mc, (*a as f64) < *b)),
        (ObjectPayload::Double(a), ObjectPayload::Int(b)) => Ok(vm.new_bool(mc, *a < (*b as f64))),
        (ObjectPayload::Bool(a), ObjectPayload::Bool(b)) => Ok(vm.new_bool(mc, *a && !*b)),
        (ObjectPayload::String(a), ObjectPayload::String(b)) => Ok(vm.new_bool(mc, **a < **b)),
        _ => Err(BBError::TypeError {
            expected: "comparable types".to_string(),
            got: format!("{:?} and {:?}", args[0], args[1]),
            msg: format!("Cannot compare {:?} and {:?}", args[0], args[1]),
        }),
    }
}

// Native helper: gt
pub fn native_gt<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, BBError> {
    if args.len() != 2 {
        return Err(BBError::ArgumentCountMismatch {
            expected: 2,
            got: args.len(),
            msg: "gt expects 2 arguments".to_string(),
        });
    }

    let receiver = args[0];
    if vm.lookup_method(mc, receiver, ">:", &args[1..])?.is_some() {
        return vm.call_method(mc, receiver, ">:", args[1..].to_vec());
    }

    let (p0, p1) = match (&args[0], &args[1]) {
        (Value::Object(o1), Value::Object(o2)) => (&o1.borrow().payload, &o2.borrow().payload),
        _ => {
            return Err(BBError::TypeError {
                expected: "Object".to_string(),
                got: format!("{:?} and {:?}", args[0], args[1]),
                msg: "gt expected objects".to_string(),
            });
        }
    };
    match (p0, p1) {
        (ObjectPayload::Int(a), ObjectPayload::Int(b)) => Ok(vm.new_bool(mc, a > b)),
        (ObjectPayload::Double(a), ObjectPayload::Double(b)) => Ok(vm.new_bool(mc, a > b)),
        (ObjectPayload::Int(a), ObjectPayload::Double(b)) => Ok(vm.new_bool(mc, (*a as f64) > *b)),
        (ObjectPayload::Double(a), ObjectPayload::Int(b)) => Ok(vm.new_bool(mc, *a > (*b as f64))),
        (ObjectPayload::Bool(a), ObjectPayload::Bool(b)) => Ok(vm.new_bool(mc, !*a && *b)),
        (ObjectPayload::String(a), ObjectPayload::String(b)) => Ok(vm.new_bool(mc, **a > **b)),
        _ => Err(BBError::TypeError {
            expected: "comparable types".to_string(),
            got: format!("{:?} and {:?}", args[0], args[1]),
            msg: format!("Cannot compare {:?} and {:?}", args[0], args[1]),
        }),
    }
}

// Native helper: le
pub fn native_le<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, BBError> {
    if args.len() != 2 {
        return Err(BBError::ArgumentCountMismatch {
            expected: 2,
            got: args.len(),
            msg: "le expects 2 arguments".to_string(),
        });
    }

    let receiver = args[0];
    if vm.lookup_method(mc, receiver, "<=:", &args[1..])?.is_some() {
        return vm.call_method(mc, receiver, "<=:", args[1..].to_vec());
    }

    let (p0, p1) = match (&args[0], &args[1]) {
        (Value::Object(o1), Value::Object(o2)) => (&o1.borrow().payload, &o2.borrow().payload),
        _ => {
            return Err(BBError::TypeError {
                expected: "Object".to_string(),
                got: format!("{:?} and {:?}", args[0], args[1]),
                msg: "le expected objects".to_string(),
            });
        }
    };
    match (p0, p1) {
        (ObjectPayload::Int(a), ObjectPayload::Int(b)) => Ok(vm.new_bool(mc, a <= b)),
        (ObjectPayload::Double(a), ObjectPayload::Double(b)) => Ok(vm.new_bool(mc, a <= b)),
        (ObjectPayload::Int(a), ObjectPayload::Double(b)) => Ok(vm.new_bool(mc, (*a as f64) <= *b)),
        (ObjectPayload::Double(a), ObjectPayload::Int(b)) => Ok(vm.new_bool(mc, *a <= (*b as f64))),
        (ObjectPayload::Bool(a), ObjectPayload::Bool(b)) => Ok(vm.new_bool(mc, *a || !*b)),
        (ObjectPayload::String(a), ObjectPayload::String(b)) => Ok(vm.new_bool(mc, **a <= **b)),
        _ => Err(BBError::TypeError {
            expected: "comparable types".to_string(),
            got: format!("{:?} and {:?}", args[0], args[1]),
            msg: format!("Cannot compare {:?} and {:?}", args[0], args[1]),
        }),
    }
}

// Native helper: ge
pub fn native_ge<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, BBError> {
    if args.len() != 2 {
        return Err(BBError::ArgumentCountMismatch {
            expected: 2,
            got: args.len(),
            msg: "ge expects 2 arguments".to_string(),
        });
    }

    let receiver = args[0];
    if vm.lookup_method(mc, receiver, ">=:", &args[1..])?.is_some() {
        return vm.call_method(mc, receiver, ">=:", args[1..].to_vec());
    }

    let (p0, p1) = match (&args[0], &args[1]) {
        (Value::Object(o1), Value::Object(o2)) => (&o1.borrow().payload, &o2.borrow().payload),
        _ => {
            return Err(BBError::TypeError {
                expected: "Object".to_string(),
                got: format!("{:?} and {:?}", args[0], args[1]),
                msg: "ge expected objects".to_string(),
            });
        }
    };
    match (p0, p1) {
        (ObjectPayload::Int(a), ObjectPayload::Int(b)) => Ok(vm.new_bool(mc, a >= b)),
        (ObjectPayload::Double(a), ObjectPayload::Double(b)) => Ok(vm.new_bool(mc, a >= b)),
        (ObjectPayload::Int(a), ObjectPayload::Double(b)) => Ok(vm.new_bool(mc, (*a as f64) >= *b)),
        (ObjectPayload::Double(a), ObjectPayload::Int(b)) => Ok(vm.new_bool(mc, *a >= (*b as f64))),
        (ObjectPayload::Bool(a), ObjectPayload::Bool(b)) => Ok(vm.new_bool(mc, !*a || *b)),
        (ObjectPayload::String(a), ObjectPayload::String(b)) => Ok(vm.new_bool(mc, **a >= **b)),
        _ => Err(BBError::TypeError {
            expected: "comparable types".to_string(),
            got: format!("{:?} and {:?}", args[0], args[1]),
            msg: format!("Cannot compare {:?} and {:?}", args[0], args[1]),
        }),
    }
}

// Native helper: logic not
pub fn native_not<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, BBError> {
    if args.len() != 1 {
        return Err(BBError::ArgumentCountMismatch {
            expected: 1,
            got: args.len(),
            msg: "not expects exactly 1 argument (receiver)".to_string(),
        });
    }
    Ok(vm.new_bool(mc, !args[0].is_truthy()))
}

// Native helper: negated
pub fn native_negated<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, BBError> {
    if args.len() != 1 {
        return Err(BBError::ArgumentCountMismatch {
            expected: 1,
            got: args.len(),
            msg: "negated expects exactly 1 argument (receiver)".to_string(),
        });
    }
    let payload = match &args[0] {
        Value::Object(obj) => &obj.borrow().payload,
        _ => {
            return Err(BBError::TypeError {
                expected: "Object".to_string(),
                got: args[0].type_name().to_string(),
                msg: "negated expected an object".to_string(),
            });
        }
    };
    match payload {
        ObjectPayload::Int(i) => Ok(vm.new_int(mc, -*i)),
        ObjectPayload::Double(f) => Ok(vm.new_double(mc, -*f)),
        _ => Err(BBError::TypeError {
            expected: "Integer or Double".to_string(),
            got: args[0].type_name().to_string(),
            msg: format!("negated expects integer or double, got {:?}", args[0]),
        }),
    }
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

    // Operators
    funcs.push(("+".to_string(), vm.new_native(mc, NativeFunc(native_add))));
    funcs.push(("-".to_string(), vm.new_native(mc, NativeFunc(native_sub))));
    funcs.push(("*".to_string(), vm.new_native(mc, NativeFunc(native_mul))));
    funcs.push(("/".to_string(), vm.new_native(mc, NativeFunc(native_div))));
    funcs.push(("%".to_string(), vm.new_native(mc, NativeFunc(native_mod))));
    funcs.push(("==".to_string(), vm.new_native(mc, NativeFunc(native_eq))));
    funcs.push(("!=".to_string(), vm.new_native(mc, NativeFunc(native_ne))));
    funcs.push(("<".to_string(), vm.new_native(mc, NativeFunc(native_lt))));
    funcs.push((">".to_string(), vm.new_native(mc, NativeFunc(native_gt))));
    funcs.push(("<=".to_string(), vm.new_native(mc, NativeFunc(native_le))));
    funcs.push((">=".to_string(), vm.new_native(mc, NativeFunc(native_ge))));
    funcs.push(("~".to_string(), vm.new_native(mc, NativeFunc(native_match))));

    // Unary
    funcs.push(("!".to_string(), vm.new_native(mc, NativeFunc(native_not))));
    funcs.push((
        "negated".to_string(),
        vm.new_native(mc, NativeFunc(native_negated)),
    ));

    let mut globals = vm.globals.borrow_mut(mc);
    for (name, val) in funcs {
        globals.insert(NamespacedName::new(Vec::new(), name), val);
    }
}
