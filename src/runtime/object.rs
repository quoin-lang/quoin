use crate::error::QuoinError;
use crate::runtime::pretty;
use crate::value::{NativeClassBuilder, ObjectPayload, Value};

pub fn build_object_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Object", None)
        .abstract_class()
        .class_doc(
            "The universal root: every value is an Object, and every class inherits from it. \
             It carries the protocol everything shares -- identity and equality (`==:` / `!=:` / \
             `~:`), rendering (`s` / `pp` / `print`), reflection (`class` / `can?:` / `doc` / \
             `docFor:` / `perform:args:`), and raising a value as an error (`throw`).",
        )
        // The hash-code half of the key contract (docs in `value_hash_scalar`):
        // scalars answer their structural hash; instances default to IDENTITY
        // (gc-arena is non-moving, so the pointer is stable), matching
        // Object's identity `==:`. A class that overrides `==:` with value
        // semantics must override `hash` to match — equal values must hash
        // equal, or map lookups miss (never corrupt).
        .instance_method("hash", |vm, mc, receiver, _args| {
            let h = match crate::value::value_hash_scalar(&receiver) {
                Some(h) => h,
                None => match receiver {
                    Value::Object(obj) => crate::value::hash_i64(gc_arena::Gc::as_ptr(obj) as i64),
                    _ => 0,
                },
            };
            Ok(vm.new_int(mc, h as i64))
        })
        .returns("Integer")
        .doc(
            "The receiver's hash code. Scalars hash structurally; instances default to \
             IDENTITY, matching Object's identity `==:`. A class that overrides `==:` with \
             value semantics must override `hash` to match -- equal values must hash equal, \
             or map lookups miss.",
        )
        // Reflective send: `obj.perform:'add:' args:#( 3 )`. Raises the same
        // MessageNotUnderstood a direct send would (the legacy nil-for-absent
        // convention of the internal call_method helper does NOT apply here).
        // Added for the WorkerService host loop; generally useful reflection.
        .instance_method("perform:args:", |vm, mc, receiver, args| {
            let sel = match args[0] {
                Value::Object(obj) => match &obj.borrow().payload {
                    crate::value::ObjectPayload::String(s) => (**s).clone(),
                    _ => {
                        return Err(QuoinError::Other(
                            "perform:args: expects a String selector".into(),
                        ));
                    }
                },
                _ => {
                    return Err(QuoinError::Other(
                        "perform:args: expects a String selector".into(),
                    ));
                }
            };
            let call_args: Vec<Value> = if let Value::Nil = args[1] {
                Vec::new()
            } else {
                args[1]
                    .with_native_state::<crate::runtime::list::NativeListState, _, _>(|l| {
                        l.get_vec().to_vec()
                    })
                    .map_err(|_| {
                        QuoinError::Other("perform:args: expects a List of arguments".into())
                    })?
            };
            let symbol = crate::symbol::Symbol::intern(&sel);
            if vm
                .lookup_method(mc, receiver, symbol, &call_args)?
                .is_none()
            {
                let candidates = vm
                    .collect_method_candidates(receiver, symbol)
                    .iter()
                    .map(|&mv| vm.format_candidate_signature(mv, symbol))
                    .collect();
                return Err(QuoinError::MessageNotUnderstood {
                    receiver: receiver.class_name(),
                    selector: sel,
                    args: call_args.iter().map(|a| a.class_name()).collect(),
                    candidates,
                });
            }
            vm.call_method(mc, receiver, &sel, call_args)
        })
        .doc(
            "Send a selector reflectively: the String selector, with the List's elements as \
             arguments (nil is an empty argument list). Raises the same MessageNotUnderstood \
             a direct send would.\n\n\
             ```\n\
             3.perform:'+:' args:#( 4 )    \"* -> 7\n\
             ```",
        )
        // The default `.s` for a value with no intrinsic human form: fall back to the
        // structural `.pp`. The Rust Display impl is for Rust-level debugging only — no Quoin
        // `.s` routes through it. (Types with an intrinsic form — Integer, String, Error, … —
        // override this.)
        .sdk_instance_method("s", |host, receiver, _args| {
            let width = host
                .options()
                .console_width
                .map(|w| w as usize)
                .unwrap_or(80);
            // Methods return plain text; color is a REPL display concern (the `=>` path).
            Ok(host.new_string(pretty::render(receiver, width, false)))
        })
        .returns("String")
        .doc(
            "The receiver rendered for humans, as a String. The default for a value with no \
             intrinsic form falls back to the structural `pp` rendering; types with one \
             (Integer, String, Error, ...) override it.\n\n\
             ```\n\
             A <- { |@x @y| init -> { @x = 1; @y = 2 } };\n\
             A.new.s    \"* -> A{@x: 1 @y: 2}\n\
             ```",
        )
        // `pp` — a structural, canonical dump of the value graph for debugging/inspection
        // (escaped strings, instance vars, intrinsic collections). Width-aware: defaults to the
        // console width; `pp:` takes an explicit width. Never calls `.s`.
        .sdk_instance_method("pp", |host, receiver, _args| {
            let width = host
                .options()
                .console_width
                .map(|w| w as usize)
                .unwrap_or(80);
            // Methods return plain text; color is a REPL display concern (the `=>` path).
            Ok(host.new_string(pretty::render(receiver, width, false)))
        })
        .returns("String")
        .doc(
            "A structural, canonical dump of the value graph for debugging and inspection -- \
             escaped strings, instance variables, intrinsic collections. Width-aware (wraps \
             to the console width; `pp:` takes an explicit width) and never calls `s`.\n\n\
             ```\n\
             'hi'.pp        \"* -> 'hi'\n\
             #( 1 2 ).pp    \"* -> #(1 2)\n\
             ```",
        )
        .sdk_instance_method("pp:", |host, receiver, args| {
            let width = match args.first() {
                Some(Value::Int(w)) if *w > 0 => *w as usize,
                _ => 80,
            };
            // Methods return plain text; color is a REPL display concern (the `=>` path).
            Ok(host.new_string(pretty::render(receiver, width, false)))
        })
        .doc(
            "As `pp`, but wrapped to the given width (a positive Integer) instead of the \
             console's.",
        )
        .instance_method("sealed!", |vm, mc, receiver, _args| {
            // Seal an instance: get-or-create its eigenclass and freeze it, so further
            // `<--` on this instance is refused. (`Class#sealed!` handles class
            // receivers; for a value type this targets the type's shared class, matching
            // how `value <-- {…}` extends it.)
            let tc = vm
                .get_target_class_for_def(mc, receiver)
                .map_err(QuoinError::Other)?;
            tc.borrow_mut(mc).is_sealed = true;
            Ok(receiver)
        })
        .doc(
            "Seal the receiver against further extension: `value <-- { ... }` on it is \
             refused afterwards. On an instance this freezes its eigenclass; on a value type \
             (an Integer, a String, ...) it targets the type's shared class, matching how \
             `value <-- { ... }` extends it. Answers the receiver. (`Class#sealed!` handles \
             class receivers.)",
        )
        .instance_method("class", |vm, _mc, receiver, _args| {
            if let Some(c) = vm.get_class_for_lookup(receiver) {
                Ok(Value::Class(c))
            } else {
                Err(QuoinError::Other(format!(
                    "Class not found for type {}",
                    receiver.type_name()
                )))
            }
        })
        .doc(
            "The receiver's class, as a Class object.\n\n\
             ```\n\
             3.class.name    \"* -> Integer\n\
             ```",
        )
        // `can?:` is overloaded by argument:
        //   - a Symbol or String selector -> does the receiver implement that method?
        //     (instance methods for an instance or class receiver; class-side
        //     methods for a metaclass receiver)
        //   - a Class -> is the receiver an instance of / does it mix in that class?
        .instance_method("can?:", |vm, mc, receiver, args| {
            let cap = args[0];
            let responds = if let Value::Class(c) = cap {
                vm.is_instance_of(receiver, c)
            } else {
                let name = match cap {
                    Value::Object(obj) => match &obj.borrow().payload {
                        ObjectPayload::Symbol(s) | ObjectPayload::String(s) => Some((**s).clone()),
                        _ => None,
                    },
                    _ => None,
                };
                let name = name.ok_or_else(|| QuoinError::TypeError {
                    expected: "Symbol, String, or Class".to_string(),
                    got: cap.type_name().to_string(),
                    msg: "can?: expects a selector (symbol or string) or a class".to_string(),
                })?;
                match receiver {
                    Value::Class(c) => vm.lookup_in_class_hierarchy(c, &name, false).is_some(),
                    Value::ClassMeta(c) => vm.lookup_in_class_hierarchy(c, &name, true).is_some(),
                    // Object + immediate value types: dispatch via the derived class.
                    _ => match vm.get_class_for_lookup(receiver) {
                        Some(class) => vm.lookup_in_class_hierarchy(class, &name, false).is_some(),
                        None => false,
                    },
                }
            };
            Ok(vm.new_bool(mc, responds))
        })
        .doc(
            "Whether the receiver 'can do' the argument -- overloaded by argument. With a \
             Symbol or String selector: does the receiver implement that method? (A Class \
             receiver reports its INSTANCE methods; ask `.meta.can?:` for class-side ones.) \
             With a Class: is the receiver an instance of it -- mixins and superclasses \
             included?\n\n\
             ```\n\
             List.can?:#add:    \"* -> true\n\
             3.can?:Integer     \"* -> true\n\
             3.can?:String      \"* -> false\n\
             ```",
        )
        // The reference-doc query surface (docs/DOCS_ARCH.md §6): read-only, lazy — Quoin docs
        // live in source (the `"*` block above the definition) and are extracted on demand;
        // native docs come from the builder's `.doc(..)`/`.class_doc(..)` metadata. On Object,
        // like `can?:`, so a Class, a `.meta`, and an instance all answer.
        .instance_method("doc", |vm, mc, receiver, _args| {
            let class = match receiver {
                Value::Class(c) | Value::ClassMeta(c) => Some(c),
                _ => vm.get_class_for_lookup(receiver),
            };
            let doc = class
                .map(|c| c.borrow().name.to_string())
                .and_then(|name| crate::introspect::doc_of_class(vm, &name));
            Ok(match doc {
                Some(text) => vm.new_string(mc, text),
                None => vm.new_nil(mc),
            })
        })
        .returns("String?")
        .doc(
            "The receiver's class-level reference doc, or nil. Mirrors `can?:`: a Class \
             answers for itself, an instance for its class.",
        )
        // docFor: follows can?:'s receiver convention exactly: a Class receiver answers for
        // INSTANCE methods, `.meta` for class-side ones (see qnlib/tests/17-can.qn).
        .instance_method("docFor:", |vm, mc, receiver, args| {
            let name = match args[0] {
                Value::Object(obj) => match &obj.borrow().payload {
                    ObjectPayload::Symbol(s) | ObjectPayload::String(s) => Some((**s).clone()),
                    _ => None,
                },
                _ => None,
            };
            let name = name.ok_or_else(|| QuoinError::TypeError {
                expected: "Symbol or String".to_string(),
                got: args[0].type_name().to_string(),
                msg: "docFor: expects a selector (symbol or string)".to_string(),
            })?;
            let doc = match receiver {
                Value::Class(c) => crate::introspect::doc_of_method(vm, c, &name, false),
                Value::ClassMeta(c) => crate::introspect::doc_of_method(vm, c, &name, true),
                _ => vm
                    .get_class_for_lookup(receiver)
                    .and_then(|c| crate::introspect::doc_of_method(vm, c, &name, false)),
            };
            Ok(match doc {
                Some(text) => vm.new_string(mc, text),
                None => vm.new_nil(mc),
            })
        })
        .returns("String?")
        .doc(
            "The reference doc for a selector, or nil. A Class receiver answers for instance \
             methods; `.meta.docFor:` for class-side ones -- the same sides `can?:` reports.",
        )
        .sdk_instance_method("~:", |host, receiver, args| {
            host.call_method(receiver, "==:", vec![args[0]])
        })
        .doc(
            "The match operator, `pattern ~ subject` -- it dispatches on the LEFT operand \
             (the pattern). Object's default is plain equality (delegates to `==:`); pattern \
             kinds override it: a Class matches its instances (`Integer ~ 3`), a Block runs \
             as a predicate, a Regex matches strings. `case:`'s `when:` clauses match with \
             it.\n\n\
             ```\n\
             3 ~ 3          \"* -> true\n\
             Integer ~ 3    \"* -> true\n\
             ```",
        )
        .sdk_instance_method("==:", |host, receiver, args| {
            let lhs = receiver;
            let rhs = args[0];
            Ok(host.new_bool(lhs == rhs))
        })
        .doc(
            "Equality. Object's default is IDENTITY -- true only when both sides are the \
             very same object (scalars compare by value). Value classes override it \
             (numbers compare across Integer/Float, collections structurally); any `==:` \
             override must be matched by a `hash` override.\n\n\
             ```\n\
             A <- {}; var a = A.new;\n\
             a == a            \"* -> true\n\
             A.new == A.new    \"* -> false\n\
             ```",
        )
        .sdk_instance_method("!=:", |host, receiver, args| {
            let lhs = receiver;
            let rhs = args[0];

            let eq_result = host.call_method(lhs, "==:", vec![rhs])?;
            let false_val = host.new_bool(false);
            Ok(host.new_bool(eq_result == false_val))
        })
        .doc(
            "The negation of `==:`, defined once here -- overriding `==:` is enough to get \
             `!=` right.\n\n\
             ```\n\
             1 != 2    \"* -> true\n\
             ```",
        )
        .sdk_instance_method("init", |_host, receiver, _args| Ok(receiver))
        .doc(
            "The default initializer: does nothing and answers the receiver. Instantiation \
             runs it on each fresh instance; classes define their own `init` (or keyword \
             `init:` forms) to set up state.",
        )
        .sdk_instance_method("print", |host, receiver, _args| {
            let s_result = host.call_method(receiver, "s", vec![])?;
            let text = match s_result {
                Value::Object(obj) => match &obj.borrow().payload {
                    ObjectPayload::String(string) => string.to_string(),
                    _ => format!("{}", s_result),
                },
                x => format!("{}", x),
            };
            // Route through the VM's stdout sink (not `println!`) so the DAP adapter can capture
            // program output as `output` events instead of it hitting fd 1/2 directly.
            host.write_std(crate::vm::StdStream::Out, format!("{text}\n").as_bytes())
                .map_err(|e| QuoinError::Other(e.to_string()))?;
            Ok(host.new_nil())
        })
        .doc(
            "Render the receiver with `s` and write it to standard output with a trailing \
             newline. Answers nil.\n\n\
             ```\n\
             'hello'.print    \"* prints hello\n\
             ```",
        )
        .sdk_instance_method("throw", |host, receiver, _args| {
            host.set_active_exception(receiver);
            Err(QuoinError::Thrown)
        })
        .doc(
            "Raise the receiver as an exception: unwind to the nearest enclosing `catch:` \
             whose handler matches (by the handler parameter's declared type). Any value can \
             be thrown, not just Error instances.\n\n\
             ```\n\
             { 'boom'.throw }.catch:{ |e| e }    \"* -> boom\n\
             ```",
        )
}
