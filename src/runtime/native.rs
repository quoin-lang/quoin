use crate::error::BBError;
use crate::value::{NativeFunc, Value, ObjectPayload, GcRegex};
use crate::vm::VmState;
use crate::gc;

use gc_arena::{Mutation, RefLock};

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
    let (p0, p1) = match (&args[0], &args[1]) {
        (Value::Object(o1), Value::Object(o2)) => (&o1.borrow().payload, &o2.borrow().payload),
        _ => {
            return Err(BBError::TypeError {
                expected: "Object".to_string(),
                got: format!("{:?} and {:?}", args[0], args[1]),
                msg: "regex_match expected objects".to_string(),
            });
        }
    };
    match (p0, p1) {
        (ObjectPayload::Regex(r), ObjectPayload::String(s)) => {
            let matched = r.0.is_match(&**s);
            Ok(vm.new_bool(mc, matched))
        }
        _ => Err(format!(
            "regex_match expects regex and string, got {:?} and {:?}",
            args[0], args[1]
        )
        .into()),
    }
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
    let method = vm.lookup_method(receiver, "==:");
    if let Some(method) = method {
        method.call(vm, mc, args)?;
        return Ok(vm.pop()?);
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

// Native helper: list index lookup (at:)
pub fn native_list_at<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, BBError> {
    if args.len() != 2 {
        return Err(BBError::ArgumentCountMismatch {
            expected: 2,
            got: args.len(),
            msg: "at expects exactly 2 arguments (receiver, index)".to_string(),
        });
    }
    let (p0, p1) = match (&args[0], &args[1]) {
        (Value::Object(o1), Value::Object(o2)) => (&o1.borrow().payload, &o2.borrow().payload),
        _ => {
            return Err(BBError::TypeError {
                expected: "Object".to_string(),
                got: format!("{:?} and {:?}", args[0], args[1]),
                msg: "at expected objects".to_string(),
            });
        }
    };
    match (p0, p1) {
        (ObjectPayload::List(l), ObjectPayload::Int(idx)) => {
            let borrowed = l.borrow();
            let idx = *idx;
            if idx >= 0 && idx < borrowed.len() as i64 {
                Ok(borrowed[idx as usize])
            } else {
                Ok(vm.new_nil(mc))
            }
        }
        _ => Err(format!(
            "at expects list and integer, got {:?} and {:?}",
            args[0], args[1]
        )
        .into()),
    }
}

// Native helper: list sliceFrom:
pub fn native_list_slice_from<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, BBError> {
    if args.len() != 2 {
        return Err(BBError::ArgumentCountMismatch {
            expected: 2,
            got: args.len(),
            msg: "sliceFrom expects exactly 2 arguments (receiver, index)".to_string(),
        });
    }
    let (p0, p1) = match (&args[0], &args[1]) {
        (Value::Object(o1), Value::Object(o2)) => (&o1.borrow().payload, &o2.borrow().payload),
        _ => {
            return Err(BBError::TypeError {
                expected: "Object".to_string(),
                got: format!("{:?} and {:?}", args[0], args[1]),
                msg: "sliceFrom expected objects".to_string(),
            });
        }
    };
    match (p0, p1) {
        (ObjectPayload::List(l), ObjectPayload::Int(idx)) => {
            let borrowed = l.borrow();
            let start = (*idx).max(0) as usize;
            let sliced = if start < borrowed.len() {
                borrowed[start..].to_vec()
            } else {
                Vec::new()
            };
            Ok(vm.new_list(mc, sliced))
        }
        _ => Err(format!(
            "sliceFrom expects list and integer, got {:?} and {:?}",
            args[0], args[1]
        )
        .into()),
    }
}

pub fn register_native_funcs<'gc>(vm: &mut VmState<'gc>, mc: &Mutation<'gc>) {
    let mut funcs = Vec::new();
    
    funcs.push(("print:".to_string(), vm.new_native(mc, NativeFunc(native_print))));
    funcs.push(("print:and:".to_string(), vm.new_native(mc, NativeFunc(native_print))));
    funcs.push(("print:and:and:and:".to_string(), vm.new_native(mc, NativeFunc(native_print))));
    funcs.push(("regex_match:".to_string(), vm.new_native(mc, NativeFunc(native_regex_match))));
    
    // Operators
    funcs.push(("+".to_string(), vm.new_native(mc, NativeFunc(native_add))));
    funcs.push(("-".to_string(), vm.new_native(mc, NativeFunc(native_sub))));
    funcs.push(("*".to_string(), vm.new_native(mc, NativeFunc(native_mul))));
    funcs.push(("/".to_string(), vm.new_native(mc, NativeFunc(native_div))));
    funcs.push(("==".to_string(), vm.new_native(mc, NativeFunc(native_eq))));
    funcs.push(("!=".to_string(), vm.new_native(mc, NativeFunc(native_ne))));
    funcs.push(("<".to_string(), vm.new_native(mc, NativeFunc(native_lt))));
    funcs.push((">".to_string(), vm.new_native(mc, NativeFunc(native_gt))));
    funcs.push(("<=".to_string(), vm.new_native(mc, NativeFunc(native_le))));
    funcs.push((">=".to_string(), vm.new_native(mc, NativeFunc(native_ge))));
    
    // Unary
    funcs.push(("!".to_string(), vm.new_native(mc, NativeFunc(native_not))));
    funcs.push(("negated".to_string(), vm.new_native(mc, NativeFunc(native_negated))));
    
    // List destructuring
    funcs.push(("at:".to_string(), vm.new_native(mc, NativeFunc(native_list_at))));
    funcs.push(("sliceFrom:".to_string(), vm.new_native(mc, NativeFunc(native_list_slice_from))));

    let mut globals = vm.globals.borrow_mut(mc);
    for (name, val) in funcs {
        globals.insert(name, val);
    }
}
