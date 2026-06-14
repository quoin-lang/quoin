use crate::value::{Block, NativeClassBuilder, NativeFunc, Object, Value};
use crate::vm::{VmState, VmStatus};
use new_vm::error::BBError;
use new_vm::{arg, arg_obj, gc, gcl};

mod compiler;
mod instruction;
mod parser;
mod value;
mod vm;

use gc_arena::{lock::RefLock, Arena, Gc, Mutation, Rootable};
use std::collections::HashMap;

// Native helper: print
fn native_print<'gc>(
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
fn native_regex_match<'gc>(
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
fn native_add<'gc>(
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
        (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a + b)),
        (Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 + b)),
        (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a + *b as f64)),
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
fn native_sub<'gc>(
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
        (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a - b)),
        (Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 - b)),
        (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a - *b as f64)),
        _ => Err(BBError::TypeError {
            expected: "numeric types".to_string(),
            got: format!("{:?} and {:?}", args[0], args[1]),
            msg: format!("Cannot subtract {:?} and {:?}", args[0], args[1]),
        }),
    }
}

// Native helper: mul
fn native_mul<'gc>(
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
        (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a * b)),
        (Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 * b)),
        (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a * *b as f64)),
        _ => Err(BBError::TypeError {
            expected: "numeric types".to_string(),
            got: format!("{:?} and {:?}", args[0], args[1]),
            msg: format!("Cannot multiply {:?} and {:?}", args[0], args[1]),
        }),
    }
}

// Native helper: div
fn native_div<'gc>(
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
        (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a / b)),
        (Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 / b)),
        (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a / *b as f64)),
        _ => Err(BBError::TypeError {
            expected: "numeric types".to_string(),
            got: format!("{:?} and {:?}", args[0], args[1]),
            msg: format!("Cannot divide {:?} and {:?}", args[0], args[1]),
        }),
    }
}

// Native helper: eq
fn native_eq<'gc>(
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
fn native_ne<'gc>(
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
fn native_lt<'gc>(
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
        (Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a < b)),
        (Value::Int(a), Value::Float(b)) => Ok(Value::Bool((*a as f64) < *b)),
        (Value::Float(a), Value::Int(b)) => Ok(Value::Bool(*a < (*b as f64))),
        _ => Err(BBError::TypeError {
            expected: "comparable types".to_string(),
            got: format!("{:?} and {:?}", args[0], args[1]),
            msg: format!("Cannot compare {:?} and {:?}", args[0], args[1]),
        }),
    }
}

// Native helper: gt
fn native_gt<'gc>(
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
        (Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a > b)),
        (Value::Int(a), Value::Float(b)) => Ok(Value::Bool((*a as f64) > *b)),
        (Value::Float(a), Value::Int(b)) => Ok(Value::Bool(*a > (*b as f64))),
        _ => Err(BBError::TypeError {
            expected: "comparable types".to_string(),
            got: format!("{:?} and {:?}", args[0], args[1]),
            msg: format!("Cannot compare {:?} and {:?}", args[0], args[1]),
        }),
    }
}

// Native helper: le
fn native_le<'gc>(
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
        (Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a <= b)),
        (Value::Int(a), Value::Float(b)) => Ok(Value::Bool((*a as f64) <= *b)),
        (Value::Float(a), Value::Int(b)) => Ok(Value::Bool(*a <= (*b as f64))),
        _ => Err(BBError::TypeError {
            expected: "comparable types".to_string(),
            got: format!("{:?} and {:?}", args[0], args[1]),
            msg: format!("Cannot compare {:?} and {:?}", args[0], args[1]),
        }),
    }
}

// Native helper: ge
fn native_ge<'gc>(
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
        (Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a >= b)),
        (Value::Int(a), Value::Float(b)) => Ok(Value::Bool((*a as f64) >= *b)),
        (Value::Float(a), Value::Int(b)) => Ok(Value::Bool(*a >= (*b as f64))),
        _ => Err(BBError::TypeError {
            expected: "comparable types".to_string(),
            got: format!("{:?} and {:?}", args[0], args[1]),
            msg: format!("Cannot compare {:?} and {:?}", args[0], args[1]),
        }),
    }
}

// Native helper: logic not
fn native_not<'gc>(
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
fn native_negated<'gc>(
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
        Value::Float(f) => Ok(Value::Float(-*f)),
        _ => Err(BBError::TypeError {
            expected: "Integer or Float".to_string(),
            got: args[0].type_name().to_string(),
            msg: format!("negated expects integer or float, got {:?}", args[0]),
        }),
    }
}

// Native helper: list index lookup (at:)
fn native_list_at<'gc>(
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
fn native_list_slice_from<'gc>(
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

fn main() {
    let script = r#"
"* Test 1: Simple assignments, variables, and operators
x = 10;
y = 20;
z = x + y;
.print: 'z = x + y =' and: z;

"* Test 2: List destructuring
a b *rest = #(100 200 300 400 500);
.print: 'a =' and: a;
.print: 'b =' and: b;
.print: 'rest =' and: rest;

"* Test 3: Lexical scopes and blocks/closures
make_counter = { |initial|
  count = initial;
  {
    count = count + 1;
    count
  }
};

counter = make_counter.value: 10;
c1 = counter.value;
c2 = counter.value;
.print: 'c1 =' and: c1;
.print: 'c2 =' and: c2;

"* Test 4: Unary operators
flag = true;
inv_flag = !flag;
.print: 'flag =' and: flag and: 'inv_flag =' and: inv_flag;

num = 50;
neg_num = -num;
.print: 'num =' and: num and: 'neg_num =' and: neg_num;

"* Test 5: Dicts & Regex
my_dict = #{ 'foo': 100 'bar': 200 };
.print: 'dict =' and: my_dict;

re = #/^[a-z]+$/;
is_match = re.regex_match: 'gemini';
.print: 'regex match =' and: is_match;

"* Test 6: Native Class & Methods
p1 = Point.newX: 3 y: 4;
p2 = Point.newX: 0 y: 0;
.print: 'p1.x =' and: p1.x;
.print: 'p1.y =' and: p1.y;
d = p1.dist: p2;
.print: 'distance =' and: d;
p1.print;

"* Test 7: Fatal error unwinding
.print: 'Triggering error:';
'Fatal exception yeeted!'.throw;
"#;

    println!("Parsing BuildingBlocks script to AST...");
    let ast = parser::parser::parse_building_blocks_string(script);

    let program_node = match &ast.value {
        parser::ast_visitor::NodeValue::Program(p) => p,
        _ => {
            eprintln!("Error: Root AST node is not a ProgramNode");
            std::process::exit(1);
        }
    };

    println!("Compiling AST to VM bytecode...");
    let mut compiler = compiler::Compiler::new();
    let program = match compiler.compile_program(program_node) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Compilation error: {}", e);
            std::process::exit(1);
        }
    };
    println!("Compilation successful!");

    println!("Initializing gc-arena and virtual machine...");
    let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
        let mut vm = VmState::new(mc);

        // Register dynamic methods/operators in globals
        {
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

        vm.register_native_class(mc, build_object_class());
        vm.register_native_class(mc, build_point_class());

        // Register placeholder classes for all of the builtin types.
        for t in [
            "Nil",
            "Boolean",
            "Integer",
            "Float",
            "String",
            "List",
            "Dictionary",
            "Regex",
            "Block",
            "Method",
            "Native",
        ] {
            vm.register_native_class(mc, NativeClassBuilder::new(t, Some("Object")));
        }

        // Convert StaticBlock to Block in GC and start it
        let main_block = gc!(
            mc,
            Block {
                name: program.name.clone(),
                is_nested_block: program.is_nested_block,
                param_names: program.param_names.clone(),
                bytecode: program.bytecode.clone(),
                parent_env: None,
            }
        );
        vm.start_block(mc, main_block, Vec::new());

        vm
    });

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum ExecutionStatus {
        Running,
        Finished,
        Yeeted,
    }

    println!("Running virtual machine...");
    let mut step_count = 0;
    loop {
        let status = arena.mutate_root(|mc, vm| match vm.step(mc) {
            Ok(VmStatus::Running) => Ok(ExecutionStatus::Running),
            Ok(VmStatus::Finished(val)) => {
                println!("VM execution finished successfully. Top value: {}", val);
                Ok(ExecutionStatus::Finished)
            }
            Ok(VmStatus::Yeeted(val)) => {
                println!("VM execution terminated with uncaught exception: {}", val);
                Ok(ExecutionStatus::Yeeted)
            }
            Err(e) => Err(e),
        });
        match status {
            Ok(ExecutionStatus::Running) => {
                step_count += 1;
                if step_count % 10 == 0 {
                    arena.collect_debt();
                }
            }
            Ok(ExecutionStatus::Finished) | Ok(ExecutionStatus::Yeeted) => {
                break;
            }
            Err(e) => {
                eprintln!("VM execution error: {}", e);
                break;
            }
        }
    }

    println!("Running full garbage collection cycle...");
    arena.finish_cycle();
    println!("Done!");
}

fn build_object_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Object", None)
        .instance_method("s", |_vm, mc, args| {
            Ok(Value::String(gc!(mc, format!("{}", args[0]))))
        })
        .instance_method("print", |vm, mc, args| {
            let s_result = vm.call_method(mc, args[0], "s", vec![])?;

            println!(
                "{}",
                match s_result {
                    Value::String(string) => string.to_string(),
                    x => format!("{:?}", x),
                }
            );

            Ok(Value::Nil)
        })
        .instance_method("throw", |_vm, _mc, args| {
            // TODO: implement throw properly
            Err(format!("{}", args[0]).into())
        })
}

fn build_point_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Point", Some("Object"))
        .class_method("newX:y:", |_vm, mc, args| {
            if args.len() != 3 {
                return Err(BBError::ArgumentCountMismatch {
                    expected: 3,
                    got: args.len(),
                    msg: "Point newX:y: expects exactly 2 arguments (x, y)".to_string(),
                });
            }
            let class_ref = arg!(args, Class, 0, "Expected Class as receiver");
            let mut fields = HashMap::new();
            fields.insert("x".to_string(), args[1]);
            fields.insert("y".to_string(), args[2]);
            Ok(Value::Object(gcl!(
                mc,
                Object {
                    class: class_ref,
                    fields,
                }
            )))
        })
        .instance_method("x", |_vm, _mc, args| {
            let obj = arg_obj!(args, "Point", 0);
            Ok(obj.borrow().get_field_or_default("x"))
        })
        .instance_method("y", |_vm, _mc, args| {
            let obj = arg_obj!(args, "Point", 0);
            Ok(obj.borrow().get_field_or_default("y"))
        })
        .instance_method("dist:", |_vm, _mc, args| {
            let obj1 = arg_obj!(args, "Point", 0);
            let (x1, y1) = (
                obj1.borrow().get_field_or_default("x"),
                obj1.borrow().get_field_or_default("y"),
            );
            let obj2 = arg_obj!(args, "Point", 1);
            let (x2, y2) = (
                obj2.borrow().get_field_or_default("x"),
                obj2.borrow().get_field_or_default("y"),
            );

            let to_f64 = |val| match val {
                Value::Int(i) => i as f64,
                Value::Float(f) => f,
                _ => 0.0,
            };

            let dx = to_f64(x1) - to_f64(x2);
            let dy = to_f64(y1) - to_f64(y2);
            Ok(Value::Float((dx * dx + dy * dy).sqrt()))
        })
}
