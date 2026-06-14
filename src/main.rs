use new_vm::error::BBError;
use new_vm::parser::{ast_visitor, parser};
use new_vm::runtime::{native, object};
use new_vm::value::{Block, NativeClassBuilder, Object, Value};
use new_vm::vm::{VmState, VmStatus};
use new_vm::{arg, arg_obj, compiler, gc, gcl};

use gc_arena::{lock::RefLock, Arena, Gc, Rootable};
use std::collections::HashMap;

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
.print: 'p1.id =' and: p1.id;
.print: 'p2.id =' and: p2.id;
.print: 'p1.id =' and: p1.id;
.print: 'p2.id =' and: p2.id;

"* Test 7: Fatal error unwinding
.print: 'Triggering error:';
'Fatal exception yeeted!'.throw;
"#;

    let ast = parser::parse_building_blocks_string(script);

    let program_node = match &ast.value {
        ast_visitor::NodeValue::Program(p) => p,
        _ => {
            eprintln!("Error: Root AST node is not a ProgramNode");
            std::process::exit(1);
        }
    };

    let mut compiler = compiler::Compiler::new();
    let program = match compiler.compile_program(program_node) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Compilation error: {}", e);
            std::process::exit(1);
        }
    };

    let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
        let mut vm = VmState::new(mc);

        native::register_native_funcs(&mut vm, mc);

        vm.register_native_class(mc, object::build_object_class());
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

    arena.finish_cycle();
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
