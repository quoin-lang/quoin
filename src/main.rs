mod instruction;
mod value;
mod vm;
mod assembler;

use gc_arena::{Arena, Rootable, Mutation, Gc};
use crate::value::{Value, Block, NativeFunc};
use crate::vm::{VmState, VmStatus};

// Native helper: print
fn native_print<'gc>(
    _vm: &mut VmState<'gc>,
    _mc: &Mutation<'gc>,
    args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, String> {
    for (i, arg) in args.iter().enumerate() {
        if i > 0 {
            print!(" ");
        }
        print!("{}", arg);
    }
    println!();
    Ok(Value::Nil)
}

// Native helper: len
fn native_len<'gc>(
    _vm: &mut VmState<'gc>,
    _mc: &Mutation<'gc>,
    args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, String> {
    if args.len() != 1 {
        return Err("len() expects exactly 1 argument".to_string());
    }
    match &args[0] {
        Value::String(s) => Ok(Value::Int((**s).len() as i64)),
        Value::List(l) => Ok(Value::Int(l.borrow().len() as i64)),
        Value::Dict(d) => Ok(Value::Int(d.borrow().len() as i64)),
        _ => Err(format!("len() expects string, list, or dict, got {:?}", args[0])),
    }
}

// Native helper: list push
fn native_push<'gc>(
    _vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, String> {
    if args.len() != 2 {
        return Err("push() expects exactly 2 arguments (list, element)".to_string());
    }
    match &args[0] {
        Value::List(l) => {
            l.borrow_mut(mc).push(args[1]);
            Ok(Value::Nil)
        }
        _ => Err(format!("push() first argument must be list, got {:?}", args[0])),
    }
}

// Native helper: list pop
fn native_pop<'gc>(
    _vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, String> {
    if args.len() != 1 {
        return Err("pop() expects exactly 1 argument (list)".to_string());
    }
    match &args[0] {
        Value::List(l) => {
            let val = l.borrow_mut(mc).pop().unwrap_or(Value::Nil);
            Ok(val)
        }
        _ => Err(format!("pop() first argument must be list, got {:?}", args[0])),
    }
}

// Native helper: regex_match
fn native_regex_match<'gc>(
    _vm: &mut VmState<'gc>,
    _mc: &Mutation<'gc>,
    args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, String> {
    if args.len() != 2 {
        return Err("regex_match() expects exactly 2 arguments (regex, string)".to_string());
    }
    match (&args[0], &args[1]) {
        (Value::Regex(r), Value::String(s)) => {
            let matched = r.0.is_match(&**s);
            Ok(Value::Bool(matched))
        }
        _ => Err(format!(
            "regex_match() expects regex and string, got {:?} and {:?}",
            args[0], args[1]
        )),
    }
}

// Native helper: add
fn native_add<'gc>(
    _vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, String> {
    if args.len() != 2 {
        return Err("add expects 2 arguments".into());
    }
    match (&args[0], &args[1]) {
        (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a + b)),
        (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a + b)),
        (Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 + b)),
        (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a + *b as f64)),
        (Value::String(a), Value::String(b)) => {
            let new_str = format!("{}{}", **a, **b);
            Ok(Value::String(Gc::new(mc, new_str)))
        }
        _ => Err(format!("Cannot add {:?} and {:?}", args[0], args[1])),
    }
}

// Native helper: sub
fn native_sub<'gc>(
    _vm: &mut VmState<'gc>,
    _mc: &Mutation<'gc>,
    args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, String> {
    if args.len() != 2 {
        return Err("sub expects 2 arguments".into());
    }
    match (&args[0], &args[1]) {
        (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a - b)),
        (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a - b)),
        (Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 - b)),
        (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a - *b as f64)),
        _ => Err(format!("Cannot subtract {:?} and {:?}", args[0], args[1])),
    }
}

// Native helper: mul
fn native_mul<'gc>(
    _vm: &mut VmState<'gc>,
    _mc: &Mutation<'gc>,
    args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, String> {
    if args.len() != 2 {
        return Err("mul expects 2 arguments".into());
    }
    match (&args[0], &args[1]) {
        (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a * b)),
        (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a * b)),
        (Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 * b)),
        (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a * *b as f64)),
        _ => Err(format!("Cannot multiply {:?} and {:?}", args[0], args[1])),
    }
}

// Native helper: div
fn native_div<'gc>(
    _vm: &mut VmState<'gc>,
    _mc: &Mutation<'gc>,
    args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, String> {
    if args.len() != 2 {
        return Err("div expects 2 arguments".into());
    }
    match (&args[0], &args[1]) {
        (Value::Int(a), Value::Int(b)) => {
            if *b == 0 {
                return Err("Division by zero".into());
            }
            Ok(Value::Int(a / b))
        }
        (Value::Float(a), Value::Float(b)) => Ok(Value::Float(a / b)),
        (Value::Int(a), Value::Float(b)) => Ok(Value::Float(*a as f64 / b)),
        (Value::Float(a), Value::Int(b)) => Ok(Value::Float(a / *b as f64)),
        _ => Err(format!("Cannot divide {:?} and {:?}", args[0], args[1])),
    }
}

// Native helper: eq
fn native_eq<'gc>(
    _vm: &mut VmState<'gc>,
    _mc: &Mutation<'gc>,
    args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, String> {
    if args.len() != 2 {
        return Err("eq expects 2 arguments".into());
    }
    Ok(Value::Bool(args[0] == args[1]))
}

// Native helper: lt
fn native_lt<'gc>(
    _vm: &mut VmState<'gc>,
    _mc: &Mutation<'gc>,
    args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, String> {
    if args.len() != 2 {
        return Err("lt expects 2 arguments".into());
    }
    match (&args[0], &args[1]) {
        (Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a < b)),
        (Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a < b)),
        (Value::Int(a), Value::Float(b)) => Ok(Value::Bool((*a as f64) < *b)),
        (Value::Float(a), Value::Int(b)) => Ok(Value::Bool(*a < (*b as f64))),
        _ => Err(format!("Cannot compare {:?} and {:?}", args[0], args[1])),
    }
}

// Native helper: gt
fn native_gt<'gc>(
    _vm: &mut VmState<'gc>,
    _mc: &Mutation<'gc>,
    args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, String> {
    if args.len() != 2 {
        return Err("gt expects 2 arguments".into());
    }
    match (&args[0], &args[1]) {
        (Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a > b)),
        (Value::Float(a), Value::Float(b)) => Ok(Value::Bool(a > b)),
        (Value::Int(a), Value::Float(b)) => Ok(Value::Bool((*a as f64) > *b)),
        (Value::Float(a), Value::Int(b)) => Ok(Value::Bool(*a > (*b as f64))),
        _ => Err(format!("Cannot compare {:?} and {:?}", args[0], args[1])),
    }
}

fn main() {
    let script = r#"
# Test 1: Closures and local variables
block make_counter initial
  load_local initial
  define_local count
  
  # Anonymous nested block (using '_' name)
  nested_block _
    load_global add
    load_local count
    push int 1
    call 2
    store_local count
    load_local count
    return
  end
  
  return
end
define_local make_counter

# Call make_counter(10)
load_local make_counter
push int 10
call 1
define_local my_counter

# Call it twice
load_local my_counter
call 0
define_local c1  # 11

load_local my_counter
call 0
define_local c2  # 12

load_global print
push string "Counter step 1:"
load_local c1
call 2

load_global print
push string "Counter step 2:"
load_local c2
call 2


# Test 2: Lists, Dicts, and Regex
# Create a list
push int 10
push int 20
push int 30
new_list 3
define_local my_list

# Create a dict
push string "foo"
push int 100
push string "bar"
push int 200
new_dict 2
define_local my_dict

# Print them
load_global print
push string "List:"
load_local my_list
call 2

load_global print
push string "Dict:"
load_local my_dict
call 2

# Regex matching
push string "^[a-z]+$"
new_regex
define_local regex

load_global regex_match
load_local regex
push string "gemini"
call 2
define_local match1

load_global regex_match
load_local regex
push string "123gemini"
call 2
define_local match2

load_global print
push string "Regex match 'gemini':"
load_local match1
call 2

load_global print
push string "Regex match '123gemini':"
load_local match2
call 2


# Test 3: Fatal exception unwinding (Yeet)
block throw_error
  push string "Fatal Error Occurred!"
  yeet
end
define_local throw_error

load_global print
push string "Calling block that will yeet..."
call 1

load_local throw_error
call 0
"#;

    println!("Assembling bytecode script...");
    let program = match assembler::assemble(script) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Assembly error: {}", e);
            std::process::exit(1);
        }
    };
    println!("Assembly successful!");

    println!("Initializing gc-arena and virtual machine...");
    let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
        let mut vm = VmState::new(mc);

        // Register native globals
        {
            let mut globals = vm.globals.borrow_mut(mc);
            globals.insert("print".to_string(), Value::Native(NativeFunc(native_print)));
            globals.insert("len".to_string(), Value::Native(NativeFunc(native_len)));
            globals.insert("push".to_string(), Value::Native(NativeFunc(native_push)));
            globals.insert("pop".to_string(), Value::Native(NativeFunc(native_pop)));
            globals.insert("regex_match".to_string(), Value::Native(NativeFunc(native_regex_match)));
            globals.insert("add".to_string(), Value::Native(NativeFunc(native_add)));
            globals.insert("sub".to_string(), Value::Native(NativeFunc(native_sub)));
            globals.insert("mul".to_string(), Value::Native(NativeFunc(native_mul)));
            globals.insert("div".to_string(), Value::Native(NativeFunc(native_div)));
            globals.insert("eq".to_string(), Value::Native(NativeFunc(native_eq)));
            globals.insert("lt".to_string(), Value::Native(NativeFunc(native_lt)));
            globals.insert("gt".to_string(), Value::Native(NativeFunc(native_gt)));
        }

        // Convert StaticBlock to Block in GC and start it
        let main_block = Gc::new(
            mc,
            Block {
                name: program.name.clone(),
                is_nested_block: program.is_nested_block,
                param_names: program.param_names.clone(),
                bytecode: program.bytecode.clone(),
                parent_env: None,
            },
        );
        vm.start_block(mc, main_block, Vec::new());

        vm
    });

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum SimpleStatus {
        Running,
        Finished,
        Yeeted,
    }

    println!("Running virtual machine...");
    let mut step_count = 0;
    loop {
        let status = arena.mutate_root(|mc, vm| {
            match vm.step(mc) {
                Ok(VmStatus::Running) => Ok(SimpleStatus::Running),
                Ok(VmStatus::Finished(val)) => {
                    println!(
                        "VM execution finished successfully. Top value: {}",
                        val
                    );
                    Ok(SimpleStatus::Finished)
                }
                Ok(VmStatus::Yeeted(val)) => {
                    println!(
                        "VM execution terminated with uncaught exception: {}",
                        val
                    );
                    Ok(SimpleStatus::Yeeted)
                }
                Err(e) => Err(e),
            }
        });
        match status {
            Ok(SimpleStatus::Running) => {
                step_count += 1;
                // Incremental GC triggers occasionally
                if step_count % 10 == 0 {
                    arena.collect_debt();
                }
            }
            Ok(SimpleStatus::Finished) | Ok(SimpleStatus::Yeeted) => {
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
