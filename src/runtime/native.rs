use crate::error::BBError;
use crate::value::{NativeFunc, Value};
use crate::vm::VmState;
use crate::{gc, gcl};

use gc_arena::{Gc, Mutation, RefLock};

// Native helper: print
pub fn native_print<'gc>(
    _vm: &mut VmState<'gc>,
    _mc: &Mutation<'gc>,
    args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, BBError> {
    // args[0] is the receiver (self)
    if args.len() > 1 {
        for (i, arg) in args[1..].iter().enumerate() {
            if i > 0 {
                print!(" ");
            }
            print!("{}", arg);
        }
    }
    println!();
    Ok(Value::Nil)
}

// Native helper: regex_match
pub fn native_regex_match<'gc>(
    _vm: &mut VmState<'gc>,
    _mc: &Mutation<'gc>,
    args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, BBError> {
    if args.len() != 2 {
        return Err(BBError::ArgumentCountMismatch {
            expected: 2,
            got: args.len(),
            msg: "regex_match expects exactly 2 arguments (regex, string)".to_string(),
        });
    }
    match (&args[0], &args[1]) {
        (Value::Regex(r), Value::String(s)) => {
            let matched = r.0.is_match(&**s);
            Ok(Value::Bool(matched))
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
    _vm: &mut VmState<'gc>,
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
    match (&args[0], &args[1]) {
        (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a + b)),
        (Value::Double(a), Value::Double(b)) => Ok(Value::Double(a + b)),
        (Value::Int(a), Value::Double(b)) => Ok(Value::Double(*a as f64 + b)),
        (Value::Double(a), Value::Int(b)) => Ok(Value::Double(a + *b as f64)),
        (Value::String(a), Value::String(b)) => {
            let new_str = format!("{}{}", **a, **b);
            Ok(Value::String(gc!(mc, new_str)))
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
    _vm: &mut VmState<'gc>,
    _mc: &Mutation<'gc>,
    args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, BBError> {
    if args.len() != 2 {
        return Err(BBError::ArgumentCountMismatch {
            expected: 2,
            got: args.len(),
            msg: "sub expects 2 arguments".to_string(),
        });
    }
    match (&args[0], &args[1]) {
        (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a - b)),
        (Value::Double(a), Value::Double(b)) => Ok(Value::Double(a - b)),
        (Value::Int(a), Value::Double(b)) => Ok(Value::Double(*a as f64 - b)),
        (Value::Double(a), Value::Int(b)) => Ok(Value::Double(a - *b as f64)),
        _ => Err(BBError::TypeError {
            expected: "numeric types".to_string(),
            got: format!("{:?} and {:?}", args[0], args[1]),
            msg: format!("Cannot subtract {:?} and {:?}", args[0], args[1]),
        }),
    }
}

// Native helper: mul
pub fn native_mul<'gc>(
    _vm: &mut VmState<'gc>,
    _mc: &Mutation<'gc>,
    args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, BBError> {
    if args.len() != 2 {
        return Err(BBError::ArgumentCountMismatch {
            expected: 2,
            got: args.len(),
            msg: "mul expects 2 arguments".to_string(),
        });
    }
    match (&args[0], &args[1]) {
        (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a * b)),
        (Value::Double(a), Value::Double(b)) => Ok(Value::Double(a * b)),
        (Value::Int(a), Value::Double(b)) => Ok(Value::Double(*a as f64 * b)),
        (Value::Double(a), Value::Int(b)) => Ok(Value::Double(a * *b as f64)),
        _ => Err(BBError::TypeError {
            expected: "numeric types".to_string(),
            got: format!("{:?} and {:?}", args[0], args[1]),
            msg: format!("Cannot multiply {:?} and {:?}", args[0], args[1]),
        }),
    }
}

// Native helper: div
pub fn native_div<'gc>(
    _vm: &mut VmState<'gc>,
    _mc: &Mutation<'gc>,
    args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, BBError> {
    if args.len() != 2 {
        return Err(BBError::ArgumentCountMismatch {
            expected: 2,
            got: args.len(),
            msg: "div expects 2 arguments".to_string(),
        });
    }
    match (&args[0], &args[1]) {
        (Value::Int(a), Value::Int(b)) => {
            if *b == 0 {
                return Err(BBError::ArithmeticError("Division by zero".to_string()));
            }
            Ok(Value::Int(a / b))
        }
        (Value::Double(a), Value::Double(b)) => Ok(Value::Double(a / b)),
        (Value::Int(a), Value::Double(b)) => Ok(Value::Double(*a as f64 / b)),
        (Value::Double(a), Value::Int(b)) => Ok(Value::Double(a / *b as f64)),
        _ => Err(BBError::TypeError {
            expected: "numeric types".to_string(),
            got: format!("{:?} and {:?}", args[0], args[1]),
            msg: format!("Cannot divide {:?} and {:?}", args[0], args[1]),
        }),
    }
}

// Native helper: eq
pub fn native_eq<'gc>(
    _vm: &mut VmState<'gc>,
    _mc: &Mutation<'gc>,
    args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, BBError> {
    if args.len() != 2 {
        return Err(BBError::ArgumentCountMismatch {
            expected: 2,
            got: args.len(),
            msg: "eq expects 2 arguments".to_string(),
        });
    }
    Ok(Value::Bool(args[0] == args[1]))
}

// Native helper: ne
pub fn native_ne<'gc>(
    _vm: &mut VmState<'gc>,
    _mc: &Mutation<'gc>,
    args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, BBError> {
    if args.len() != 2 {
        return Err(BBError::ArgumentCountMismatch {
            expected: 2,
            got: args.len(),
            msg: "ne expects 2 arguments".to_string(),
        });
    }
    Ok(Value::Bool(args[0] != args[1]))
}

// Native helper: lt
pub fn native_lt<'gc>(
    _vm: &mut VmState<'gc>,
    _mc: &Mutation<'gc>,
    args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, BBError> {
    if args.len() != 2 {
        return Err(BBError::ArgumentCountMismatch {
            expected: 2,
            got: args.len(),
            msg: "lt expects 2 arguments".to_string(),
        });
    }
    match (&args[0], &args[1]) {
        (Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a < b)),
        (Value::Double(a), Value::Double(b)) => Ok(Value::Bool(a < b)),
        (Value::Int(a), Value::Double(b)) => Ok(Value::Bool((*a as f64) < *b)),
        (Value::Double(a), Value::Int(b)) => Ok(Value::Bool(*a < (*b as f64))),
        _ => Err(BBError::TypeError {
            expected: "comparable types".to_string(),
            got: format!("{:?} and {:?}", args[0], args[1]),
            msg: format!("Cannot compare {:?} and {:?}", args[0], args[1]),
        }),
    }
}

// Native helper: gt
pub fn native_gt<'gc>(
    _vm: &mut VmState<'gc>,
    _mc: &Mutation<'gc>,
    args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, BBError> {
    if args.len() != 2 {
        return Err(BBError::ArgumentCountMismatch {
            expected: 2,
            got: args.len(),
            msg: "gt expects 2 arguments".to_string(),
        });
    }
    match (&args[0], &args[1]) {
        (Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a > b)),
        (Value::Double(a), Value::Double(b)) => Ok(Value::Bool(a > b)),
        (Value::Int(a), Value::Double(b)) => Ok(Value::Bool((*a as f64) > *b)),
        (Value::Double(a), Value::Int(b)) => Ok(Value::Bool(*a > (*b as f64))),
        _ => Err(BBError::TypeError {
            expected: "comparable types".to_string(),
            got: format!("{:?} and {:?}", args[0], args[1]),
            msg: format!("Cannot compare {:?} and {:?}", args[0], args[1]),
        }),
    }
}

// Native helper: le
pub fn native_le<'gc>(
    _vm: &mut VmState<'gc>,
    _mc: &Mutation<'gc>,
    args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, BBError> {
    if args.len() != 2 {
        return Err(BBError::ArgumentCountMismatch {
            expected: 2,
            got: args.len(),
            msg: "le expects 2 arguments".to_string(),
        });
    }
    match (&args[0], &args[1]) {
        (Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a <= b)),
        (Value::Double(a), Value::Double(b)) => Ok(Value::Bool(a <= b)),
        (Value::Int(a), Value::Double(b)) => Ok(Value::Bool((*a as f64) <= *b)),
        (Value::Double(a), Value::Int(b)) => Ok(Value::Bool(*a <= (*b as f64))),
        _ => Err(BBError::TypeError {
            expected: "comparable types".to_string(),
            got: format!("{:?} and {:?}", args[0], args[1]),
            msg: format!("Cannot compare {:?} and {:?}", args[0], args[1]),
        }),
    }
}

// Native helper: ge
pub fn native_ge<'gc>(
    _vm: &mut VmState<'gc>,
    _mc: &Mutation<'gc>,
    args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, BBError> {
    if args.len() != 2 {
        return Err(BBError::ArgumentCountMismatch {
            expected: 2,
            got: args.len(),
            msg: "ge expects 2 arguments".to_string(),
        });
    }
    match (&args[0], &args[1]) {
        (Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a >= b)),
        (Value::Double(a), Value::Double(b)) => Ok(Value::Bool(a >= b)),
        (Value::Int(a), Value::Double(b)) => Ok(Value::Bool((*a as f64) >= *b)),
        (Value::Double(a), Value::Int(b)) => Ok(Value::Bool(*a >= (*b as f64))),
        _ => Err(BBError::TypeError {
            expected: "comparable types".to_string(),
            got: format!("{:?} and {:?}", args[0], args[1]),
            msg: format!("Cannot compare {:?} and {:?}", args[0], args[1]),
        }),
    }
}

// Native helper: logic not
pub fn native_not<'gc>(
    _vm: &mut VmState<'gc>,
    _mc: &Mutation<'gc>,
    args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, BBError> {
    if args.len() != 1 {
        return Err(BBError::ArgumentCountMismatch {
            expected: 1,
            got: args.len(),
            msg: "not expects exactly 1 argument (receiver)".to_string(),
        });
    }
    Ok(Value::Bool(!args[0].is_truthy()))
}

// Native helper: negated
pub fn native_negated<'gc>(
    _vm: &mut VmState<'gc>,
    _mc: &Mutation<'gc>,
    args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, BBError> {
    if args.len() != 1 {
        return Err(BBError::ArgumentCountMismatch {
            expected: 1,
            got: args.len(),
            msg: "negated expects exactly 1 argument (receiver)".to_string(),
        });
    }
    match &args[0] {
        Value::Int(i) => Ok(Value::Int(-*i)),
        Value::Double(f) => Ok(Value::Double(-*f)),
        _ => Err(BBError::TypeError {
            expected: "Integer or Float".to_string(),
            got: args[0].type_name().to_string(),
            msg: format!("negated expects integer or float, got {:?}", args[0]),
        }),
    }
}

// Native helper: list index lookup (at:)
pub fn native_list_at<'gc>(
    _vm: &mut VmState<'gc>,
    _mc: &Mutation<'gc>,
    args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, BBError> {
    if args.len() != 2 {
        return Err(BBError::ArgumentCountMismatch {
            expected: 2,
            got: args.len(),
            msg: "at expects exactly 2 arguments (receiver, index)".to_string(),
        });
    }
    match (&args[0], &args[1]) {
        (Value::List(l), Value::Int(idx)) => {
            let borrowed = l.borrow();
            let idx = *idx;
            if idx >= 0 && idx < borrowed.len() as i64 {
                Ok(borrowed[idx as usize])
            } else {
                Ok(Value::Nil)
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
    _vm: &mut VmState<'gc>,
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
    match (&args[0], &args[1]) {
        (Value::List(l), Value::Int(idx)) => {
            let borrowed = l.borrow();
            let start = (*idx).max(0) as usize;
            let sliced = if start < borrowed.len() {
                borrowed[start..].to_vec()
            } else {
                Vec::new()
            };
            Ok(Value::List(gcl!(mc, sliced)))
        }
        _ => Err(format!(
            "sliceFrom expects list and integer, got {:?} and {:?}",
            args[0], args[1]
        )
        .into()),
    }
}

pub fn register_native_funcs<'gc>(vm: &mut VmState<'gc>, mc: &Mutation<'gc>) {
    // Register dynamic methods/operators in globals
    let mut globals = vm.globals.borrow_mut(mc);
    globals.insert(
        "print:".to_string(),
        Value::Native(NativeFunc(native_print)),
    );
    globals.insert(
        "print:and:".to_string(),
        Value::Native(NativeFunc(native_print)),
    );
    globals.insert(
        "print:and:and:and:".to_string(),
        Value::Native(NativeFunc(native_print)),
    );
    globals.insert(
        "regex_match:".to_string(),
        Value::Native(NativeFunc(native_regex_match)),
    );

    // Operators
    globals.insert("+".to_string(), Value::Native(NativeFunc(native_add)));
    globals.insert("-".to_string(), Value::Native(NativeFunc(native_sub)));
    globals.insert("*".to_string(), Value::Native(NativeFunc(native_mul)));
    globals.insert("/".to_string(), Value::Native(NativeFunc(native_div)));
    globals.insert("==".to_string(), Value::Native(NativeFunc(native_eq)));
    globals.insert("!=".to_string(), Value::Native(NativeFunc(native_ne)));
    globals.insert("<".to_string(), Value::Native(NativeFunc(native_lt)));
    globals.insert(">".to_string(), Value::Native(NativeFunc(native_gt)));
    globals.insert("<=".to_string(), Value::Native(NativeFunc(native_le)));
    globals.insert(">=".to_string(), Value::Native(NativeFunc(native_ge)));

    // Unary
    globals.insert("!".to_string(), Value::Native(NativeFunc(native_not)));
    globals.insert(
        "negated".to_string(),
        Value::Native(NativeFunc(native_negated)),
    );

    // List destructuring
    globals.insert("at:".to_string(), Value::Native(NativeFunc(native_list_at)));
    globals.insert(
        "sliceFrom:".to_string(),
        Value::Native(NativeFunc(native_list_slice_from)),
    );
}
