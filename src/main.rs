use new_vm::error::BBError;
use new_vm::parser::{ast_visitor, parser};
use new_vm::runtime::{class, native, object};
use new_vm::value::{Block, NativeClassBuilder, Value};
use new_vm::vm::{VmState, VmStatus};
use new_vm::{compiler, gc};

use gc_arena::{Arena, Gc, Rootable};

fn main() {
    let script = r#"
Point <- { | @x @y |
  .meta <-- {
    newX:y: -> { |x y|
      .new: { x = x; y = y }
    }
  }

  x -> { @x }
  y -> { @y }

  dist: -> { |other|
    dx = @x - other.x;
    dy = @y - other.y;
    ((dx * dx) + (dy * dy)).sqrt
  }
};

Point <- Point3D <- { | @z |
    z -> { @z }
};

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

p3 = Point3D.new: { |x y z| x = 10; y = 20; z = 30 };
p3.print;
.print:p3.class;
.print:p3.class.s;
.print:p3.class.name;
.print:p3.class.class;
.print:p3.class.class.name;
.print:p3.class.parent;
.print:p3.class.parent.name;
.print:p3.class.parent.parent;
.print:p3.class.parent.parent.name;
.print:(p3.id==p3.id);
.print:(p3.id!=p1.id);
.print:(p3.class==p3.class);
.print:(p3.class!=p1.class);
p3.print:'p3.x =' and: p3.x;
p3.print:'p3.y =' and: p3.y;
p3.print:'p3.z =' and: p3.z;

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

"* Test 6: Non-local return (^^)
Point <-- {
  test_nlr -> {
    bar_func = { |blk|
      blk.value;
      .print: 'Inside bar: should NOT reach here!';
      111
    };

    nested_block = {
      ^^ 777
    };
    bar_func.value: nested_block;
    .print: 'Inside foo: should NOT reach here!';
    222
  }
};

result = p1.test_nlr;
.print: 'Result of non-local return =' and: result;

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
        vm.register_native_class(mc, class::build_class_class());

        // Register placeholder classes for all of the builtin types.
        for t in [
            "Nil",
            "Boolean",
            "Integer",
            "Double",
            "String",
            "List",
            "Dictionary",
            "Regex",
            "Block",
            "Method",
            "Native",
        ] {
            if t == "Double" || t == "Integer" {
                let class_builder = NativeClassBuilder::new(t, Some("Object")).instance_method(
                    "sqrt",
                    |_vm, _mc, args| {
                        if args.is_empty() {
                            return Err(BBError::Other("sqrt expects a receiver".to_string()));
                        }
                        match args[0] {
                            Value::Double(f) => Ok(Value::Double(f.sqrt())),
                            Value::Int(i) => Ok(Value::Double((i as f64).sqrt())),
                            _ => Err(BBError::Other(format!(
                                "sqrt expected number, got {:?}",
                                args[0]
                            ))),
                        }
                    },
                );
                vm.register_native_class(mc, class_builder);
            } else {
                vm.register_native_class(mc, NativeClassBuilder::new(t, Some("Object")));
            }
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
                enclosing_method_id: None,
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
