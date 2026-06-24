use super::*;
use crate::instruction::{Constant, SharedBytecode, SharedSourceMap, StaticBlock};
use crate::parser::ast::NodeValue;
use crate::runtime::block::build_block_class;
use crate::runtime::boolean::build_boolean_class;
use crate::runtime::class::build_class_class;
use crate::runtime::double::build_double_class;
use crate::runtime::integer::build_integer_class;
use crate::runtime::list::build_list_class;
use crate::runtime::map::{build_key_value_pair_class, build_map_class};
use crate::runtime::nil::build_nil_class;
use crate::runtime::object::build_object_class;
use crate::runtime::regex::build_regex_class;
use crate::runtime::string::build_string_class;
use crate::value::{NativeClassBuilder, OpaqueState};
use gc_arena::{Arena, Rootable};

fn native_add<'gc>(
    vm: &mut VmState<'gc>,
    mc: &Mutation<'gc>,
    receiver: Value<'gc>,
    args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, QuoinError> {
    let a = receiver
        .as_i64()
        .ok_or_else(|| QuoinError::Other("Invalid types".to_string()))?;
    let b = args[0]
        .as_i64()
        .ok_or_else(|| QuoinError::Other("Invalid types".to_string()))?;
    Ok(vm.new_int(mc, a + b))
}

#[derive(Debug, PartialEq, Clone)]
enum ValueSpec {
    Nil,
    Bool(bool),
    Int(i64),
    Double(f64),
    String(String),
    Symbol(String),
    Class(String),
    ClassMeta(String),
    List(Vec<ValueSpec>),
    Map(HashMap<String, ValueSpec>),
    Regex(String),
    Block(Option<String>),
    Instance(String),
}

fn to_spec(val: Value<'_>) -> ValueSpec {
    match val {
        Value::Int(i) => ValueSpec::Int(i),
        Value::Double(d) => ValueSpec::Double(d),
        Value::Bool(b) => ValueSpec::Bool(b),
        Value::Nil => ValueSpec::Nil,
        Value::Class(c) => ValueSpec::Class(c.borrow().name.to_string()),
        Value::ClassMeta(c) => ValueSpec::ClassMeta(c.borrow().name.to_string()),
        Value::Object(obj) => {
            let borrowed = obj.borrow();
            match &borrowed.payload {
                ObjectPayload::String(s) => ValueSpec::String((**s).clone()),
                ObjectPayload::Symbol(s) => ValueSpec::Symbol((**s).clone()),
                _ if borrowed.class_name() == "List" => {
                    let res = val.with_native_state::<NativeListState, _, _>(|l| {
                        let list_specs = l.get_vec().iter().map(|&v| to_spec(v)).collect();
                        ValueSpec::List(list_specs)
                    });
                    res.unwrap_or_else(|_| ValueSpec::Instance("List".to_string()))
                }
                _ if borrowed.class_name() == "Map" => {
                    let res = val.with_native_state::<NativeMapState, _, _>(|m| {
                        let map_specs = m
                            .get_map()
                            .iter()
                            .map(|(k, &v)| (k.clone(), to_spec(v)))
                            .collect();
                        ValueSpec::Map(map_specs)
                    });
                    res.unwrap_or_else(|_| ValueSpec::Instance("Map".to_string()))
                }
                _ if borrowed.class_name() == "Regex" => {
                    let res = val.with_native_state::<NativeRegexState, _, _>(|r| {
                        ValueSpec::Regex(r.regex.as_str().to_string())
                    });
                    res.unwrap_or_else(|_| ValueSpec::Instance("Regex".to_string()))
                }
                ObjectPayload::Block(b) => ValueSpec::Block(b.name.clone()),
                ObjectPayload::Bytes(_) => ValueSpec::Instance("Bytes".to_string()),
                ObjectPayload::Instance | ObjectPayload::NativeState(_) => {
                    ValueSpec::Instance(borrowed.class.borrow().name.to_string())
                }
            }
        }
    }
}

#[derive(Debug, PartialEq, Clone)]
enum VmStatusSpec {
    Running,
    Finished(ValueSpec),
    Yeeted(ValueSpec),
}

fn to_status_spec(status: VmStatus<'_>) -> VmStatusSpec {
    match status {
        VmStatus::Running => VmStatusSpec::Running,
        VmStatus::Finished(val) => VmStatusSpec::Finished(to_spec(val)),
        VmStatus::Yeeted(val) => VmStatusSpec::Yeeted(to_spec(val)),
    }
}

fn stack_spec(vm: &VmState<'_>) -> Vec<ValueSpec> {
    vm.stack.iter().copied().map(to_spec).collect()
}

fn run_test_steps<F>(instructions: Vec<Instruction>, check_steps: F)
where
    F: for<'gc> FnOnce(&mut VmState<'gc>, &Mutation<'gc>),
{
    let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
        let mut vm = VmState::new(mc, VmOptions::default());

        // Register standard classes first, so that they exist when new_xxx helper methods are called.
        vm.register_native_class(mc, build_object_class());
        vm.register_native_class(mc, build_class_class());
        vm.register_native_class(mc, build_boolean_class());
        vm.register_native_class(mc, build_block_class());
        vm.register_native_class(mc, build_list_class());
        vm.register_native_class(mc, build_double_class());
        vm.register_native_class(mc, build_integer_class());
        vm.register_native_class(mc, build_string_class());
        vm.register_native_class(mc, build_nil_class());
        vm.register_native_class(mc, build_map_class());
        vm.register_native_class(mc, build_key_value_pair_class());
        vm.register_native_class(mc, build_regex_class());

        for t in ["Method"] {
            vm.register_native_class(mc, NativeClassBuilder::new(t, Some("Object")));
        }

        // Register standard native functions we might need
        let native_val = vm.new_native_method(mc, "+".to_string(), NativeFunc(native_add), None);
        vm.globals
            .borrow_mut(mc)
            .insert(NamespacedName::new(Vec::new(), "+".to_string()), native_val);

        let static_block = StaticBlock {
            source_info: None,
            name: Some("test_main".to_string()),
            is_nested_block: false,
            param_syms: Vec::new(),
            param_types: Vec::new(),
            bytecode: instructions.into(),
            decl_block: None,
            source_map: SharedSourceMap::from(Vec::new()),
        };
        let block = gc!(
            mc,
            Block {
                source_info: None,
                name: static_block.name.clone(),
                is_nested_block: static_block.is_nested_block,
                param_syms: static_block.param_syms.clone(),
                param_types: static_block.param_types.clone(),
                bytecode: static_block.bytecode.clone(),
                parent_env: None,
                enclosing_method_id: None,
                decl_block: None,
                source_map: SharedSourceMap::from(Vec::new()),
            }
        );
        vm.start_block(mc, block, Vec::new(), None, None);
        vm
    });

    arena.mutate_root(|mc, vm| {
        check_steps(vm, mc);
    });
}

#[test]
fn test_push_pop_dup() {
    run_test_steps(
        vec![
            Instruction::Push(Constant::Int(10)),
            Instruction::Push(Constant::Int(20)),
            Instruction::Pop,
            Instruction::Dup,
        ],
        |vm, mc| {
            // Initial: Stack = []
            assert_eq!(vm.stack.len(), 0);

            // Step 1: Push(10)
            let status = vm.step(mc).unwrap();
            assert_eq!(to_status_spec(status), VmStatusSpec::Running);
            assert_eq!(stack_spec(vm), vec![ValueSpec::Int(10)]);

            // Step 2: Push(20)
            let status = vm.step(mc).unwrap();
            assert_eq!(to_status_spec(status), VmStatusSpec::Running);
            assert_eq!(stack_spec(vm), vec![ValueSpec::Int(10), ValueSpec::Int(20)]);

            // Step 3: Pop
            let status = vm.step(mc).unwrap();
            assert_eq!(to_status_spec(status), VmStatusSpec::Running);
            assert_eq!(stack_spec(vm), vec![ValueSpec::Int(10)]);

            // Step 4: Dup
            let status = vm.step(mc).unwrap();
            assert_eq!(to_status_spec(status), VmStatusSpec::Running);
            assert_eq!(stack_spec(vm), vec![ValueSpec::Int(10), ValueSpec::Int(10)]);

            // Step 5: Implicit return Nil
            let status = vm.step(mc).unwrap();
            assert_eq!(to_status_spec(status), VmStatusSpec::Running);
            assert_eq!(
                stack_spec(vm),
                vec![ValueSpec::Int(10), ValueSpec::Int(10), ValueSpec::Nil]
            );

            // Pop the remaining values left on stack for testing to satisfy the stack-empty assertion
            vm.pop().unwrap(); // Nil
            vm.pop().unwrap(); // 10
            vm.pop().unwrap(); // 10
            let nil_val = vm.new_nil(mc);
            vm.push(nil_val); // Push it back as the return value

            // Step 6: Finished
            let status = vm.step(mc).unwrap();
            assert_eq!(
                to_status_spec(status),
                VmStatusSpec::Finished(ValueSpec::Nil)
            );
        },
    );
}

#[test]
fn test_symbol_interning_pointer_equality() {
    // Pull the inner interned string out of a symbol value.
    fn inner(v: Value) -> Gc<String> {
        match v {
            Value::Object(obj) => match obj.borrow().payload {
                ObjectPayload::Symbol(s) => s,
                _ => panic!("expected a Symbol payload"),
            },
            _ => panic!("expected an Object value"),
        }
    }

    run_test_steps(Vec::new(), |vm, mc| {
        let a = vm.new_symbol(mc, "foo".to_string());
        let b = vm.new_symbol(mc, "foo".to_string());
        let c = vm.new_symbol(mc, "bar".to_string());

        // Same name => the inner Gc<String> is pointer-identical (real interning,
        // not the `id`/content fallbacks in Value::eq).
        assert!(
            Gc::ptr_eq(inner(a), inner(b)),
            "interned symbols of the same name must share the inner Gc<String>"
        );
        // ...and the whole canonical Object is shared too.
        match (a, b) {
            (Value::Object(oa), Value::Object(ob)) => assert!(
                Gc::ptr_eq(oa, ob),
                "interned symbols of the same name must be the same Object"
            ),
            _ => panic!("symbols must be Object values"),
        }
        // Different names => distinct pointers.
        assert!(
            !Gc::ptr_eq(inner(a), inner(c)),
            "symbols of different names must not share a pointer"
        );

        // Sanity: Quoin-level equality agrees with identity.
        assert_eq!(a, b);
        assert_ne!(a, c);
    });
}

#[test]
fn test_deferred_call_values_survive_collection() {
    // A `DeferredCall` holds GC `Value`s (receiver + args) in `Frame.defers`.
    // They must be traced so a collection between when a defer is enqueued (e.g.
    // by `mix:`) and when it runs (the Return handler) does not free them. The
    // run loop collects between steps, so this really can happen.
    // (`pending_class_def` / `unregister_on_defer_failure` hold only a 'static
    // `NamespacedName` — no GC pointers — so they need no such guard.)
    let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
        let mut vm = VmState::new(mc, VmOptions::default());
        vm.register_native_class(mc, build_object_class());
        vm.register_native_class(mc, build_string_class());

        // Start a frame the defer can attach to (mirrors run_test_steps).
        let static_block = StaticBlock {
            source_info: None,
            name: Some("defer_gc_test".to_string()),
            is_nested_block: false,
            param_syms: Vec::new(),
            param_types: Vec::new(),
            bytecode: Vec::<Instruction>::new().into(),
            decl_block: None,
            source_map: SharedSourceMap::from(Vec::new()),
        };
        let block = gc!(
            mc,
            Block {
                source_info: None,
                name: static_block.name.clone(),
                is_nested_block: static_block.is_nested_block,
                param_syms: static_block.param_syms.clone(),
                param_types: static_block.param_types.clone(),
                bytecode: static_block.bytecode.clone(),
                parent_env: None,
                enclosing_method_id: None,
                decl_block: None,
                source_map: SharedSourceMap::from(Vec::new()),
            }
        );
        vm.start_block(mc, block, Vec::new(), None, None);
        vm
    });

    // Enqueue a deferred call whose receiver and args are freshly-allocated
    // strings reachable ONLY through the defer.
    arena.mutate_root(|mc, vm| {
        let receiver = vm.new_string(mc, "DEFER-RECEIVER".to_string());
        let arg = vm.new_string(mc, "DEFER-ARG".to_string());
        let frame = vm.frames.last_mut().expect("a frame to hold the defer");
        frame.defers.push(DeferredCall {
            receiver,
            selector: "check:".to_string(),
            args: vec![arg],
        });
    });

    // Allocate a pile of unreachable garbage so the collector has real work to
    // sweep, then drive it through full cycles. If `Frame.defers` weren't traced,
    // the deferred strings would be swept right alongside this garbage.
    arena.mutate_root(|mc, vm| {
        for i in 0..512 {
            let _garbage = vm.new_string(mc, format!("garbage-{i}"));
        }
    });
    arena.finish_cycle();
    arena.finish_cycle();

    // After collection the deferred Values must still be the exact strings.
    arena.mutate_root(|_mc, vm| {
        let frame = vm.frames.last().expect("frame still present");
        assert_eq!(frame.defers.len(), 1, "the defer must survive collection");
        let d = &frame.defers[0];
        assert_eq!(d.selector, "check:");
        for (val, expected) in [(d.receiver, "DEFER-RECEIVER"), (d.args[0], "DEFER-ARG")] {
            match val {
                Value::Object(obj) => match &obj.borrow().payload {
                    ObjectPayload::String(s) => assert_eq!(s.as_str(), expected),
                    _ => panic!("deferred value is no longer a String — collected?"),
                },
                _ => panic!("deferred value is not an Object — collected?"),
            }
        }
    });
}

#[test]
fn test_native_methods_are_chainable() {
    // Native methods are `Method` chain nodes (`NativeState` wrapping a native
    // body), so another variant can be appended onto a native method's chain —
    // which previously errored "Invalid method object in chain" (overriding e.g.
    // List#count). (Bare native-function objects no longer exist at all.)
    let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
        let mut vm = VmState::new(mc, VmOptions::default());
        vm.register_native_class(mc, build_object_class());
        vm
    });

    arena.mutate_root(|mc, vm| {
        let obj_class = vm.get_or_create_builtin_class(mc, "Object");
        let native_method = obj_class
            .borrow()
            .instance_methods
            .get("can?:")
            .copied()
            .expect("Object should have a native can?: method");

        // A native method is a chainable NativeState node.
        match native_method {
            Value::Object(o) => assert!(
                matches!(&o.borrow().payload, ObjectPayload::NativeState(_)),
                "native method should be a chainable NativeState node"
            ),
            _ => panic!("native method should be an object"),
        }

        // Appending another variant onto it succeeds (previously crashed).
        let appended = vm.new_native_method(
            mc,
            "can?:".to_string(),
            NativeFunc(|vm, mc, _receiver, _args| Ok(vm.new_nil(mc))),
            None,
        );
        VmState::append_method_to_chain(mc, native_method, appended)
            .expect("appending onto a native method's chain should succeed");
        assert!(
            vm.get_next_method_in_chain(native_method).is_some(),
            "the native method should now chain to the appended variant"
        );
    });
}

#[test]
fn test_typed_native_method_dispatches_by_type() {
    // Phase 2b: native methods can carry a type signature, so several typed
    // native variants of one selector are routed by argument type, exactly like
    // user multimethod variants.
    use crate::value::NativeClassBuilder;
    let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
        let mut vm = VmState::new(mc, VmOptions::default());
        vm.register_native_class(mc, build_object_class());
        vm.register_native_class(mc, build_integer_class());
        vm.register_native_class(mc, build_string_class());
        // Two typed class-side `kind:` variants on one selector.
        let builder = NativeClassBuilder::new("ScoreTest", Some("Object"))
            .typed_class_method("kind:", &["Integer"], |vm, mc, _receiver, _args| {
                Ok(vm.new_string(mc, "int".to_string()))
            })
            .typed_class_method("kind:", &["String"], |vm, mc, _receiver, _args| {
                Ok(vm.new_string(mc, "str".to_string()))
            });
        vm.register_native_class(mc, builder);
        vm
    });

    arena.mutate_root(|mc, vm| {
        let test_class = vm.get_or_create_builtin_class(mc, "ScoreTest");
        let recv = Value::Class(test_class);

        let check = |v: Value<'_>, expected: &str| match v {
            Value::Object(o) => match &o.borrow().payload {
                ObjectPayload::String(s) => assert_eq!(s.as_str(), expected),
                _ => panic!("expected a String result, got a different payload"),
            },
            _ => panic!("expected a String result"),
        };

        let int_arg = vm.new_int(mc, 5);
        check(
            vm.call_method(mc, recv, "kind:", vec![int_arg]).unwrap(),
            "int",
        );

        let str_arg = vm.new_string(mc, "hi".to_string());
        check(
            vm.call_method(mc, recv, "kind:", vec![str_arg]).unwrap(),
            "str",
        );
    });
}

#[test]
fn test_local_variables() {
    run_test_steps(
        vec![
            Instruction::Push(Constant::Int(42)),
            Instruction::DefineLocal(Symbol::intern("a")),
            Instruction::LoadLocal(Symbol::intern("a")),
            Instruction::Push(Constant::Int(100)),
            Instruction::StoreLocal(Symbol::intern("a")),
            Instruction::LoadLocal(Symbol::intern("a")),
        ],
        |vm, mc| {
            // Step 1: Push(42) -> [Int(42)]
            vm.step(mc).unwrap();
            assert_eq!(stack_spec(vm), vec![ValueSpec::Int(42)]);

            // Step 2: DefineLocal("a") -> []
            vm.step(mc).unwrap();
            assert_eq!(vm.stack.len(), 0);

            // Step 3: LoadLocal("a") -> [Int(42)]
            vm.step(mc).unwrap();
            assert_eq!(stack_spec(vm), vec![ValueSpec::Int(42)]);

            // Step 4: Push(100) -> [Int(42), Int(100)]
            vm.step(mc).unwrap();
            assert_eq!(
                stack_spec(vm),
                vec![ValueSpec::Int(42), ValueSpec::Int(100)]
            );

            // Step 5: StoreLocal("a") -> [Int(42)]
            vm.step(mc).unwrap();
            assert_eq!(stack_spec(vm), vec![ValueSpec::Int(42)]);

            // Step 6: LoadLocal("a") -> [Int(42), Int(100)]
            vm.step(mc).unwrap();
            assert_eq!(
                stack_spec(vm),
                vec![ValueSpec::Int(42), ValueSpec::Int(100)]
            );
        },
    );
}

#[test]
fn test_global_variables() {
    run_test_steps(
        vec![
            Instruction::Push(Constant::Int(77)),
            Instruction::StoreGlobal(NamespacedName::new(Vec::new(), "g_var".to_string()), false),
            Instruction::LoadGlobal(NamespacedName::new(Vec::new(), "g_var".to_string())),
        ],
        |vm, mc| {
            // Step 1: Push(77)
            vm.step(mc).unwrap();
            assert_eq!(stack_spec(vm), vec![ValueSpec::Int(77)]);

            // Step 2: StoreGlobal("g_var")
            vm.step(mc).unwrap();
            assert_eq!(vm.stack.len(), 0);

            // Step 3: LoadGlobal("g_var")
            vm.step(mc).unwrap();
            assert_eq!(stack_spec(vm), vec![ValueSpec::Int(77)]);
        },
    );
}

#[test]
fn test_constants() {
    run_test_steps(
        vec![
            Instruction::Push(Constant::Nil),
            Instruction::Push(Constant::Bool(true)),
            Instruction::Push(Constant::Double(3.14)),
            Instruction::Push(Constant::String("hello".to_string())),
        ],
        |vm, mc| {
            // Nil
            vm.step(mc).unwrap();
            assert_eq!(stack_spec(vm), vec![ValueSpec::Nil]);

            // Bool
            vm.step(mc).unwrap();
            assert_eq!(stack_spec(vm), vec![ValueSpec::Nil, ValueSpec::Bool(true)]);

            // Float
            vm.step(mc).unwrap();
            assert_eq!(
                stack_spec(vm),
                vec![
                    ValueSpec::Nil,
                    ValueSpec::Bool(true),
                    ValueSpec::Double(3.14)
                ]
            );

            // String
            vm.step(mc).unwrap();
            assert_eq!(stack_spec(vm).len(), 4);
            assert_eq!(stack_spec(vm)[3], ValueSpec::String("hello".to_string()));
        },
    );
}

#[test]
fn test_jump_if_else() {
    run_test_steps(
        vec![
            // 0: Push true
            Instruction::Push(Constant::Bool(true)),
            // 1: IfJump to 4 (offset +3 -> 4)
            Instruction::IfJump(3),
            // 2: Push 99 (should be skipped)
            Instruction::Push(Constant::Int(99)),
            // 3: Jump to 5 (offset +2 -> 5)
            Instruction::Jump(2),
            // 4: Push 42 (target of IfJump)
            Instruction::Push(Constant::Int(42)),
            // 5: Push false
            Instruction::Push(Constant::Bool(false)),
            // 6: ElseJump to 9 (offset +3 -> 9)
            Instruction::ElseJump(3),
            // 7: Push 88 (should be skipped)
            Instruction::Push(Constant::Int(88)),
            // 8: Jump to 10 (offset +2 -> 10)
            Instruction::Jump(2),
            // 9: Push 55 (target of ElseJump)
            Instruction::Push(Constant::Int(55)),
        ],
        |vm, mc| {
            // Push true -> [Bool(true)]
            vm.step(mc).unwrap();
            assert_eq!(stack_spec(vm), vec![ValueSpec::Bool(true)]);

            // IfJump(3) -> condition true -> jump to index 4 (Push 42). Stack becomes []
            vm.step(mc).unwrap();
            assert_eq!(vm.stack.len(), 0);
            assert_eq!(vm.frames[0].ip, 4);

            // Push 42 -> [Int(42)]
            vm.step(mc).unwrap();
            assert_eq!(stack_spec(vm), vec![ValueSpec::Int(42)]);

            // Push false -> [Int(42), Bool(false)]
            vm.step(mc).unwrap();
            assert_eq!(
                stack_spec(vm),
                vec![ValueSpec::Int(42), ValueSpec::Bool(false)]
            );

            // ElseJump(3) -> condition false -> jump to index 9 (Push 55). Stack becomes [Int(42)]
            vm.step(mc).unwrap();
            assert_eq!(stack_spec(vm), vec![ValueSpec::Int(42)]);
            assert_eq!(vm.frames[0].ip, 9);

            // Push 55 -> [Int(42), Int(55)]
            vm.step(mc).unwrap();
            assert_eq!(stack_spec(vm), vec![ValueSpec::Int(42), ValueSpec::Int(55)]);
        },
    );
}

#[test]
fn test_list_map_regex() {
    run_test_steps(
        vec![
            // List of 2 elements: Push 1, Push 2, NewList(2)
            Instruction::Push(Constant::Int(1)),
            Instruction::Push(Constant::Int(2)),
            Instruction::NewList(2),
            // Map of 1 pair: Push key "a", Push val 10, NewMap(1)
            Instruction::Push(Constant::String("a".to_string())),
            Instruction::Push(Constant::Int(10)),
            Instruction::NewMap(1),
            // Regex: Push pattern "^ab$", NewRegex
            Instruction::Push(Constant::String("^ab$".to_string())),
            Instruction::NewRegex,
        ],
        |vm, mc| {
            // List creation
            vm.step(mc).unwrap();
            vm.step(mc).unwrap();
            vm.step(mc).unwrap(); // NewList(2)
            assert_eq!(vm.stack.len(), 1);
            assert_eq!(
                stack_spec(vm),
                vec![ValueSpec::List(vec![ValueSpec::Int(1), ValueSpec::Int(2)])]
            );

            // Map creation
            vm.step(mc).unwrap();
            vm.step(mc).unwrap();
            vm.step(mc).unwrap(); // NewMap(1)
            assert_eq!(vm.stack.len(), 2);
            let mut expected_map = HashMap::new();
            expected_map.insert("a".to_string(), ValueSpec::Int(10));
            assert_eq!(stack_spec(vm)[1], ValueSpec::Map(expected_map));

            // Regex creation
            vm.step(mc).unwrap();
            vm.step(mc).unwrap(); // NewRegex
            assert_eq!(vm.stack.len(), 3);
            assert_eq!(stack_spec(vm)[2], ValueSpec::Regex("^ab$".to_string()));
        },
    );
}

#[test]
fn test_send_message() {
    run_test_steps(
        vec![
            Instruction::Push(Constant::Int(5)),
            Instruction::Push(Constant::Int(10)),
            // Send "+" with 1 argument (selector: "+", receiver: Int(5), arg: Int(10))
            Instruction::Send(Symbol::intern("+"), 1),
        ],
        |vm, mc| {
            vm.step(mc).unwrap();
            vm.step(mc).unwrap();
            // Send -> receiver 5 + argument 10 -> returns Int(15)
            vm.step(mc).unwrap();
            assert_eq!(stack_spec(vm), vec![ValueSpec::Int(15)]);
        },
    );
}

#[test]
fn test_block_execution_and_returns() {
    // We will push a block constant, then send "value" to it.
    // The block bytecode will load its parameter, add 1 to it, and return.
    let block_static = StaticBlock {
        source_info: None,
        name: Some("test_block".to_string()),
        is_nested_block: false,
        param_syms: crate::value::intern_param_syms(&vec!["x".to_string()]),
        param_types: vec!["Object".to_string()],
        bytecode: SharedBytecode::from(vec![
            Instruction::LoadLocal(Symbol::intern("x")),
            Instruction::Push(Constant::Int(1)),
            Instruction::Send(Symbol::intern("+"), 1),
            Instruction::Return,
        ]),
        decl_block: None,
        source_map: SharedSourceMap::from(Vec::new()),
    };

    run_test_steps(
        vec![
            Instruction::Push(Constant::Block(block_static)),
            Instruction::Push(Constant::Int(41)),
            // Send "value:" with 1 arg
            Instruction::Send(Symbol::intern("value:"), 1),
        ],
        |vm, mc| {
            vm.step(mc).unwrap(); // Push block -> [Block]
            vm.step(mc).unwrap(); // Push 41 -> [Block, Int(41)]
            assert_eq!(vm.frames.len(), 1);

            // Send -> starts block frame -> [Block]
            vm.step(mc).unwrap();
            assert_eq!(vm.frames.len(), 2);
            assert_eq!(vm.frames[1].block.name, Some("test_block".to_string()));

            // Inside block: LoadLocal("x") -> push 41 -> [41]
            vm.step(mc).unwrap();
            assert_eq!(stack_spec(vm), vec![ValueSpec::Int(41)]);

            // Inside block: Push(1) -> [41, 1]
            vm.step(mc).unwrap();
            assert_eq!(stack_spec(vm), vec![ValueSpec::Int(41), ValueSpec::Int(1)]);

            // Inside block: Send("+", 1) -> [42]
            vm.step(mc).unwrap();
            assert_eq!(stack_spec(vm), vec![ValueSpec::Int(42)]);

            // Inside block: Return -> pops block frame, leaves return value on stack -> [42]
            vm.step(mc).unwrap();
            assert_eq!(vm.frames.len(), 1);
            assert_eq!(stack_spec(vm), vec![ValueSpec::Int(42)]);
        },
    );
}

#[test]
fn test_yeet_exception() {
    run_test_steps(
        vec![Instruction::Push(Constant::Int(500)), Instruction::Yeet],
        |vm, mc| {
            vm.step(mc).unwrap();
            let status = vm.step(mc).unwrap();
            assert_eq!(
                to_status_spec(status),
                VmStatusSpec::Yeeted(ValueSpec::Int(500))
            );
            assert_eq!(vm.frames.len(), 0);
        },
    );
}

#[test]
fn test_method_return() {
    // Block 1 is the method context.
    // Block 2 is a nested block context.
    // If Block 2 executes MethodReturn, it should unwind all frames up to and including the method context (Block 1).

    // Block 2: nested block
    // Bytecode: Push(999), MethodReturn
    let block_nested = StaticBlock {
        source_info: None,
        name: Some("nested".to_string()),
        is_nested_block: true,
        param_syms: Vec::new(),
        param_types: Vec::new(),
        bytecode: SharedBytecode::from(vec![
            Instruction::Push(Constant::Int(999)),
            Instruction::MethodReturn,
        ]),
        decl_block: None,
        source_map: SharedSourceMap::from(Vec::new()),
    };

    // Block 1: method
    // Bytecode: Push(Block(nested)), Send("value", 0), Push(100), Return
    let block_method = StaticBlock {
        source_info: None,
        name: Some("method".to_string()),
        is_nested_block: false, // enclosing_method_id will be this frame's ID
        param_syms: Vec::new(),
        param_types: Vec::new(),
        bytecode: SharedBytecode::from(vec![
            Instruction::Push(Constant::Block(block_nested)),
            Instruction::Send(Symbol::intern("value"), 0),
            Instruction::Push(Constant::Int(100)), // this should be skipped due to MethodReturn
            Instruction::Return,
        ]),
        decl_block: None,
        source_map: SharedSourceMap::from(Vec::new()),
    };

    run_test_steps(
        vec![
            Instruction::Push(Constant::Block(block_method)),
            Instruction::Send(Symbol::intern("value"), 0),
        ],
        |vm, mc| {
            vm.step(mc).unwrap(); // Push block_method
            vm.step(mc).unwrap(); // Send "value" -> starts block_method frame (frame id = 2, enclosing_method_id = 2)
            assert_eq!(vm.frames.len(), 2);
            assert_eq!(vm.frames[1].enclosing_method_id, Some(vm.frames[1].id));

            vm.step(mc).unwrap(); // Inside block_method: Push(block_nested)
            vm.step(mc).unwrap(); // Inside block_method: Send("value", 0) -> starts block_nested frame (frame id = 3, enclosing_method_id = 2)
            assert_eq!(vm.frames.len(), 3);
            assert_eq!(vm.frames[2].enclosing_method_id, Some(vm.frames[1].id));

            vm.step(mc).unwrap(); // Inside block_nested: Push(999) -> Stack has [999]
            // Inside block_nested: MethodReturn.
            // It should pop frame 3 (nested) and frame 2 (method), leaving only the main frame (frame 1),
            // and pushing 999 to the stack.
            vm.step(mc).unwrap();
            assert_eq!(vm.frames.len(), 1);
            assert_eq!(stack_spec(vm), vec![ValueSpec::Int(999)]);
        },
    );
}

#[test]
fn test_non_local_return_callback() {
    // block_nested: Push(777), MethodReturn
    let block_nested = StaticBlock {
        source_info: None,
        name: Some("nested".to_string()),
        is_nested_block: true,
        param_syms: Vec::new(),
        param_types: Vec::new(),
        bytecode: SharedBytecode::from(vec![
            Instruction::Push(Constant::Int(777)),
            Instruction::MethodReturn,
        ]),
        decl_block: None,
        source_map: SharedSourceMap::from(Vec::new()),
    };

    // block_bar: blk.value, Push(111), Return
    let block_bar = StaticBlock {
        source_info: None,
        name: Some("bar".to_string()),
        is_nested_block: false,
        param_syms: crate::value::intern_param_syms(&vec!["blk".to_string()]),
        param_types: vec!["Object".to_string()],
        bytecode: SharedBytecode::from(vec![
            Instruction::LoadLocal(Symbol::intern("blk")),
            Instruction::Send(Symbol::intern("value"), 0),
            Instruction::Push(Constant::Int(111)),
            Instruction::Return,
        ]),
        decl_block: None,
        source_map: SharedSourceMap::from(Vec::new()),
    };

    // block_foo: bar.value: block_nested, Push(222), Return
    let block_foo = StaticBlock {
        source_info: None,
        name: Some("foo".to_string()),
        is_nested_block: false,
        param_syms: Vec::new(),
        param_types: Vec::new(),
        bytecode: SharedBytecode::from(vec![
            Instruction::LoadGlobal(NamespacedName::new(Vec::new(), "bar_func".to_string())),
            Instruction::Push(Constant::Block(block_nested)),
            Instruction::Send(Symbol::intern("value:"), 1),
            Instruction::Push(Constant::Int(222)),
            Instruction::Return,
        ]),
        decl_block: None,
        source_map: SharedSourceMap::from(Vec::new()),
    };

    let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
        let mut vm = VmState::new(mc, VmOptions::default());
        let bar_block = Block {
            source_info: None,
            name: block_bar.name.clone(),
            is_nested_block: block_bar.is_nested_block,
            param_syms: block_bar.param_syms.clone(),
            param_types: block_bar.param_types.clone(),
            bytecode: block_bar.bytecode.clone(),
            parent_env: None,
            enclosing_method_id: None,
            decl_block: None,
            source_map: SharedSourceMap::from(Vec::new()),
        };
        let bar_block_val = vm.new_block(mc, bar_block);
        vm.globals.borrow_mut(mc).insert(
            NamespacedName::new(Vec::new(), "bar_func".to_string()),
            bar_block_val,
        );

        let foo_block = gc!(
            mc,
            Block {
                source_info: None,
                name: block_foo.name.clone(),
                is_nested_block: block_foo.is_nested_block,
                param_syms: block_foo.param_syms.clone(),
                param_types: block_foo.param_types.clone(),
                bytecode: block_foo.bytecode.clone(),
                parent_env: None,
                enclosing_method_id: None,
                decl_block: None,
                source_map: SharedSourceMap::from(Vec::new()),
            }
        );
        vm.start_block(mc, foo_block, Vec::new(), None, None);
        vm
    });

    arena.mutate_root(|mc, vm| {
        // Step 1: Inside foo: LoadGlobal(bar_func)
        vm.step(mc).unwrap();
        // Step 2: Inside foo: Push(block_nested)
        vm.step(mc).unwrap();
        // Step 3: Inside foo: Send(value:) -> starts block_bar frame
        vm.step(mc).unwrap();
        assert_eq!(vm.frames.len(), 2);
        assert_eq!(vm.frames[1].block.name, Some("bar".to_string()));

        // Step 4: Inside bar: LoadLocal(blk)
        vm.step(mc).unwrap();
        // Step 5: Inside bar: Send(value) -> starts block_nested frame
        vm.step(mc).unwrap();
        assert_eq!(vm.frames.len(), 3);
        assert_eq!(vm.frames[2].block.name, Some("nested".to_string()));

        // Step 6: Inside nested: Push(777)
        vm.step(mc).unwrap();
        // Step 7: Inside nested: MethodReturn -> unwinds nested, bar, and foo frames!
        vm.step(mc).unwrap();

        // All frames should be unwound.
        assert_eq!(vm.frames.len(), 0);
        assert_eq!(stack_spec(vm), vec![ValueSpec::Int(777)]);
    });
}

#[test]
fn test_class_and_method_definition_vm() {
    let class_block = StaticBlock {
        source_info: None,
        name: Some("class_block".to_string()),
        is_nested_block: false,
        param_syms: Vec::new(),
        param_types: Vec::new(),
        bytecode: SharedBytecode::from(vec![
            // 1. Define inst method x
            Instruction::Push(Constant::Block(StaticBlock {
                source_info: None,
                name: Some("x".to_string()),
                is_nested_block: false,
                param_syms: Vec::new(),
                param_types: Vec::new(),
                bytecode: vec![
                    Instruction::LoadLocal(Symbol::intern("self")),
                    Instruction::Return,
                ]
                .into(),
                decl_block: None,
                source_map: Vec::new().into(),
            })),
            Instruction::DefineMethod("x".to_string()),
            // 2. Override inst method x
            Instruction::Push(Constant::Block(StaticBlock {
                source_info: None,
                name: Some("x".to_string()),
                is_nested_block: false,
                param_syms: Vec::new(),
                param_types: Vec::new(),
                bytecode: vec![Instruction::Push(Constant::Int(42)), Instruction::Return].into(),
                decl_block: None,
                source_map: Vec::new().into(),
            })),
            Instruction::OverrideMethod("x".to_string()),
            Instruction::Return,
        ]),
        decl_block: None,
        source_map: SharedSourceMap::from(Vec::new()),
    };

    run_test_steps(
        vec![
            // Define class Point
            Instruction::DefineClass {
                name: NamespacedName::new(Vec::new(), "Point".to_string()),
                parent_name: None,
                instance_vars: vec!["x".to_string(), "y".to_string()],
            },
            // Push class block
            Instruction::Push(Constant::Block(class_block)),
            // Execute block with Point as self
            Instruction::ExecuteBlockWithSelf,
            // Send "meta" to Point
            Instruction::LoadGlobal(NamespacedName::new(Vec::new(), "Point".to_string())),
            Instruction::Send(Symbol::intern("meta"), 0),
        ],
        |vm, mc| {
            // Step DefineClass
            vm.step(mc).unwrap();
            let class_val = vm.peek().unwrap();
            if let Value::Class(c) = class_val {
                assert_eq!(c.borrow().name.to_string(), "Point");
                assert_eq!(
                    c.borrow().instance_vars,
                    vec!["x".to_string(), "y".to_string()]
                );
            } else {
                panic!("Expected Class value");
            }

            // Step Push Block
            vm.step(mc).unwrap();
            // Step ExecuteBlockWithSelf -> frame for class_block starts
            vm.step(mc).unwrap();
            assert_eq!(vm.frames.len(), 2);
            assert_eq!(
                EnvFrame::get(vm.frames[1].env, self_symbol()).unwrap(),
                class_val
            );

            // Inside class_block: Push(x_block)
            vm.step(mc).unwrap();
            // Inside class_block: DefineMethod("x")
            vm.step(mc).unwrap();

            // Verify method x exists in instance_methods
            if let Value::Class(c) = class_val {
                assert!(c.borrow().instance_methods.contains_key("x"));
            }

            // Inside class_block: Push(override_x_block)
            vm.step(mc).unwrap();
            // Inside class_block: OverrideMethod("x")
            vm.step(mc).unwrap();

            // Inside class_block: Return -> pops class_block frame, pushes Nil to main stack
            vm.step(mc).unwrap();
            assert_eq!(vm.frames.len(), 1);

            // Step LoadGlobal Point
            vm.step(mc).unwrap();
            // Step Send meta
            vm.step(mc).unwrap();

            // Stack should have [Point, Nil, ClassMeta(Point)]
            let meta_val = vm.peek().unwrap();
            if let Value::ClassMeta(c) = meta_val {
                assert_eq!(c.borrow().name.to_string(), "Point");
            } else {
                panic!("Expected ClassMeta, got {:?}", meta_val);
            }
        },
    );
}

#[test]
fn test_class_method_lookup_fallback() {
    run_test_steps(
        vec![
            Instruction::LoadGlobal(NamespacedName::new(Vec::new(), "Point".to_string())),
            Instruction::Send(Symbol::intern("name"), 0),
        ],
        |vm, mc| {
            let point_class = gcl!(
                mc,
                Class {
                    name: NamespacedName::new(Vec::new(), "Point".to_string()),
                    parent: None,
                    instance_vars: Vec::new(),
                    instance_methods: HashMap::new(),
                    class_methods: HashMap::new(),
                    mixin_classes: Vec::new(),
                    field_slots: HashMap::new(),
                    is_eigenclass: false,
                    is_sealed: false,
                    is_abstract: false,
                }
            );
            vm.globals.borrow_mut(mc).insert(
                NamespacedName::new(Vec::new(), "Point".to_string()),
                Value::Class(point_class),
            );

            vm.step(mc).unwrap();
            assert_eq!(vm.stack.len(), 1);

            vm.step(mc).unwrap();
            assert_eq!(vm.stack.len(), 1);

            assert_eq!(stack_spec(vm), vec![ValueSpec::String("Point".to_string())]);
        },
    );
}

#[test]
fn test_primitive_methods_and_overrides() {
    let custom_true_method = StaticBlock {
        source_info: None,
        name: Some("custom_true_method".to_string()),
        is_nested_block: false,
        param_syms: Vec::new(),
        param_types: Vec::new(),
        bytecode: SharedBytecode::from(vec![
            Instruction::Push(Constant::Int(42)),
            Instruction::Return,
        ]),
        decl_block: None,
        source_map: SharedSourceMap::from(Vec::new()),
    };

    let class_extension_block = StaticBlock {
        source_info: None,
        name: Some("class_extension_block".to_string()),
        is_nested_block: false,
        param_syms: Vec::new(),
        param_types: Vec::new(),
        bytecode: SharedBytecode::from(vec![
            Instruction::Push(Constant::Block(custom_true_method)),
            Instruction::DefineMethod("custom_true".to_string()),
            Instruction::Push(Constant::Nil),
            Instruction::Return,
        ]),
        decl_block: None,
        source_map: SharedSourceMap::from(Vec::new()),
    };

    run_test_steps(
        vec![
            Instruction::Push(Constant::Bool(true)),
            Instruction::Send(Symbol::intern("class"), 0),
            Instruction::Push(Constant::Bool(true)),
            Instruction::Push(Constant::Block(class_extension_block)),
            Instruction::ExecuteBlockWithSelf,
            Instruction::Push(Constant::Bool(true)),
            Instruction::Send(Symbol::intern("class"), 0),
            Instruction::Push(Constant::Bool(true)),
            Instruction::Send(Symbol::intern("custom_true"), 0),
            Instruction::Push(Constant::Bool(false)),
            Instruction::Send(Symbol::intern("class"), 0),
        ],
        |vm, mc| {
            vm.step(mc).unwrap();
            vm.step(mc).unwrap();
            let class_val = vm.pop().unwrap();
            if let Value::Class(c) = class_val {
                assert_eq!(c.borrow().name.to_string(), "Boolean");
            } else {
                panic!("Expected Class Boolean, got {:?}", class_val);
            }

            vm.step(mc).unwrap();
            vm.step(mc).unwrap();
            vm.step(mc).unwrap();
            assert_eq!(vm.frames.len(), 2);

            vm.step(mc).unwrap();
            vm.step(mc).unwrap();
            vm.step(mc).unwrap();
            vm.step(mc).unwrap();
            assert_eq!(vm.frames.len(), 1);
            assert_eq!(to_spec(vm.pop().unwrap()), ValueSpec::Bool(true));

            vm.step(mc).unwrap();
            vm.step(mc).unwrap();
            let class_val = vm.pop().unwrap();
            if let Value::Class(c) = class_val {
                assert_eq!(c.borrow().name.to_string(), "$TrueClass");
            } else {
                panic!("Expected Class $TrueClass, got {:?}", class_val);
            }

            vm.step(mc).unwrap();
            vm.step(mc).unwrap();
            assert_eq!(vm.frames.len(), 2);
            vm.step(mc).unwrap();
            vm.step(mc).unwrap();
            assert_eq!(vm.frames.len(), 1);
            assert_eq!(to_spec(vm.pop().unwrap()), ValueSpec::Int(42));

            vm.step(mc).unwrap();
            vm.step(mc).unwrap();
            let class_val = vm.pop().unwrap();
            if let Value::Class(c) = class_val {
                assert_eq!(c.borrow().name.to_string(), "Boolean");
            } else {
                panic!("Expected Class Boolean, got {:?}", class_val);
            }
        },
    );
}

#[test]
fn test_class_new() {
    run_test_steps(
        vec![
            Instruction::DefineClass {
                name: NamespacedName::new(Vec::new(), "Point".to_string()),
                parent_name: None,
                instance_vars: vec!["x".to_string(), "y".to_string()],
            },
            Instruction::LoadGlobal(NamespacedName::new(Vec::new(), "Point".to_string())),
            Instruction::Send(Symbol::intern("new"), 0),
        ],
        |vm, mc| {
            vm.step(mc).unwrap(); // DefineClass
            vm.step(mc).unwrap(); // LoadGlobal
            vm.step(mc).unwrap(); // Send "new"
            let obj_val = vm.pop().unwrap();
            if let Value::Object(obj) = obj_val {
                assert_eq!(obj.borrow().class.borrow().name.to_string(), "Point");
                let ob = obj.borrow();
                assert_eq!(ob.fields.len(), 2); // @x, @y
                assert!(
                    ob.fields
                        .iter()
                        .all(|v| matches!(to_spec(*v), ValueSpec::Nil))
                );
            } else {
                panic!("Expected Object, got {:?}", obj_val);
            }
        },
    );
}

#[test]
fn test_namespaced_native_class() {
    let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
        let mut vm = VmState::new(mc, VmOptions::default());

        // Build a namespaced native class [IO]File
        let file_builder = NativeClassBuilder::new("[IO]File", Some("Object"))
            .instance_method("path", |vm, mc, _receiver, _args| {
                Ok(vm.new_string(mc, "/etc/passwd".to_string()))
            });
        vm.register_native_class(mc, file_builder);
        vm
    });

    arena.mutate_root(|_mc, vm| {
        // Verify [IO]File class is in globals
        let file_key = NamespacedName::new(vec!["IO".to_string()], "File".to_string());
        let val = vm.globals.borrow().get(&file_key).copied().unwrap();
        if let Value::Class(c) = val {
            assert_eq!(c.borrow().name.to_string(), "[IO]File");
            assert_eq!(c.borrow().name.path, vec!["IO".to_string()]);
            assert_eq!(c.borrow().name.name, "File");
        } else {
            panic!("Expected Class, got {:?}", val);
        }
    });
}

#[derive(Debug)]
struct MyCustomResource {
    counter: i32,
}

#[test]
fn test_native_state_holding_rust_state() {
    let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
        let mut vm = VmState::new(mc, VmOptions::default());

        // Build native class [IO]Resource
        let resource_builder = NativeClassBuilder::new("[IO]Resource", Some("Object"))
            .class_method("create", |vm, mc, _receiver, _args| {
                let class_obj = vm.get_builtin_class("[IO]Resource");
                let state = OpaqueState(MyCustomResource { counter: 10 });
                Ok(vm.new_native_state(mc, class_obj, state))
            })
            .instance_method("get", |vm, mc, receiver, _args| {
                let val = receiver
                    .with_native_state::<MyCustomResource, _, _>(|res| res.counter)
                    .unwrap();
                Ok(vm.new_int(mc, val as i64))
            })
            .instance_method("inc:", |_vm, mc, receiver, args| {
                let val = args[0].as_i64().expect("Expected Int") as i32;
                receiver
                    .with_native_state_mut::<MyCustomResource, _, _>(mc, |res| {
                        res.counter += val;
                    })
                    .unwrap();
                Ok(receiver)
            });
        vm.register_native_class(mc, resource_builder);
        vm
    });

    arena.mutate_root(|mc, vm| {
        // Instantiate [IO]Resource via sending "create"
        let resource_class = vm.get_builtin_class("[IO]Resource");
        let instance = vm
            .call_method(mc, Value::Class(resource_class), "create", vec![])
            .unwrap();

        // Check counter is 10
        let counter_val = vm.call_method(mc, instance, "get", vec![]).unwrap();
        assert_eq!(to_spec(counter_val), ValueSpec::Int(10));

        // Increment by 5
        let five = vm.new_int(mc, 5);
        vm.call_method(mc, instance, "inc:", vec![five]).unwrap();

        // Check counter is 15
        let counter_val = vm.call_method(mc, instance, "get", vec![]).unwrap();
        assert_eq!(to_spec(counter_val), ValueSpec::Int(15));
    });
}

#[test]
fn test_mixin_method_lookup_and_instance_vars() {
    let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
        let vm = VmState::new(mc, VmOptions::default());
        vm
    });

    arena.mutate_root(|mc, vm| {
        // Define mixin class Point
        let point_class = gcl!(
            mc,
            Class {
                name: NamespacedName::new(Vec::new(), "Point".to_string()),
                parent: None,
                instance_vars: vec!["x".to_string(), "y".to_string()],
                instance_methods: {
                    let mut m = HashMap::new();
                    m.insert(
                        "name".to_string(),
                        vm.new_native_method(
                            mc,
                            "name".to_string(),
                            NativeFunc::new(|vm, mc, _receiver, _args| {
                                Ok(vm.new_string(mc, "Point".to_string()))
                            }),
                            None,
                        ),
                    );
                    m
                },
                class_methods: HashMap::new(),
                mixin_classes: Vec::new(),
                field_slots: HashMap::new(),
                is_eigenclass: false,
                is_sealed: false,
                is_abstract: false,
            }
        );

        // Define class PType which mixes in Point
        let ptype_class = gcl!(
            mc,
            Class {
                name: NamespacedName::new(Vec::new(), "PType".to_string()),
                parent: None,
                instance_vars: vec!["z".to_string()],
                instance_methods: HashMap::new(),
                class_methods: HashMap::new(),
                mixin_classes: vec![point_class],
                field_slots: HashMap::new(),
                is_eigenclass: false,
                is_sealed: false,
                is_abstract: false,
            }
        );

        // Check instance variables (should contain z, x, y)
        let vars = vm.get_all_instance_vars(ptype_class);
        assert!(vars.contains(&"x".to_string()));
        assert!(vars.contains(&"y".to_string()));
        assert!(vars.contains(&"z".to_string()));

        // Instantiate PType
        let obj = vm.new_object(mc, ptype_class);

        // Look up "name" on PType instance -> should find Point's name method
        let _method = vm
            .lookup_method(mc, Value::Object(obj), Symbol::intern("name"), &[])
            .unwrap()
            .unwrap();

        // Execute method
        let ret = vm
            .call_method(mc, Value::Object(obj), "name", vec![])
            .unwrap();
        assert_eq!(to_spec(ret), ValueSpec::String("Point".to_string()));
    });
}

#[test]
fn test_execute_block_helper() {
    let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
        let vm = VmState::new(mc, VmOptions::default());
        vm
    });

    arena.mutate_root(|mc, vm| {
        // Build a block that adds two arguments (a, b) and returns self + a + b
        let block = gc!(
            mc,
            Block {
                source_info: None,
                name: Some("test_block".to_string()),
                is_nested_block: false,
                param_syms: crate::value::intern_param_syms(&vec![
                    "a".to_string(),
                    "b".to_string()
                ]),
                param_types: vec!["Object".to_string(), "Object".to_string()],
                bytecode: SharedBytecode::from(vec![
                    Instruction::LoadLocal(Symbol::intern("self")),
                    Instruction::LoadLocal(Symbol::intern("a")),
                    Instruction::Send(Symbol::intern("+"), 1),
                    Instruction::LoadLocal(Symbol::intern("b")),
                    Instruction::Send(Symbol::intern("+"), 1),
                    Instruction::Return,
                ]),
                parent_env: None,
                enclosing_method_id: None,
                decl_block: None,
                source_map: SharedSourceMap::from(Vec::new()),
            }
        );

        // Register standard native functions we need (+ operator)
        let native_val = vm.new_native_method(mc, "+".to_string(), NativeFunc(native_add), None);
        vm.globals
            .borrow_mut(mc)
            .insert(NamespacedName::new(Vec::new(), "+".to_string()), native_val);

        // Execute block with args [10, 20] and self = 100
        let self_val = vm.new_int(mc, 100);
        let arg1 = vm.new_int(mc, 10);
        let arg2 = vm.new_int(mc, 20);

        let res = vm
            .execute_block(mc, block, vec![arg1, arg2], Some(self_val))
            .unwrap();

        assert_eq!(to_spec(res), ValueSpec::Int(130));

        // Execute block without self: a + b
        let block2 = gc!(
            mc,
            Block {
                source_info: None,
                name: Some("test_block_no_self".to_string()),
                is_nested_block: false,
                param_syms: crate::value::intern_param_syms(&vec![
                    "a".to_string(),
                    "b".to_string()
                ]),
                param_types: vec!["Object".to_string(), "Object".to_string()],
                bytecode: SharedBytecode::from(vec![
                    Instruction::LoadLocal(Symbol::intern("a")),
                    Instruction::LoadLocal(Symbol::intern("b")),
                    Instruction::Send(Symbol::intern("+"), 1),
                    Instruction::Return,
                ]),
                parent_env: None,
                enclosing_method_id: None,
                decl_block: None,
                source_map: SharedSourceMap::from(Vec::new()),
            }
        );

        let res2 = vm
            .execute_block(mc, block2, vec![arg1, arg2], None)
            .unwrap();

        assert_eq!(to_spec(res2), ValueSpec::Int(30));
    });
}

#[test]
fn test_cannot_redefine_existing_class() {
    run_test_steps(
        vec![Instruction::DefineClass {
            name: NamespacedName::new(Vec::new(), "Object".to_string()),
            parent_name: None,
            instance_vars: Vec::new(),
        }],
        |vm, mc| {
            let res = vm.step(mc);
            assert!(res.is_err());
            let err_msg = format!("{}", res.err().unwrap());
            assert!(err_msg.contains("Cannot redefine class [/]Object because it already exists"));
        },
    );
}

#[test]
fn test_cannot_extend_non_existent_class() {
    run_test_steps(
        vec![
            Instruction::Push(Constant::Nil),
            Instruction::Push(Constant::Block(StaticBlock {
                source_info: None,
                name: Some("ext_block".to_string()),
                is_nested_block: false,
                param_syms: Vec::new(),
                param_types: Vec::new(),
                bytecode: SharedBytecode::from(vec![
                    Instruction::Push(Constant::Nil),
                    Instruction::Return,
                ]),
                decl_block: None,
                source_map: SharedSourceMap::from(Vec::new()),
            })),
            Instruction::ExecuteBlockWithSelf,
        ],
        |vm, mc| {
            let res = vm.step(mc);
            assert!(res.is_ok());
            let res = vm.step(mc);
            assert!(res.is_ok());
            let res = vm.step(mc);
            assert!(res.is_err());
            let err_msg = format!("{}", res.err().unwrap());
            assert!(err_msg.contains("Cannot extend nil or non-existent class/object"));
        },
    );
}

#[test]
fn test_short_circuit_and_or() {
    // Test: false && (panic!)
    run_test_steps(
        vec![
            Instruction::Push(Constant::Bool(false)),
            Instruction::Dup,
            Instruction::ElseJump(3),
            Instruction::Pop,
            Instruction::Push(Constant::Int(99)),
        ],
        |vm, mc| {
            vm.step(mc).unwrap(); // Push false
            vm.step(mc).unwrap(); // Dup
            vm.step(mc).unwrap(); // ElseJump (should jump to end, i.e., index 5)
            assert_eq!(stack_spec(vm), vec![ValueSpec::Bool(false)]);
            assert_eq!(vm.frames[0].ip, 5);
        },
    );

    // Test: true && 42
    run_test_steps(
        vec![
            Instruction::Push(Constant::Bool(true)),
            Instruction::Dup,
            Instruction::ElseJump(3),
            Instruction::Pop,
            Instruction::Push(Constant::Int(42)),
        ],
        |vm, mc| {
            vm.step(mc).unwrap(); // Push true
            vm.step(mc).unwrap(); // Dup
            vm.step(mc).unwrap(); // ElseJump (should not jump, IP becomes 3)
            assert_eq!(stack_spec(vm), vec![ValueSpec::Bool(true)]);
            assert_eq!(vm.frames[0].ip, 3);
            vm.step(mc).unwrap(); // Pop
            assert_eq!(vm.stack.len(), 0);
            vm.step(mc).unwrap(); // Push 42
            assert_eq!(stack_spec(vm), vec![ValueSpec::Int(42)]);
        },
    );

    // Test: true || (panic!)
    run_test_steps(
        vec![
            Instruction::Push(Constant::Bool(true)),
            Instruction::Dup,
            Instruction::IfJump(3),
            Instruction::Pop,
            Instruction::Push(Constant::Int(99)),
        ],
        |vm, mc| {
            vm.step(mc).unwrap(); // Push true
            vm.step(mc).unwrap(); // Dup
            vm.step(mc).unwrap(); // IfJump (should jump to end, i.e. index 5)
            assert_eq!(stack_spec(vm), vec![ValueSpec::Bool(true)]);
            assert_eq!(vm.frames[0].ip, 5);
        },
    );

    // Test: false || 42
    run_test_steps(
        vec![
            Instruction::Push(Constant::Bool(false)),
            Instruction::Dup,
            Instruction::IfJump(3),
            Instruction::Pop,
            Instruction::Push(Constant::Int(42)),
        ],
        |vm, mc| {
            vm.step(mc).unwrap(); // Push false
            vm.step(mc).unwrap(); // Dup
            vm.step(mc).unwrap(); // IfJump (should not jump, IP becomes 3)
            assert_eq!(stack_spec(vm), vec![ValueSpec::Bool(false)]);
            assert_eq!(vm.frames[0].ip, 3);
            vm.step(mc).unwrap(); // Pop
            assert_eq!(vm.stack.len(), 0);
            vm.step(mc).unwrap(); // Push 42
            assert_eq!(stack_spec(vm), vec![ValueSpec::Int(42)]);
        },
    );
}

#[test]
fn test_error_annotation_and_display() {
    use crate::compiler::Compiler;
    use crate::parser::parse_quoin_string;

    let code = "1.foo;";
    let ast = parse_quoin_string(code);
    let mut compiler = Compiler::new();
    let compiled = compiler
        .compile_program(match &ast.value {
            NodeValue::Program(p) => p,
            _ => unreachable!(),
        })
        .unwrap();

    let mut arena =
        Arena::<Rootable![VmState<'_>]>::new(|mc| VmState::new(mc, VmOptions::default()));

    arena.mutate_root(|mc, vm| {
        let decl_block = compiled.decl_block.as_ref().map(|db| {
            gc!(
                mc,
                Block {
                    source_info: db.source_info.clone(),
                    name: db.name.clone(),
                    is_nested_block: db.is_nested_block,
                    param_syms: db.param_syms.clone(),
                    param_types: db.param_types.clone(),
                    bytecode: db.bytecode.clone(),
                    parent_env: None,
                    enclosing_method_id: None,
                    decl_block: None,
                    source_map: db.source_map.clone(),
                }
            )
        });
        let block = gc!(
            mc,
            Block {
                source_info: compiled.source_info.clone(),
                name: compiled.name.clone(),
                is_nested_block: compiled.is_nested_block,
                param_syms: compiled.param_syms.clone(),
                param_types: compiled.param_types.clone(),
                bytecode: compiled.bytecode.clone(),
                parent_env: None,
                enclosing_method_id: None,
                decl_block,
                source_map: compiled.source_map.clone(),
            }
        );
        vm.start_block(mc, block, Vec::new(), None, None);

        // Run until error. It should fail because Integer/Nil does not have 'foo' method.
        let mut err = None;
        loop {
            match vm.step(mc) {
                Ok(VmStatus::Running) => {}
                Ok(_) => break,
                Err(e) => {
                    err = Some(e);
                    break;
                }
            }
        }

        let err = err.expect("Expected execution error");
        let err_str = err.to_string();

        // Check that the error message displays the source information
        assert!(err_str.contains("at <string>:1:1"));
        assert!(err_str.contains("1.foo"));
    });
}

#[test]
fn test_error_annotation_with_color() {
    use crate::compiler::Compiler;
    use crate::parser::parse_quoin_string;

    let code = "1.foo;";
    let ast = parse_quoin_string(code);
    let mut compiler = Compiler::new();
    let compiled = compiler
        .compile_program(match &ast.value {
            NodeValue::Program(p) => p,
            _ => unreachable!(),
        })
        .unwrap();

    let mut options = VmOptions::default();
    options.supports_color = true;

    let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| VmState::new(mc, options));

    arena.mutate_root(|mc, vm| {
        let decl_block = compiled.decl_block.as_ref().map(|db| {
            gc!(
                mc,
                Block {
                    source_info: db.source_info.clone(),
                    name: db.name.clone(),
                    is_nested_block: db.is_nested_block,
                    param_syms: db.param_syms.clone(),
                    param_types: db.param_types.clone(),
                    bytecode: db.bytecode.clone(),
                    parent_env: None,
                    enclosing_method_id: None,
                    decl_block: None,
                    source_map: db.source_map.clone(),
                }
            )
        });
        let block = gc!(
            mc,
            Block {
                source_info: compiled.source_info.clone(),
                name: compiled.name.clone(),
                is_nested_block: compiled.is_nested_block,
                param_syms: compiled.param_syms.clone(),
                param_types: compiled.param_types.clone(),
                bytecode: compiled.bytecode.clone(),
                parent_env: None,
                enclosing_method_id: None,
                decl_block,
                source_map: compiled.source_map.clone(),
            }
        );
        vm.start_block(mc, block, Vec::new(), None, None);

        // Run until error.
        let mut err = None;
        loop {
            match vm.step(mc) {
                Ok(VmStatus::Running) => {}
                Ok(_) => break,
                Err(e) => {
                    err = Some(e);
                    break;
                }
            }
        }

        let err = err.expect("Expected execution error");
        let err_str = err.to_string();

        // Check that the error message contains the ANSI escape codes
        // Selector is purple (38;2;171;130;255)
        // Filename text has colon gray (38;2;128;128;128)
        // Numbers are light blue (38;2;0;191;255)
        assert!(err_str.contains("\x1b[38;2;171;130;255mfoo\x1b[0;00;22;39;49m"));
        assert!(err_str.contains("\x1b[38;2;0;191;255m1\x1b[0;00;22;39;49m"));
    });
}

#[test]
fn test_error_annotation_with_console_width() {
    use crate::compiler::Compiler;
    use crate::parser::parse_quoin_string;

    let code = "1.foo;";
    let ast = parse_quoin_string(code);
    let mut compiler = Compiler::new();
    let compiled = compiler
        .compile_program(match &ast.value {
            NodeValue::Program(p) => p,
            _ => unreachable!(),
        })
        .unwrap();

    let mut options = VmOptions::default();
    options.console_width = Some(120);

    let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| VmState::new(mc, options));

    arena.mutate_root(|mc, vm| {
        let block = gc!(
            mc,
            Block {
                source_info: compiled.source_info.clone(),
                name: compiled.name.clone(),
                is_nested_block: compiled.is_nested_block,
                param_syms: compiled.param_syms.clone(),
                param_types: compiled.param_types.clone(),
                bytecode: compiled.bytecode.clone(),
                parent_env: None,
                enclosing_method_id: None,
                decl_block: None,
                source_map: compiled.source_map.clone(),
            }
        );
        vm.start_block(mc, block, Vec::new(), None, None);

        // Run until error.
        let mut err = None;
        loop {
            match vm.step(mc) {
                Ok(VmStatus::Running) => {}
                Ok(_) => break,
                Err(e) => {
                    err = Some(e);
                    break;
                }
            }
        }

        let err = err.expect("Expected execution error");
        assert!(matches!(err, QuoinError::WithSourceInfo { .. }));
    });
}

#[test]
fn highlighted_snippet_does_not_panic_on_truncated_source() {
    // Regression: the error annotator highlights a frame's source. When the frame's filename
    // isn't a readable file (REPL/eval frames use synthetic names like `<repl>`), it falls
    // back to `source_text` truncated to the available width `w`. A truncation that cuts an
    // expression in half yields an unparseable fragment, and the old code fed it to the
    // panicking `parse_quoin_string` — crashing the REPL the moment a long line errored
    // (e.g. `([HTTP]Client.get:'https://…').pp.print;` cut to `([HTTP]Client`). The fix
    // routes through the resilient highlighter, which never panics.
    let mut options = VmOptions::default();
    options.supports_color = true;
    let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| VmState::new(mc, options));

    arena.mutate_root(|_mc, vm| {
        let source = "([HTTP]Client.get:'https://quoinlang.dev/').pp.print;".to_string();
        // `w = 13` slices the source to `([HTTP]Client` — exactly the mid-expression cut that
        // used to panic. `<repl>` is not a real file, so the `source_text` branch is taken.
        let out = vm.get_highlighted_snippet("<repl>", 0, 0, 0, source.len(), Some(&source), 13);
        let out = out.expect("snippet should be produced, not a panic");
        // The highlighter preserves text exactly, so the colors strip back to the truncation.
        let expected: String = source.chars().take(13).collect();
        assert_eq!(crate::ansi_colorizer::decolorize(&out), expected);
    });
}

#[test]
fn test_vm_to_s() {
    let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
        let mut vm = VmState::new(mc, VmOptions::default());
        vm.register_native_class(mc, build_object_class());
        vm.register_native_class(mc, build_class_class());
        vm.register_native_class(mc, build_boolean_class());
        vm.register_native_class(mc, build_block_class());
        vm.register_native_class(mc, build_list_class());
        vm.register_native_class(mc, build_double_class());
        vm.register_native_class(mc, build_integer_class());
        vm.register_native_class(mc, build_string_class());
        vm.register_native_class(mc, build_nil_class());
        vm.register_native_class(mc, build_map_class());
        vm.register_native_class(mc, build_key_value_pair_class());
        vm.register_native_class(mc, build_regex_class());
        for t in ["Method"] {
            vm.register_native_class(mc, NativeClassBuilder::new(t, Some("Object")));
        }
        vm
    });

    arena.mutate_root(|mc, vm| {
        // Test 1: Value::Class Display Output
        let string_class = vm.get_builtin_class("String");
        let class_val = Value::Class(string_class);
        let result = vm.to_s(mc, class_val).unwrap();
        assert_eq!(
            to_spec(result),
            ValueSpec::String("class String".to_string())
        );

        // Test 2: Value::ClassMeta Display Output
        let class_meta_val = Value::ClassMeta(string_class);
        let result = vm.to_s(mc, class_meta_val).unwrap();
        assert_eq!(
            to_spec(result),
            ValueSpec::String("class String meta".to_string())
        );

        // Test 3: Value::Object (Int / String / Nil / Bool)
        let int_val = vm.new_int(mc, 42);
        let result = vm.to_s(mc, int_val).unwrap();
        assert_eq!(to_spec(result), ValueSpec::String("42".to_string()));

        let bool_val = vm.new_bool(mc, true);
        let result = vm.to_s(mc, bool_val).unwrap();
        assert_eq!(to_spec(result), ValueSpec::String("true".to_string()));

        let nil_val = vm.new_nil(mc);
        let result = vm.to_s(mc, nil_val).unwrap();
        assert_eq!(to_spec(result), ValueSpec::String("nil".to_string()));

        let string_val = vm.new_string(mc, "hello".to_string());
        let result = vm.to_s(mc, string_val).unwrap();
        assert_eq!(to_spec(result), ValueSpec::String("hello".to_string()));
    });
}

#[test]
fn test_vm_options_at_runtime() {
    let options = VmOptions {
        arguments: vec!["foo".to_string(), "bar".to_string()],
        supports_color: true,
        console_width: None,
    };

    let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
        let mut vm = VmState::new(mc, options);
        vm.register_native_class(mc, build_object_class());
        vm.register_native_class(mc, build_class_class());
        vm.register_native_class(mc, build_boolean_class());
        vm.register_native_class(mc, build_block_class());
        vm.register_native_class(mc, build_list_class());
        vm.register_native_class(mc, build_double_class());
        vm.register_native_class(mc, build_integer_class());
        vm.register_native_class(mc, build_string_class());
        vm.register_native_class(mc, build_nil_class());
        vm.register_native_class(mc, build_map_class());
        vm.register_native_class(mc, build_key_value_pair_class());
        vm.register_native_class(mc, build_regex_class());
        vm.register_native_class(mc, crate::runtime::runtime::build_runtime_class());
        for t in ["Method"] {
            vm.register_native_class(mc, NativeClassBuilder::new(t, Some("Object")));
        }
        vm
    });

    arena.mutate_root(|mc, vm| {
        // Check that Runtime.arguments returns the list ["foo", "bar"]
        let runtime_class = vm.get_builtin_class("Runtime");
        let args_val = vm
            .call_method(mc, Value::Class(runtime_class), "arguments", vec![])
            .unwrap();

        // args_val should be a List of strings
        let count_val = vm.call_method(mc, args_val, "count", vec![]).unwrap();
        assert_eq!(to_spec(count_val), ValueSpec::Int(2));

        let idx0 = vm.new_int(mc, 0);
        let arg0 = vm.call_method(mc, args_val, "at:", vec![idx0]).unwrap();
        assert_eq!(to_spec(arg0), ValueSpec::String("foo".to_string()));

        let idx1 = vm.new_int(mc, 1);
        let arg1 = vm.call_method(mc, args_val, "at:", vec![idx1]).unwrap();
        assert_eq!(to_spec(arg1), ValueSpec::String("bar".to_string()));

        // Check options method
        let opts_val = vm
            .call_method(mc, Value::Class(runtime_class), "options", vec![])
            .unwrap();
        // opts_val should be a Map
        let key = vm.new_string(mc, "arguments".to_string());
        let mapped_args = vm.call_method(mc, opts_val, "at:", vec![key]).unwrap();

        let mapped_count = vm.call_method(mc, mapped_args, "count", vec![]).unwrap();
        assert_eq!(to_spec(mapped_count), ValueSpec::Int(2));

        // Check supportsColor method
        let supports_color_val = vm
            .call_method(mc, Value::Class(runtime_class), "supportsColor", vec![])
            .unwrap();
        assert_eq!(to_spec(supports_color_val), ValueSpec::Bool(true));

        // Check options map has supports_color
        let key_color = vm.new_string(mc, "supports_color".to_string());
        let mapped_color = vm
            .call_method(mc, opts_val, "at:", vec![key_color])
            .unwrap();
        assert_eq!(to_spec(mapped_color), ValueSpec::Bool(true));
    });
}
