
use super::*;
use crate::parser::ast::*;
use crate::parser::parse_quoin_string;
use crate::value::NamespacedName;

use std::sync::Arc;

fn ns(name: &str) -> NamespacedName {
    NamespacedName::parse(name)
}

// Helpers to easily construct Nodes
fn int(value: i64) -> Node {
    Node {
        source_info: None,
        value: NodeValue::Integer(IntegerNode { value }),
    }
}

fn double(value: f64) -> Node {
    Node {
        source_info: None,
        value: NodeValue::Double(DoubleNode { value }),
    }
}

fn string(value: &str) -> Node {
    Node {
        source_info: None,
        value: NodeValue::Str(StringNode {
            value: value.to_string(),
        }),
    }
}

fn sym(value: &str) -> Node {
    Node {
        source_info: None,
        value: NodeValue::Symbol(SymbolNode {
            value: value.to_string(),
        }),
    }
}

fn local_id(name: &str) -> Node {
    Node {
        source_info: None,
        value: NodeValue::Identifier(IdentifierNode {
            source_info: None,
            namespace: None,
            name: name.to_string(),
            identifier_type: IdentifierType::Local,
        }),
    }
}

// Builds a `var` declaration. First-binding compilation is now `var` (a bare
// assignment to an undeclared local is a strict-mode error — tested separately in
// `strict_declaration_semantics`). A fresh `var` binding emits the same
// Dup/DefineLocal bytecode the old implicit first-assignment did.
fn assign_node(lvals: Vec<Node>, rval: Node) -> Node {
    Node {
        source_info: None,
        value: NodeValue::Declaration(DeclarationNode {
            kind: DeclKind::Var,
            lvalues: lvals.into_iter().map(Arc::new).collect(),
            type_hint: None,
            rvalue: Arc::new(rval),
        }),
    }
}

#[test]
fn resolver_flags_unknown_types() {
    fn diags(src: &str) -> Vec<String> {
        let node = crate::parser::parse_quoin_string(src);
        let NodeValue::Program(p) = &node.value else {
            panic!("expected a program");
        };
        let mut c = Compiler::new();
        c.compile_program(p).unwrap();
        c.diagnostics().iter().map(|d| d.message.clone()).collect()
    }

    // Builtins resolve silently — in a return type and in a param type.
    assert!(diags("Foo <- { greet -> { |^String| ^^ 'hi' } }").is_empty());
    assert!(diags("Foo <- { take -> { |n: Integer| ^^ n } }").is_empty());

    // An unknown class is flagged (non-fatal: compilation still succeeds).
    let d = diags("Foo <- { make -> { |^Widget| ^^ nil } }");
    assert_eq!(d.len(), 1, "{d:?}");
    assert!(d[0].contains("unknown type `Widget`"), "{d:?}");
    // …and in a param type.
    assert!(diags("Foo <- { take -> { |g: Gadget| ^^ g } }")[0].contains("Gadget"));

    // `T?` is flagged iff its base is unknown.
    assert!(diags("Foo <- { make -> { |^Widget?| ^^ nil } }")[0].contains("Widget"));
    assert!(diags("Foo <- { make -> { |^String?| ^^ nil } }").is_empty());

    // A class defined anywhere in the unit is known — forward reference via the pre-scan.
    // (`^Widget?` so the `nil` body is a valid return, not a nil-misuse.)
    assert!(diags("Foo <- { make -> { |^Widget?| ^^ nil } }; Widget <- { }").is_empty());
}

#[test]
fn records_declared_method_returns_from_ast() {
    // Compile `src` and return the recorded returns for class `name` (Phase 3c·4a).
    fn returns_of(src: &str, name: &str) -> HashMap<Arc<str>, Type> {
        let node = parse_quoin_string(src);
        let NodeValue::Program(p) = &node.value else {
            panic!("expected a program");
        };
        let mut c = Compiler::new();
        c.compile_program(p).unwrap();
        c.class_table.get(name).unwrap().method_returns
    }

    // A `^Ret` header on a `Foo <- {}` method is recorded; a header-less method is not.
    let r = returns_of("Foo <- { make -> { |^Integer| 5 }; plain -> { 1 } }", "Foo");
    assert_eq!(r.get("make"), Some(&Type::Int));
    assert_eq!(r.get("plain"), None);

    // A `Foo <-- {}` reopen contributes its declared returns too (how the core classes do it).
    let r = returns_of("Foo <- { }; Foo <-- { grow -> { |^String| 'x' } }", "Foo");
    assert_eq!(r.get("grow"), Some(&Type::String));
}

#[test]
fn checker_flags_return_covariance() {
    fn diags(src: &str) -> Vec<String> {
        let node = parse_quoin_string(src);
        let NodeValue::Program(p) = &node.value else {
            panic!("expected a program");
        };
        let mut c = Compiler::new();
        c.compile_program(p).unwrap();
        c.diagnostics()
            .iter()
            .map(|d| d.message.clone())
            .filter(|m| m.contains("override of"))
            .collect()
    }

    // Subclassing is `Parent <- Child <- { }`. Dog <: Animal, and B <: A below.
    // Widening an inherited return is a violation: base `get:` returns `Dog`, the override
    // returns `Animal` — a supertype, not usable where `Dog` is expected.
    let d = diags(
        "Animal <- { }; Animal <- Dog <- { }; \
             A <- { get: -> { |x ^Dog| x } }; A <- B <- { get: -> { |x ^Animal| x } }",
    );
    assert_eq!(d.len(), 1, "{d:?}");
    assert!(d[0].contains("override of `get:`") && d[0].contains("incompatible"));

    // Narrowing to a subtype is fine (covariant returns): base `Animal`, override `Dog`.
    let d = diags(
        "Animal <- { }; Animal <- Dog <- { }; \
             A <- { get: -> { |x ^Animal| x } }; A <- B <- { get: -> { |x ^Dog| x } }",
    );
    assert!(d.is_empty(), "{d:?}");

    // A confident scalar mismatch is flagged (base `String`, override `Integer`).
    let d = diags("A <- { n -> { |^String| 'x' } }; A <- B <- { n -> { |^Integer| 5 } }");
    assert_eq!(d.len(), 1, "{d:?}");

    // Same scalar return is silent.
    let d = diags("A <- { n -> { |^String| 'x' } }; A <- B <- { n -> { |^String| 'y' } }");
    assert!(d.is_empty(), "{d:?}");
}

#[test]
fn defined_guard_inlines_directly_when_object_contract_is_known() {
    fn bytecode(seed_object: bool) -> Vec<Instruction> {
        let node = parse_quoin_string("var x = 5; x.defined?.if:{ 1 } else:{ 2 }");
        let NodeValue::Program(p) = &node.value else {
            panic!("expected a program");
        };
        let mut c = Compiler::new();
        if seed_object {
            // Simulate the loaded bootstrap contract `Object#defined? : Boolean`.
            let mut r = HashMap::new();
            r.insert(Arc::from("defined?"), Type::Bool);
            c.class_table.add_returns("Object", r);
        }
        c.compile_program(p)
            .unwrap()
            .bytecode
            .0
            .iter()
            .cloned()
            .collect()
    }
    let has_guard = |bc: &[Instruction]| {
        bc.iter()
            .any(|i| matches!(i, Instruction::BranchIfNotBool(_)))
    };

    // Without the contract `x.defined?` is `Any` → a *guarded* inline (a runtime Bool-check
    // that falls back to the real send for a non-Bool receiver).
    assert!(
        has_guard(&bytecode(false)),
        "expected a guarded inline without the Object contract"
    );
    // With `Object#defined? : Boolean` known, covariance makes `x.defined?` statically
    // `Boolean`, so the guard devirt-inlines as a *direct* Bool conditional — no runtime check.
    assert!(
        !has_guard(&bytecode(true)),
        "expected a direct inline with the Object contract"
    );
}

#[test]
fn checker_flags_return_mismatches() {
    fn diags(src: &str) -> Vec<String> {
        let node = crate::parser::parse_quoin_string(src);
        let NodeValue::Program(p) = &node.value else {
            panic!("expected a program");
        };
        let mut c = Compiler::new();
        c.compile_program(p).unwrap();
        c.diagnostics().iter().map(|d| d.message.clone()).collect()
    }

    // A confident return mismatch is flagged (non-fatal).
    assert!(
        diags("F <- { m -> { |^Integer| 'x' } }")[0].contains("expected `Integer`, found `String`")
    );
    // Correct returns are silent.
    assert!(diags("F <- { m -> { |^Integer| 40 + 2 } }").is_empty());
    assert!(diags("F <- { m -> { |^String| 'hi' } }").is_empty());
    // Nullable: `nil` satisfies `T?`.
    assert!(diags("F <- { m -> { |^Integer?| nil } }").is_empty());
    // Numeric literal promotion: an Integer literal where a Double is declared is fine…
    assert!(diags("F <- { m -> { |^Double| 1 } }").is_empty());
    // …but a non-constant Integer where a Double is expected is flagged (strict signatures).
    assert!(
        diags("F <- { m: -> { |n: Integer ^Double| n } }")[0]
            .contains("expected `Double`, found `Integer`")
    );
    // An explicit `^` return is checked too.
    assert!(diags("F <- { m -> { |^Integer| ^'x' } }")[0].contains("found `String`"));
}

#[test]
fn checker_flags_decl_mismatches() {
    fn diags(src: &str) -> Vec<String> {
        let node = crate::parser::parse_quoin_string(src);
        let NodeValue::Program(p) = &node.value else {
            panic!("expected a program");
        };
        let mut c = Compiler::new();
        c.compile_program(p).unwrap();
        c.diagnostics().iter().map(|d| d.message.clone()).collect()
    }

    assert!(diags("var x: Integer = 'hi'")[0].contains("expected `Integer`, found `String`"));
    assert!(diags("var x: Integer = 5").is_empty());
    // Numeric literal promotion applies to initializers too.
    assert!(diags("var x: Double = 1").is_empty());
    assert!(diags("var x: String = 'hi'").is_empty());
    // Nullable: `nil` satisfies `T?`.
    assert!(diags("var x: Integer? = nil").is_empty());
    // A typed decl now resolves its annotation, so an unknown type is flagged.
    assert!(diags("var x: Nope = 5")[0].contains("unknown type `Nope`"));
}

#[test]
fn checker_subtyping_via_class_table() {
    fn diags(src: &str) -> Vec<String> {
        let node = crate::parser::parse_quoin_string(src);
        let NodeValue::Program(p) = &node.value else {
            panic!("expected a program");
        };
        let mut c = Compiler::new();
        c.compile_program(p).unwrap();
        c.diagnostics().iter().map(|d| d.message.clone()).collect()
    }

    // `Animal <- Dog` makes Dog a subtype of Animal — a Dog where an Animal is wanted is fine.
    assert!(
        diags("Animal <- { }; Animal <- Dog <- { }; var d: Dog = Dog.new; var a: Animal = d")
            .is_empty()
    );
    // Unrelated classes in the same hierarchy still mismatch.
    assert!(
            diags("Animal <- { }; Animal <- Dog <- { }; Animal <- Cat <- { }; var d: Dog = Dog.new; var c: Cat = d")[0]
                .contains("expected `Cat`, found `Dog`")
        );
}

#[test]
fn checker_flags_typed_reassignment() {
    fn diags(src: &str) -> Vec<String> {
        let node = crate::parser::parse_quoin_string(src);
        let NodeValue::Program(p) = &node.value else {
            panic!("expected a program");
        };
        let mut c = Compiler::new();
        c.compile_program(p).unwrap();
        c.diagnostics().iter().map(|d| d.message.clone()).collect()
    }

    // Reassigning an *annotated* var to a wrong type is flagged.
    assert!(diags("var x: Integer = 5; x = nil")[0].contains("expected `Integer`, found `Nil`"));
    assert!(
        diags("var x: Integer = 5; x = 'hi'")[0].contains("expected `Integer`, found `String`")
    );
    // Correct type — and a promotable Integer literal into a Double var — are silent.
    assert!(diags("var x: Integer = 5; x = 6").is_empty());
    assert!(diags("var x: Double = 1.0; x = 2").is_empty());
    // An UN-annotated var is untyped: its inferred type is a devirt hint, not a contract, so
    // reassigning it to any type is fine (the `optimisticIntFallback` corpus pattern).
    assert!(diags("var x = 5; x = 'hi'").is_empty());
}

#[test]
fn narrowing_overlay_reads_innermost_scope() {
    // 3c·0 plumbing: the narrowing overlay stores per-scope refinements; innermost wins, and
    // an absent key falls through (gradual). No rules install narrowing yet, so this exercises
    // the store/lookup directly.
    let mut c = Compiler::new();
    let x = NarrowKey::Local("x".to_string());
    assert_eq!(c.narrowed_type(&x), None);

    c.scopes
        .last_mut()
        .unwrap()
        .narrowed
        .insert(x.clone(), Type::Int);
    assert_eq!(c.narrowed_type(&x), Some(Type::Int));

    // A pushed inner scope still sees the outer narrowing…
    c.push_scope(HashSet::new());
    assert_eq!(c.narrowed_type(&x), Some(Type::Int));
    // …but its own narrowing shadows it.
    c.scopes
        .last_mut()
        .unwrap()
        .narrowed
        .insert(x.clone(), Type::String);
    assert_eq!(c.narrowed_type(&x), Some(Type::String));

    // An absent key stays `None`.
    assert_eq!(c.narrowed_type(&NarrowKey::Field("y".to_string())), None);
}

#[test]
fn checker_narrows_nullable_after_defined_guard() {
    // Narrowing is observable through the decl check: `var y: Integer = x` type-checks only
    // where `x: Integer?` has been narrowed non-nil.
    fn diags(src: &str) -> Vec<String> {
        let node = crate::parser::parse_quoin_string(src);
        let NodeValue::Program(p) = &node.value else {
            panic!("expected a program");
        };
        let mut c = Compiler::new();
        c.compile_program(p).unwrap();
        c.diagnostics()
            .iter()
            .filter(|d| d.message.contains("type mismatch"))
            .map(|d| d.message.clone())
            .collect()
    }

    // Unguarded: assigning a nullable to an `Integer` local → warns.
    assert!(!diags("Foo <- { m -> { |x: Integer?| var y: Integer = x } }").is_empty());
    // Guarded true-arm narrows `x` non-nil, so the arm's decl type-checks.
    assert!(
        diags("Foo <- { m -> { |x: Integer?| x.defined?.if:{ var y: Integer = x } } }").is_empty()
    );
    // Divergent nil-arm: after `.else:{ ^^0 }`, `x` is non-nil for the rest of the body.
    assert!(
        diags("Foo <- { m -> { |x: Integer?| x.defined?.else:{ ^^0 }; var y: Integer = x } }")
            .is_empty()
    );
    // Reassignment re-narrows a declared nullable local: `x = 5` makes it non-nil.
    assert!(
        diags("Foo <- { m -> { var x: Integer? = nil; x = 5; var y: Integer = x } }").is_empty()
    );
}

#[test]
fn type_join_is_the_nil_lattice_lub() {
    use Type::*;
    let opt = |t: Type| Nullable(Box::new(t));
    assert_eq!(Int.join(&Int), Int);
    assert_eq!(Int.join(&Nil), opt(Int)); // T ⊔ Nil = T?
    assert_eq!(Nil.join(&Int), opt(Int));
    assert_eq!(Int.join(&opt(Int)), opt(Int)); // T ⊔ T? = T?
    assert_eq!(opt(Int).join(&Nil), opt(Int));
    assert_eq!(Nil.join(&Nil), Nil);
    assert_eq!(Int.join(&Bool), Any); // two concrete cores, no union
    assert_eq!(Int.join(&Any), Any); // Any absorbing
    assert_eq!(Never.join(&Int), Int); // diverging path contributes nothing
    assert_eq!(Int.join(&Never), Int);
}

#[test]
fn checker_joins_arm_exits_after_a_guard() {
    fn diags(src: &str) -> Vec<String> {
        let node = crate::parser::parse_quoin_string(src);
        let NodeValue::Program(p) = &node.value else {
            panic!("expected a program");
        };
        let mut c = Compiler::new();
        c.compile_program(p).unwrap();
        c.diagnostics()
            .iter()
            .filter(|d| d.message.contains("type mismatch"))
            .map(|d| d.message.clone())
            .collect()
    }
    // Every body opens with a declared-nullable local `x` (reassignment updates its narrowing).
    let m = |body: &str| format!("Foo <- {{ m -> {{ var x: Integer? = nil; {body} }} }}");

    // Both paths leave x non-nil (if-arm guard, else-arm reassignment) → Integer at the join.
    assert!(diags(&m("x.defined?.if:{ } else:{ x = 0 }; var y: Integer = x")).is_empty());
    // Both arms reassign to non-nil → non-nil after, regardless of the guard branch taken.
    assert!(
        diags(&m(
            "x.defined?.if:{ x = 1 } else:{ x = 2 }; var y: Integer = x"
        ))
        .is_empty()
    );
    // A diverging arm drops out of the join → the surviving path's refinement holds after.
    assert!(diags(&m("x.defined?.else:{ ^^0 }; var y: Integer = x")).is_empty());

    // Empty else leaves x nil on the false path → x stays nullable, so the decl warns.
    assert!(!diags(&m("x.defined?.if:{ } else:{ }; var y: Integer = x")).is_empty());
    // `if:`-only: the condition-false fall-through leaves x nil → still nullable after.
    assert!(!diags(&m("x.defined?.if:{ x = 7 }; var y: Integer = x")).is_empty());
}

#[test]
fn typed_param_is_a_declared_contract() {
    fn diags(src: &str) -> Vec<String> {
        let node = crate::parser::parse_quoin_string(src);
        let NodeValue::Program(p) = &node.value else {
            panic!("expected a program");
        };
        let mut c = Compiler::new();
        c.compile_program(p).unwrap();
        c.diagnostics()
            .iter()
            .filter(|d| d.message.contains("type mismatch"))
            .map(|d| d.message.clone())
            .collect()
    }

    // A typed param's annotation is a reassignment contract, like a `var x: T` local:
    // an incompatible reassignment warns…
    assert!(!diags("Foo <- { m -> { |x: Integer| x = 'str'; x } }").is_empty());
    // …a compatible one is silent.
    assert!(diags("Foo <- { m -> { |x: Integer| x = x + 1; x } }").is_empty());
    // Reassigning a nullable param to a concrete value narrows it non-nil.
    assert!(diags("Foo <- { m -> { |x: Integer?| x = 5; var y: Integer = x; y } }").is_empty());
    // The arm-exit join now works for a nullable *param* too (reassignment flow-updates it).
    assert!(
            diags("Foo <- { m -> { |x: Integer?| x.defined?.if:{ } else:{ x = 0 }; var y: Integer = x; y } }")
                .is_empty()
        );
}

#[test]
fn object_annotation_is_the_top_type() {
    // `Object` (and `Object?`) is the universal top → the gradual `Any`, never a concrete class.
    assert_eq!(Type::from_annotation_name("Object"), Type::Any);
    assert_eq!(Type::from_annotation_name("Object?"), Type::Any);

    fn diags(src: &str) -> Vec<String> {
        let node = crate::parser::parse_quoin_string(src);
        let NodeValue::Program(p) = &node.value else {
            panic!("expected a program");
        };
        let mut c = Compiler::new();
        c.compile_program(p).unwrap();
        c.diagnostics()
            .iter()
            .filter(|d| d.message.contains("type mismatch"))
            .map(|d| d.message.clone())
            .collect()
    }
    // `Object` constrains nothing — no false positive on a decl or a param reassignment.
    assert!(diags("Foo <- { m -> { var x: Object = 5; x } }").is_empty());
    assert!(diags("Foo <- { m -> { |x: Object| x = 'y'; x } }").is_empty());
    // A real annotation still constrains.
    assert!(!diags("Foo <- { m -> { var x: Integer = 'no'; x } }").is_empty());
}

#[test]
fn object_rooted_s_and_pp_type_as_string() {
    fn diags(src: &str, seed: bool) -> Vec<String> {
        let node = parse_quoin_string(src);
        let NodeValue::Program(p) = &node.value else {
            panic!("expected a program");
        };
        let mut c = Compiler::new();
        if seed {
            // Simulate the native `Object` contracts seeded by `populate_from_vm`.
            let mut r = HashMap::new();
            r.insert(Arc::from("s"), Type::String);
            r.insert(Arc::from("pp"), Type::String);
            c.class_table.add_returns("Object", r);
        }
        c.compile_program(p).unwrap();
        c.diagnostics()
            .iter()
            .filter(|d| d.message.contains("type mismatch"))
            .map(|d| d.message.clone())
            .collect()
    }
    // With the contract, `x.s`/`x.pp` are `String` for any receiver.
    assert!(diags("Foo <- { m -> { |x| var t: String = x.s; t } }", true).is_empty());
    assert!(!diags("Foo <- { m -> { |x| var n: Integer = x.s; n } }", true).is_empty());
    assert!(!diags("Foo <- { m -> { |x| var n: Integer = x.pp; n } }", true).is_empty());
    // Without it they're `Any` → gradual, silent either way (no false positive).
    assert!(diags("Foo <- { m -> { |x| var n: Integer = x.s; n } }", false).is_empty());
}

#[test]
fn checker_flags_nil_misuse() {
    fn diags(src: &str) -> Vec<String> {
        let node = crate::parser::parse_quoin_string(src);
        let NodeValue::Program(p) = &node.value else {
            panic!("expected a program");
        };
        let mut c = Compiler::new();
        c.compile_program(p).unwrap();
        c.diagnostics()
            .iter()
            .filter(|d| d.message.contains("may be nil"))
            .map(|d| d.message.clone())
            .collect()
    }

    // A non-nil-safe send to an un-narrowed nullable → warns.
    assert!(!diags("Foo <- { m -> { |x: Integer?| x.abs } }").is_empty());
    // Guarded: `x` is narrowed non-nil in the arm → silent.
    assert!(diags("Foo <- { m -> { |x: Integer?| x.defined?.if:{ x.abs } } }").is_empty());
    // Nil-safe methods don't dereference → silent even on a nullable.
    assert!(diags("Foo <- { m -> { |x: Integer?| x.s } }").is_empty());
    // Non-nullable, and gradual `Any`, receivers → silent.
    assert!(diags("Foo <- { m -> { |x: Integer| x.abs } }").is_empty());
    assert!(diags("Foo <- { m -> { |x| x.abs } }").is_empty());
}

#[test]
fn checker_nil_misuse_binops_and_conditions() {
    fn diags(src: &str) -> Vec<String> {
        let node = crate::parser::parse_quoin_string(src);
        let NodeValue::Program(p) = &node.value else {
            panic!("expected a program");
        };
        let mut c = Compiler::new();
        c.compile_program(p).unwrap();
        c.diagnostics()
            .iter()
            .filter(|d| d.message.contains("may be nil"))
            .map(|d| d.message.clone())
            .collect()
    }

    // Binop nil-misuse: `x + 1` dereferences a nullable left → warns; `==` is nil-safe.
    assert!(!diags("Foo <- { m -> { |x: Integer?| x + 1 } }").is_empty());
    assert!(diags("Foo <- { m -> { |x: Integer?| x == 1 } }").is_empty());

    // `!= nil` / `== nil` guard conditions narrow their arms.
    assert!(diags("Foo <- { m -> { |x: Integer?| (x != nil).if:{ x + 1 } } }").is_empty());
    assert!(
        diags("Foo <- { m -> { |x: Integer?| (x == nil).if:{ 0 } else:{ x + 1 } } }").is_empty()
    );

    // `&&` short-circuit narrows the RHS.
    assert!(diags("Foo <- { m -> { |x: Integer?| x.defined? && (x + 1) } }").is_empty());
}

#[test]
fn strict_declaration_semantics() {
    fn compile_src(src: &str) -> Result<StaticBlock, String> {
        let node = crate::parser::parse_quoin_string(src);
        let NodeValue::Program(p) = &node.value else {
            panic!("expected a program");
        };
        Compiler::new().compile_program(p)
    }

    // `var` declares; a later plain assignment reassigns the same binding.
    assert!(compile_src("var x = 5; x = 6").is_ok());
    assert!(compile_src("var a b = #(1 2); a b = #(3 4)").is_ok());
    assert!(compile_src("var f = { |n| n * f.value: 1 }").is_ok()); // recursive self-ref

    // A bare assignment to an undeclared local is a strict-mode error.
    let e = compile_src("z = 10").unwrap_err();
    assert!(e.contains("undeclared local"), "{e}");

    // A `let` binding cannot be reassigned.
    let e = compile_src("let w = 1; w = 2").unwrap_err();
    assert!(e.contains("let"), "{e}");

    // Re-declaring a name in the same scope is an error.
    let e = compile_src("var d = 1; var d = 2").unwrap_err();
    assert!(e.contains("already declared"), "{e}");

    // `var`/`let` cannot declare an instance variable.
    let e = compile_src("var @x = 1").unwrap_err();
    assert!(e.contains("instance variable"), "{e}");
}

#[test]
fn sealed_leaf_self_send_is_inlined() {
    fn compile_src(src: &str) -> StaticBlock {
        let node = crate::parser::parse_quoin_string(src);
        let NodeValue::Program(p) = &node.value else {
            panic!("expected a program");
        };
        Compiler::new().compile_program(p).unwrap()
    }
    // Does any send-family instruction dispatch `sel` (recursing into nested blocks)?
    fn dispatches(sb: &StaticBlock, sel: Symbol) -> bool {
        sb.bytecode.0.iter().any(|inst| match inst {
            Instruction::Send(s, _)
            | Instruction::SendLocal(_, s, _)
            | Instruction::SendConst(_, s, _)
            | Instruction::SendField(_, s, _)
            | Instruction::SendLocalLocal(_, _, s, _)
            | Instruction::SendLocalConst(_, _, s, _) => *s == sel,
            Instruction::Push(Constant::Block(nested)) => dispatches(nested, sel),
            _ => false,
        })
    }
    let width = Symbol::intern("width");

    // Sealed class, leaf accessor `width -> { @width }` self-sent → inlined (never dispatched).
    let inlined = compile_src("Foo <- { width -> { @width }; getW -> { .width }; .sealed! }");
    assert!(
        !dispatches(&inlined, width),
        "sealed leaf accessor self-send should inline, not dispatch"
    );
    // Un-sealed: the callee isn't provably fixed, so the same send stays a real dispatch.
    let unsealed = compile_src("Foo <- { width -> { @width }; getW -> { .width } }");
    assert!(
        dispatches(&unsealed, width),
        "un-sealed leaf self-send must dispatch"
    );
    // Phase 5·2: a non-leaf but inline-safe body (a binop over self-sends) is also inlined; its
    // inner leaf self-sends inline in turn, so neither the computed method nor its accessors
    // dispatch.
    let computed = compile_src(
        "Foo <- { w -> { @w }; h -> { @h }; area -> { .w * .h }; go -> { .area }; .sealed! }",
    );
    assert!(
        !dispatches(&computed, Symbol::intern("area"))
            && !dispatches(&computed, Symbol::intern("w")),
        "sealed inline-safe non-leaf self-send should inline"
    );

    // A body that isn't inline-safe (contains a block ⇒ a possible `^^`) must NOT be inlined —
    // splicing it could turn a callee return into a caller return. Stays a real dispatch.
    let blocky =
        compile_src("Foo <- { pick -> { @xs.detect:{ |x| x } }; run -> { .pick }; .sealed! }");
    assert!(
        dispatches(&blocky, Symbol::intern("pick")),
        "a block-bearing body must dispatch (soundness: no inlined `^^`)"
    );
}

#[test]
fn self_send_inlining_is_depth_bounded() {
    // A self-recursive no-arg body would inline forever without the depth guard; compilation
    // must terminate, and the recursion bottoms out in a real dispatch.
    let node = crate::parser::parse_quoin_string("Foo <- { loop -> { .loop }; .sealed! }");
    let NodeValue::Program(p) = &node.value else {
        panic!("expected a program");
    };
    let sb = Compiler::new().compile_program(p).unwrap();
    // It compiled (didn't hang / overflow) and still dispatches `loop` at the bottom — as a
    // `Send`, which the peephole fuses to `SendLocal` for a `self` receiver.
    fn dispatches(sb: &StaticBlock, sel: Symbol) -> bool {
        sb.bytecode.0.iter().any(|inst| match inst {
            Instruction::Send(s, _) | Instruction::SendLocal(_, s, _) => *s == sel,
            Instruction::Push(Constant::Block(nested)) => dispatches(nested, sel),
            _ => false,
        })
    }
    assert!(dispatches(&sb, Symbol::intern("loop")));
}

#[test]
fn exact_receiver_field_accessor_is_inlined() {
    fn compile_src(src: &str) -> StaticBlock {
        let node = crate::parser::parse_quoin_string(src);
        let NodeValue::Program(p) = &node.value else {
            panic!("expected a program");
        };
        Compiler::new().compile_program(p).unwrap()
    }
    fn loads_field_of(sb: &StaticBlock, field: &str) -> bool {
        sb.bytecode.0.iter().any(|inst| match inst {
            Instruction::LoadFieldOf(f) => f == field,
            Instruction::Push(Constant::Block(nested)) => loads_field_of(nested, field),
            _ => false,
        })
    }

    // `o.x` (o: the current sealed class) reads the field directly off `o` — no dispatch.
    let inlined = compile_src("Vec <- { |@x| x -> { @x }; get: -> { |o: Vec| o.x }; .sealed! }");
    assert!(
        loads_field_of(&inlined, "x"),
        "exact-receiver accessor on the current sealed class should emit LoadFieldOf"
    );
    // Un-sealed: the receiver's class isn't provably fixed → a normal dispatch, no LoadFieldOf.
    let unsealed = compile_src("Vec <- { |@x| x -> { @x }; get: -> { |o: Vec| o.x } }");
    assert!(
        !loads_field_of(&unsealed, "x"),
        "un-sealed exact-receiver send must dispatch"
    );

    // Phase 5·3b: an accessor on a *sibling* in-unit sealed class (defined earlier) also inlines.
    let sibling = compile_src(
        "Point <- { |@x| x -> { @x }; .sealed! }; Reader <- { get: -> { |p: Point| p.x }; .sealed! }",
    );
    assert!(
        loads_field_of(&sibling, "x"),
        "accessor on a sibling in-unit sealed class should inline"
    );
    // But a *forward* reference can't (the class's body isn't recorded yet) → dispatch.
    let forward = compile_src(
        "Reader <- { get: -> { |p: Point| p.x }; .sealed! }; Point <- { |@x| x -> { @x }; .sealed! }",
    );
    assert!(
        !loads_field_of(&forward, "x"),
        "a forward-referenced class body isn't available yet → dispatch"
    );
}

#[test]
fn exact_receiver_computed_body_is_inlined() {
    fn compile_src(src: &str) -> StaticBlock {
        let node = crate::parser::parse_quoin_string(src);
        let NodeValue::Program(p) = &node.value else {
            panic!("expected a program");
        };
        Compiler::new().compile_program(p).unwrap()
    }
    fn contains(sb: &StaticBlock, pred: &dyn Fn(&Instruction) -> bool) -> bool {
        sb.bytecode.0.iter().any(|inst| {
            pred(inst) || matches!(inst, Instruction::Push(Constant::Block(b)) if contains(b, pred))
        })
    }
    // Phase 5·3c: a computed body (`@x * @y`) is spliced at the explicit-receiver `p.area`,
    // with the fields read off the receiver via `LoadFieldOf` — the method never dispatches.
    let sb = compile_src(
        "Point <- { |@x @y| area -> { @x * @y }; .sealed! }; \
             Reader <- { get: -> { |p: Point| p.area }; .sealed! }",
    );
    let area = Symbol::intern("area");
    assert!(
        !contains(
            &sb,
            &|i| matches!(i, Instruction::Send(s, _) | Instruction::SendLocal(_, s, _) if *s == area)
        ),
        "computed exact-receiver body should inline, not dispatch"
    );
    assert!(
        contains(
            &sb,
            &|i| matches!(i, Instruction::LoadFieldOf(f) if f == "x")
        ) && contains(
            &sb,
            &|i| matches!(i, Instruction::LoadFieldOf(f) if f == "y")
        ),
        "the body's @fields are read off the receiver via LoadFieldOf"
    );
}

#[test]
fn arg_passing_inlines_with_arg_methods() {
    fn compile_src(src: &str) -> StaticBlock {
        let node = crate::parser::parse_quoin_string(src);
        let NodeValue::Program(p) = &node.value else {
            panic!("expected a program");
        };
        Compiler::new().compile_program(p).unwrap()
    }
    fn dispatches(sb: &StaticBlock, sel: Symbol) -> bool {
        sb.bytecode.0.iter().any(|inst| match inst {
            Instruction::Send(s, _)
            | Instruction::SendLocal(_, s, _)
            | Instruction::SendConst(_, s, _)
            | Instruction::SendField(_, s, _)
            | Instruction::SendLocalLocal(_, _, s, _)
            | Instruction::SendLocalConst(_, _, s, _) => *s == sel,
            Instruction::Push(Constant::Block(nested)) => dispatches(nested, sel),
            _ => false,
        })
    }
    let scale = Symbol::intern("scale:");

    // Phase 5·4: a self-send WITH an arg (`.scale:2`, body `@x * k`) inlines — the arg binds to a
    // temp and `k` in the body loads it; `scale:` never dispatches.
    let selfsend =
        compile_src("C <- { |@x| scale: -> { |k| @x * k }; go -> { .scale:2 }; .sealed! }");
    assert!(
        !dispatches(&selfsend, scale),
        "self-send with an arg should inline"
    );
    // And an explicit-receiver with an arg (`v.scale:2`) inlines too.
    let exact = compile_src(
        "C <- { |@x| scale: -> { |k| @x * k }; .sealed! }; \
             R <- { go: -> { |v: C| v.scale:2 }; .sealed! }",
    );
    assert!(
        !dispatches(&exact, scale),
        "exact-receiver with an arg should inline"
    );
}

fn binary(op: BinaryOperatorType, left: Node, right: Node) -> Node {
    Node {
        source_info: None,
        value: NodeValue::BinaryOperator(BinaryOperatorNode {
            operator: op,
            left: Arc::new(left),
            right: Arc::new(right),
        }),
    }
}

fn unary(op: UnaryOperatorType, right: Node) -> Node {
    Node {
        source_info: None,
        value: NodeValue::UnaryOperator(UnaryOperatorNode {
            operator: op,
            right: Arc::new(right),
        }),
    }
}

fn call(subject: Option<Node>, selector_name: &str, args: Vec<Node>) -> Node {
    Node {
        source_info: None,
        value: NodeValue::MethodCall(MethodCallNode {
            subject: subject.map(Arc::new),
            arguments: Arc::new(MethodCallArgumentsNode {
                signature: Arc::new(MethodSelectorNode {
                    identifiers: vec![Arc::new(IdentifierNode {
                        source_info: None,
                        namespace: None,
                        name: selector_name.to_string(),
                        identifier_type: IdentifierType::Local,
                    })],
                }),
                expressions: args.into_iter().map(Arc::new).collect(),
            }),
        }),
    }
}

// Helper to compile ProgramNode
fn compile(exprs: Vec<Node>) -> Result<StaticBlock, String> {
    let mut compiler = Compiler::new();
    let program = ProgramNode {
        expressions: exprs.into_iter().map(Arc::new).collect(),
        source_info: None,
    };
    let mut block = compiler.compile_program(&program)?;
    if block.bytecode.last() == Some(&Instruction::Return) {
        Rc::make_mut(&mut block.bytecode.0).pop();
    }
    Ok(block)
}

// Default prefix for every program
fn prefix_ops() -> Vec<Instruction> {
    vec![
        Instruction::Push(Constant::Nil),
        Instruction::DefineLocal(Symbol::intern("self")),
    ]
}

// Apply the same superinstruction fusion the compiler runs, so these tests can express
// their expected bytecode as the readable *unfused* lowering and assert the compiler
// emits its fused form. (Fusion itself is pinned by the `fuse_*` tests above; for a
// snippet with no fuseable pair this is the identity.)
fn fused(v: Vec<Instruction>) -> Vec<Instruction> {
    let n = v.len();
    fuse_bytecode(v, vec![None; n]).0
}

#[test]
fn test_compile_literals() {
    let res = compile(vec![int(123)]).unwrap();
    let mut expected = prefix_ops();
    expected.push(Instruction::Push(Constant::Int(123)));
    assert_eq!(res.bytecode, fused(expected));

    let res = compile(vec![double(1.5)]).unwrap();
    let mut expected = prefix_ops();
    expected.push(Instruction::Push(Constant::Double(1.5)));
    assert_eq!(res.bytecode, fused(expected));

    let res = compile(vec![string("hello")]).unwrap();
    let mut expected = prefix_ops();
    expected.push(Instruction::Push(Constant::String("hello".to_string())));
    assert_eq!(res.bytecode, fused(expected));

    let res = compile(vec![sym("mysym")]).unwrap();
    let mut expected = prefix_ops();
    expected.push(Instruction::Push(Constant::Symbol("mysym".to_string())));
    assert_eq!(res.bytecode, fused(expected));
}

#[test]
fn test_compile_identifiers() {
    let res = compile(vec![local_id("nil")]).unwrap();
    let mut expected = prefix_ops();
    expected.push(Instruction::Push(Constant::Nil));
    assert_eq!(res.bytecode, fused(expected));

    let res = compile(vec![local_id("true")]).unwrap();
    let mut expected = prefix_ops();
    expected.push(Instruction::Push(Constant::Bool(true)));
    assert_eq!(res.bytecode, fused(expected));

    let res = compile(vec![local_id("false")]).unwrap();
    let mut expected = prefix_ops();
    expected.push(Instruction::Push(Constant::Bool(false)));
    assert_eq!(res.bytecode, fused(expected));

    // self is always local
    let res = compile(vec![local_id("self")]).unwrap();
    let mut expected = prefix_ops();
    expected.push(Instruction::LoadLocal(Symbol::intern("self")));
    assert_eq!(res.bytecode, fused(expected));

    // unknown name defaults to LoadGlobal
    let res = compile(vec![local_id("my_var")]).unwrap();
    let mut expected = prefix_ops();
    expected.push(Instruction::LoadGlobal(ns("my_var")));
    assert_eq!(res.bytecode, fused(expected));
}

#[test]
fn test_compile_assignments() {
    // Single global assignment
    let lval = Node {
        source_info: None,
        value: NodeValue::IdentLValue(IdentLValueNode {
            identifier: Arc::new(IdentifierNode {
                source_info: None,
                namespace: None,
                name: "x".to_string(),
                identifier_type: IdentifierType::Local,
            }),
        }),
    };
    let res = compile(vec![assign_node(vec![lval.clone()], int(42))]).unwrap();
    let mut expected = prefix_ops();
    expected.push(Instruction::Push(Constant::Int(42)));
    expected.push(Instruction::Dup);
    expected.push(Instruction::DefineLocal(Symbol::intern("x")));
    assert_eq!(res.bytecode, fused(expected));

    // Destructuring assignment (e.g. a b = x)
    let lval_a = Node {
        source_info: None,
        value: NodeValue::IdentLValue(IdentLValueNode {
            identifier: Arc::new(IdentifierNode {
                source_info: None,
                namespace: None,
                name: "a".to_string(),
                identifier_type: IdentifierType::Local,
            }),
        }),
    };
    let lval_b = Node {
        source_info: None,
        value: NodeValue::IdentLValue(IdentLValueNode {
            identifier: Arc::new(IdentifierNode {
                source_info: None,
                namespace: None,
                name: "b".to_string(),
                identifier_type: IdentifierType::Local,
            }),
        }),
    };
    let res = compile(vec![assign_node(vec![lval_a, lval_b], local_id("x"))]).unwrap();
    let mut expected = prefix_ops();
    expected.push(Instruction::LoadGlobal(ns("x")));
    expected.push(Instruction::Dup);
    expected.push(Instruction::DefineLocal(Symbol::intern("__qn_temp_1")));
    expected.push(Instruction::LoadLocal(Symbol::intern("__qn_temp_1")));
    expected.push(Instruction::Push(Constant::Int(0)));
    expected.push(Instruction::Send(Symbol::intern("at:"), 1));
    expected.push(Instruction::DefineLocal(Symbol::intern("a")));
    expected.push(Instruction::LoadLocal(Symbol::intern("__qn_temp_1")));
    expected.push(Instruction::Push(Constant::Int(1)));
    expected.push(Instruction::Send(Symbol::intern("at:"), 1));
    expected.push(Instruction::DefineLocal(Symbol::intern("b")));
    assert_eq!(res.bytecode, fused(expected));

    // Splat: *rest = x; (under destruct)
    let lval_rest = Node {
        source_info: None,
        value: NodeValue::SplatLValue(SplatLValueNode {
            identifier: Arc::new(IdentifierNode {
                source_info: None,
                namespace: None,
                name: "rest".to_string(),
                identifier_type: IdentifierType::Local,
            }),
        }),
    };
    let lval_ignore = Node {
        source_info: None,
        value: NodeValue::IgnoredLValue,
    };
    let res = compile(vec![assign_node(
        vec![lval_ignore, lval_rest],
        local_id("x"),
    )])
    .unwrap();
    let mut expected = prefix_ops();
    expected.push(Instruction::LoadGlobal(ns("x")));
    expected.push(Instruction::Dup);
    expected.push(Instruction::DefineLocal(Symbol::intern("__qn_temp_1")));
    expected.push(Instruction::LoadLocal(Symbol::intern("__qn_temp_1")));
    expected.push(Instruction::Push(Constant::Int(1)));
    expected.push(Instruction::Send(Symbol::intern("sliceFrom:"), 1));
    expected.push(Instruction::DefineLocal(Symbol::intern("rest")));
    assert_eq!(res.bytecode, fused(expected));

    // IgnoredSplatLValue: _ *_ = x;
    let lval_ignore = Node {
        source_info: None,
        value: NodeValue::IgnoredLValue,
    };
    let lval_ignore_splat = Node {
        source_info: None,
        value: NodeValue::IgnoredSplatLValue,
    };
    let res = compile(vec![assign_node(
        vec![lval_ignore, lval_ignore_splat],
        local_id("x"),
    )])
    .unwrap();
    let mut expected = prefix_ops();
    expected.push(Instruction::LoadGlobal(ns("x")));
    expected.push(Instruction::Dup);
    expected.push(Instruction::DefineLocal(Symbol::intern("__qn_temp_1")));
    assert_eq!(res.bytecode, fused(expected));

    // SubLValue: a (b c) = x;
    let lval_a = Node {
        source_info: None,
        value: NodeValue::IdentLValue(IdentLValueNode {
            identifier: Arc::new(IdentifierNode {
                source_info: None,
                namespace: None,
                name: "a".to_string(),
                identifier_type: IdentifierType::Local,
            }),
        }),
    };
    let lval_b = Node {
        source_info: None,
        value: NodeValue::IdentLValue(IdentLValueNode {
            identifier: Arc::new(IdentifierNode {
                source_info: None,
                namespace: None,
                name: "b".to_string(),
                identifier_type: IdentifierType::Local,
            }),
        }),
    };
    let lval_c = Node {
        source_info: None,
        value: NodeValue::IdentLValue(IdentLValueNode {
            identifier: Arc::new(IdentifierNode {
                source_info: None,
                namespace: None,
                name: "c".to_string(),
                identifier_type: IdentifierType::Local,
            }),
        }),
    };
    let lval_nested = Node {
        source_info: None,
        value: NodeValue::SubLValue(SubLValueNode {
            lvalues: vec![Arc::new(lval_b), Arc::new(lval_c)],
        }),
    };
    let res = compile(vec![assign_node(vec![lval_a, lval_nested], local_id("x"))]).unwrap();
    let mut expected = prefix_ops();
    expected.push(Instruction::LoadGlobal(ns("x")));
    expected.push(Instruction::Dup);
    expected.push(Instruction::DefineLocal(Symbol::intern("__qn_temp_1")));
    expected.push(Instruction::LoadLocal(Symbol::intern("__qn_temp_1")));
    expected.push(Instruction::Push(Constant::Int(0)));
    expected.push(Instruction::Send(Symbol::intern("at:"), 1));
    expected.push(Instruction::DefineLocal(Symbol::intern("a")));
    expected.push(Instruction::LoadLocal(Symbol::intern("__qn_temp_1")));
    expected.push(Instruction::Push(Constant::Int(1)));
    expected.push(Instruction::Send(Symbol::intern("at:"), 1));
    expected.push(Instruction::DefineLocal(Symbol::intern("__qn_temp_2")));
    expected.push(Instruction::LoadLocal(Symbol::intern("__qn_temp_2")));
    expected.push(Instruction::Push(Constant::Int(0)));
    expected.push(Instruction::Send(Symbol::intern("at:"), 1));
    expected.push(Instruction::DefineLocal(Symbol::intern("b")));
    expected.push(Instruction::LoadLocal(Symbol::intern("__qn_temp_2")));
    expected.push(Instruction::Push(Constant::Int(1)));
    expected.push(Instruction::Send(Symbol::intern("at:"), 1));
    expected.push(Instruction::DefineLocal(Symbol::intern("c")));
    assert_eq!(res.bytecode, fused(expected));
}

#[test]
fn test_compile_method_calls() {
    // x.foo: 1
    let res = compile(vec![call(Some(local_id("x")), "foo", vec![int(1)])]).unwrap();
    let mut expected = prefix_ops();
    expected.push(Instruction::LoadGlobal(ns("x")));
    expected.push(Instruction::Push(Constant::Int(1)));
    expected.push(Instruction::Send(Symbol::intern("foo:"), 1));
    assert_eq!(res.bytecode, fused(expected));

    // Implicit subject (self): .foo
    let res = compile(vec![call(None, "foo", vec![])]).unwrap();
    let mut expected = prefix_ops();
    expected.push(Instruction::LoadLocal(Symbol::intern("self")));
    expected.push(Instruction::Send(Symbol::intern("foo"), 0));
    assert_eq!(res.bytecode, fused(expected));
}

#[test]
fn test_compile_binary_unary_operators() {
    // 1 + 2  — two Integer literals devirtualize to a direct IntAdd (no method send).
    let res = compile(vec![binary(BinaryOperatorType::Add, int(1), int(2))]).unwrap();
    let mut expected = prefix_ops();
    expected.push(Instruction::Push(Constant::Int(1)));
    expected.push(Instruction::Push(Constant::Int(2)));
    expected.push(Instruction::IntAdd);
    assert_eq!(res.bytecode, fused(expected));

    // -x
    let res = compile(vec![unary(UnaryOperatorType::Sub, local_id("x"))]).unwrap();
    let mut expected = prefix_ops();
    expected.push(Instruction::LoadGlobal(ns("x")));
    expected.push(Instruction::Send(Symbol::intern("-"), 0));
    assert_eq!(res.bytecode, fused(expected));

    // !x
    let res = compile(vec![unary(UnaryOperatorType::Bang, local_id("x"))]).unwrap();
    let mut expected = prefix_ops();
    expected.push(Instruction::LoadGlobal(ns("x")));
    expected.push(Instruction::Send(Symbol::intern("!"), 0));
    assert_eq!(res.bytecode, fused(expected));

    // +x
    let res = compile(vec![unary(UnaryOperatorType::Add, local_id("x"))]).unwrap();
    let mut expected = prefix_ops();
    expected.push(Instruction::LoadGlobal(ns("x")));
    expected.push(Instruction::Send(Symbol::intern("+"), 0));
    assert_eq!(res.bytecode, fused(expected));

    // x && y
    let res = compile(vec![binary(
        BinaryOperatorType::And,
        local_id("x"),
        local_id("y"),
    )])
    .unwrap();
    let mut expected = prefix_ops();
    expected.push(Instruction::LoadGlobal(ns("x")));
    expected.push(Instruction::Dup);
    expected.push(Instruction::ElseJump(3));
    expected.push(Instruction::Pop);
    expected.push(Instruction::LoadGlobal(ns("y")));
    assert_eq!(res.bytecode, fused(expected));

    // x || y
    let res = compile(vec![binary(
        BinaryOperatorType::Or,
        local_id("x"),
        local_id("y"),
    )])
    .unwrap();
    let mut expected = prefix_ops();
    expected.push(Instruction::LoadGlobal(ns("x")));
    expected.push(Instruction::Dup);
    expected.push(Instruction::IfJump(3));
    expected.push(Instruction::Pop);
    expected.push(Instruction::LoadGlobal(ns("y")));
    assert_eq!(res.bytecode, fused(expected));
}

#[test]
fn test_compile_blocks() {
    // { |x| x + 1 }
    let block_node = BlockNode {
        return_type: None,
        source_info: None,
        name: None,
        arguments: vec![Arc::new(BlockArgNode {
            identifier: Arc::new(IdentifierNode {
                source_info: None,
                namespace: None,
                name: "x".to_string(),
                identifier_type: IdentifierType::Local,
            }),
            type_hint: None,
        })],
        decls: vec![],
        decl_block: None,
        statements: vec![Arc::new(binary(
            BinaryOperatorType::Add,
            local_id("x"),
            int(1),
        ))],
    };
    let res = compile(vec![Node {
        source_info: None,
        value: NodeValue::Block(block_node),
    }])
    .unwrap();

    // The inner block body fuses too: LoadLocal(x); Push(1); Send(+:) -> LoadLocal(x);
    // SendConst(1, +:). Fuse the readable lowering (bytecode + source map together).
    let (inner_bc, inner_sm) = fuse_bytecode(
        vec![
            Instruction::LoadLocal(Symbol::intern("x")),
            Instruction::Push(Constant::Int(1)),
            Instruction::Send(Symbol::intern("+:"), 1),
            Instruction::Return,
        ],
        vec![None; 4],
    );
    let inner_static = StaticBlock {
        name: None,
        is_nested_block: true,
        param_syms: crate::value::intern_param_syms(&vec!["x".to_string()]),
        param_types: vec!["Object".to_string()],
        bytecode: SharedBytecode(Rc::new(inner_bc)),
        source_info: None,
        decl_block: None,
        source_map: SharedSourceMap(Rc::new(inner_sm)),
    };
    let mut expected = prefix_ops();
    expected.push(Instruction::Push(Constant::Block(inner_static)));
    assert_eq!(res.bytecode, fused(expected));
}

#[test]
fn test_compile_lists_maps_regex() {
    // #(1 2)
    let list = Node {
        source_info: None,
        value: NodeValue::List(ListNode {
            values: vec![Arc::new(int(1)), Arc::new(int(2))],
        }),
    };
    let res = compile(vec![list]).unwrap();
    let mut expected = prefix_ops();
    expected.push(Instruction::Push(Constant::Int(1)));
    expected.push(Instruction::Push(Constant::Int(2)));
    expected.push(Instruction::NewList(2));
    assert_eq!(res.bytecode, fused(expected));

    // #{'a': 1}
    let map = Node {
        source_info: None,
        value: NodeValue::Map(MapNode {
            keys: vec![Arc::new(string("a"))],
            values: vec![Arc::new(int(1))],
        }),
    };
    let res = compile(vec![map]).unwrap();
    let mut expected = prefix_ops();
    expected.push(Instruction::Push(Constant::String("a".to_string())));
    expected.push(Instruction::Push(Constant::Int(1)));
    expected.push(Instruction::NewMap(1));
    assert_eq!(res.bytecode, fused(expected));

    // #/^[a-z]+$/
    let regex = Node {
        source_info: None,
        value: NodeValue::Regex(RegexNode {
            value: "#/^[a-z]+$/".to_string(),
        }),
    };
    let res = compile(vec![regex]).unwrap();
    let mut expected = prefix_ops();
    expected.push(Instruction::Push(Constant::String("^[a-z]+$".to_string())));
    expected.push(Instruction::NewRegex);
    assert_eq!(res.bytecode, fused(expected));
}

#[test]
fn test_compile_errors_and_fallbacks() {
    // Unknown NodeValue returns error
    let res = compile(vec![Node {
        source_info: None,
        value: NodeValue::Unknown,
    }]);
    assert!(res.is_err());
    assert_eq!(
        res.err().unwrap(),
        "Encountered Unknown NodeValue (ast_visitor bug)"
    );

    // Map mismatch keys/values returns error
    let map_mismatch = Node {
        source_info: None,
        value: NodeValue::Map(MapNode {
            keys: vec![Arc::new(string("a"))],
            values: vec![],
        }),
    };
    let res = compile(vec![map_mismatch]);
    assert!(res.is_err());
    assert_eq!(res.err().unwrap(), "Map keys and values count mismatch");
}

#[test]
fn test_compile_class_and_method_definitions() {
    let block_node = BlockNode {
        return_type: None,
        source_info: None,
        arguments: vec![
            Arc::new(BlockArgNode {
                identifier: Arc::new(IdentifierNode {
                    source_info: None,
                    namespace: None,
                    name: "a".to_string(),
                    identifier_type: IdentifierType::Instance,
                }),
                type_hint: None,
            }),
            Arc::new(BlockArgNode {
                identifier: Arc::new(IdentifierNode {
                    source_info: None,
                    namespace: None,
                    name: "b".to_string(),
                    identifier_type: IdentifierType::Instance,
                }),
                type_hint: None,
            }),
        ],
        decls: vec![],
        decl_block: None,
        statements: vec![],
        name: None,
    };
    let class_def = Node {
        source_info: None,
        value: NodeValue::ClassDefinition(ClassDefinitionNode {
            identifier: Arc::new(IdentifierNode {
                source_info: None,
                namespace: None,
                name: "MyClass".to_string(),
                identifier_type: IdentifierType::Local,
            }),
            parent_identifier: Some(Arc::new(IdentifierNode {
                source_info: None,
                namespace: None,
                name: "Object".to_string(),
                identifier_type: IdentifierType::Local,
            })),
            block: Arc::new(block_node.clone()),
        }),
    };

    let res = compile(vec![class_def]).unwrap();
    let expected_block = StaticBlock {
        name: None,
        is_nested_block: true,
        param_syms: crate::value::intern_param_syms(&vec!["a".to_string(), "b".to_string()]),
        param_types: vec!["Object".to_string(), "Object".to_string()],
        bytecode: SharedBytecode(Rc::new(vec![
            Instruction::Push(Constant::Nil),
            Instruction::Return,
        ])),
        source_info: None,
        decl_block: None,
        source_map: SharedSourceMap(Rc::new(vec![None; 2])),
    };
    let mut expected = prefix_ops();
    expected.push(Instruction::DefineClass {
        name: ns("MyClass"),
        parent_name: Some(ns("Object")),
        instance_vars: vec!["a".to_string(), "b".to_string()],
    });
    expected.push(Instruction::Push(Constant::Block(expected_block)));
    expected.push(Instruction::ExecuteBlockWithSelf);
    assert_eq!(res.bytecode, fused(expected));
}

#[test]
fn test_source_info_propagation() {
    let code = "{ 1 + 2 };";
    let ast = parse_quoin_string(code);
    let mut compiler = Compiler::new();

    // The root program node itself should have the source info
    if let NodeValue::Program(ref prog) = ast.value {
        let info = prog.source_info.as_ref().unwrap();
        assert_eq!(info.filename, "<string>");
        assert_eq!(info.line, 1);
        assert_eq!(info.column, 0);
        assert_eq!(
            info.source_text.as_ref().map(|s| s.as_str()),
            Some("{ 1 + 2 };")
        );
    } else {
        panic!("Expected Program node");
    }

    let compiled = compiler
        .compile_program(match &ast.value {
            NodeValue::Program(p) => p,
            _ => unreachable!(),
        })
        .unwrap();

    // The program compiled StaticBlock should have source info
    assert!(compiled.source_info.is_some());
    let prog_info = compiled.source_info.as_ref().unwrap();
    assert_eq!(prog_info.filename, "<string>");

    // Let's find the inner block pushed in the bytecode
    let mut found_inner_block = false;
    for instr in compiled.bytecode.iter().cloned() {
        if let Instruction::Push(Constant::Block(sb)) = instr {
            found_inner_block = true;
            assert!(sb.source_info.is_some());
            let info = sb.source_info.as_ref().unwrap();
            assert_eq!(info.filename, "<string>");
            assert_eq!(info.line, 1);
            assert_eq!(info.column, 0);
            assert_eq!(
                info.source_text.as_ref().map(|s| s.as_str()),
                Some("{ 1 + 2 }")
            );
        }
    }
    assert!(found_inner_block);
}

// --- superinstruction fusion (`fuse_bytecode`) ---

fn si(line: usize) -> Option<SourceInfo> {
    Some(SourceInfo {
        filename: String::new(),
        line,
        column: 0,
        start: 0,
        end: 0,
        source_text: None,
    })
}

#[test]
fn fuse_basic_operand_send_pairs() {
    let sel = Symbol::intern("foo:");
    let code = vec![
        Instruction::LoadLocal(Symbol::intern("a")),
        Instruction::Send(sel, 1),
        Instruction::Push(Constant::Int(3)),
        Instruction::Send(sel, 1),
        Instruction::LoadField("x".into()),
        Instruction::Send(sel, 1),
        Instruction::Return,
    ];
    let (out, out_smap) = fuse_bytecode(code.clone(), vec![None; code.len()]);
    assert_eq!(
        out,
        vec![
            Instruction::SendLocal(Symbol::intern("a"), sel, 1),
            Instruction::SendConst(Constant::Int(3), sel, 1),
            Instruction::SendField("x".into(), sel, 1),
            Instruction::Return,
        ]
    );
    assert_eq!(out.len(), out_smap.len());
}

#[test]
fn fuse_leaves_non_fuseable_sends_alone() {
    // A Send with no preceding fuseable operand-load stays a plain Send.
    let sel = Symbol::intern("g");
    let code = vec![Instruction::Send(sel, 0), Instruction::Return];
    let (out, _) = fuse_bytecode(code.clone(), vec![None; code.len()]);
    assert_eq!(out, code);
}

#[test]
fn fuse_does_not_cross_jump_target() {
    let sel = Symbol::intern("f");
    // The IfJump targets the Send of a (LoadLocal, Send) pair — fusing would let the
    // jump skip the LoadLocal, so it must stay unfused.
    let code = vec![
        Instruction::Push(Constant::Bool(true)),     // 0
        Instruction::IfJump(3),                      // 1 -> target 4 (the Send)
        Instruction::Push(Constant::Nil),            // 2
        Instruction::LoadLocal(Symbol::intern("a")), // 3
        Instruction::Send(sel, 1),                   // 4  (jump target)
        Instruction::Return,                         // 5
    ];
    let (out, _) = fuse_bytecode(code.clone(), vec![None; code.len()]);
    assert_eq!(out, code); // nothing fuseable here, all left intact
    let jpos = out
        .iter()
        .position(|i| matches!(i, Instruction::IfJump(_)))
        .unwrap();
    if let Instruction::IfJump(off) = out[jpos] {
        assert!(matches!(
            out[(jpos as isize + off) as usize],
            Instruction::Send(_, _)
        ));
    }
}

#[test]
fn fuse_fixes_forward_jump_offset() {
    let sel = Symbol::intern("f");
    // Jump forward *over* a fused pair: the collapsed slot shrinks the offset.
    let code = vec![
        Instruction::Push(Constant::Bool(true)),     // 0
        Instruction::IfJump(4),                      // 1 -> target 5 (Return)
        Instruction::LoadLocal(Symbol::intern("a")), // 2 \ fuse
        Instruction::Send(sel, 0),                   // 3 /
        Instruction::Pop,                            // 4
        Instruction::Return,                         // 5  (target)
    ];
    let (out, _) = fuse_bytecode(code, vec![None; 6]);
    assert_eq!(
        out,
        vec![
            Instruction::Push(Constant::Bool(true)),
            Instruction::IfJump(3),
            Instruction::SendLocal(Symbol::intern("a"), sel, 0),
            Instruction::Pop,
            Instruction::Return,
        ]
    );
    if let Instruction::IfJump(off) = out[1] {
        assert!(matches!(out[(1 + off) as usize], Instruction::Return));
    }
}

#[test]
fn fuse_fixes_backward_jump_offset() {
    let sel = Symbol::intern("f");
    // Back-edge over a fused pair at the loop top: offset grows toward 0 by one.
    let code = vec![
        Instruction::LoadLocal(Symbol::intern("a")), // 0 \ fuse (loop top)
        Instruction::Send(sel, 0),                   // 1 /
        Instruction::Push(Constant::Bool(true)),     // 2
        Instruction::IfJump(-3),                     // 3 -> target 0
        Instruction::Return,                         // 4
    ];
    let (out, _) = fuse_bytecode(code, vec![None; 5]);
    assert_eq!(
        out,
        vec![
            Instruction::SendLocal(Symbol::intern("a"), sel, 0),
            Instruction::Push(Constant::Bool(true)),
            Instruction::IfJump(-2),
            Instruction::Return,
        ]
    );
    if let Instruction::IfJump(off) = out[2] {
        assert!(matches!(
            out[(2 + off) as usize],
            Instruction::SendLocal(..)
        ));
    }
}

#[test]
fn fuse_keeps_source_map_aligned_to_send() {
    let sel = Symbol::intern("f");
    let code = vec![
        Instruction::LoadLocal(Symbol::intern("a")),
        Instruction::Send(sel, 0),
        Instruction::Return,
    ];
    let (out, out_smap) = fuse_bytecode(code, vec![si(1), si(2), si(3)]);
    assert_eq!(out.len(), out_smap.len());
    // The fused slot keeps the Send's entry (line 2), not the LoadLocal's (line 1).
    assert_eq!(out_smap[0], si(2));
    assert_eq!(out_smap[1], si(3));
}

#[test]
fn fuse_dup_store_pop_collapses_to_plain_store() {
    // Statement assignment: Dup; Store; Pop -> Store (drops Dup + Pop).
    let code = vec![
        Instruction::Push(Constant::Int(1)),
        Instruction::Dup,
        Instruction::StoreLocal(Symbol::intern("x")),
        Instruction::Pop,
        Instruction::Return,
    ];
    let (out, _) = fuse_bytecode(code, vec![None; 5]);
    assert_eq!(
        out,
        vec![
            Instruction::Push(Constant::Int(1)),
            Instruction::StoreLocal(Symbol::intern("x")),
            Instruction::Return,
        ]
    );
}

#[test]
fn fuse_dup_store_keeps_in_expression_position() {
    // Expression assignment (no trailing Pop): Dup; StoreField -> StoreFieldKeep.
    let code = vec![
        Instruction::Push(Constant::Int(1)),
        Instruction::Dup,
        Instruction::StoreField("y".into()),
        Instruction::Return,
    ];
    let (out, _) = fuse_bytecode(code, vec![None; 4]);
    assert_eq!(
        out,
        vec![
            Instruction::Push(Constant::Int(1)),
            Instruction::StoreFieldKeep("y".into()),
            Instruction::Return,
        ]
    );
}

#[test]
fn fuse_dup_store_pop_respects_jump_into_the_pop() {
    // A jump targets the Pop -> can't drop it; fall back to the keep variant and fix
    // the offset so the jump still lands on the standalone Pop.
    let code = vec![
        Instruction::Push(Constant::Bool(true)),      // 0
        Instruction::IfJump(4),                       // 1 -> target 5 (the Pop)
        Instruction::Push(Constant::Int(1)),          // 2
        Instruction::Dup,                             // 3
        Instruction::StoreLocal(Symbol::intern("x")), // 4
        Instruction::Pop,                             // 5  (jump target)
        Instruction::Return,                          // 6
    ];
    let (out, _) = fuse_bytecode(code, vec![None; 7]);
    assert_eq!(
        out,
        vec![
            Instruction::Push(Constant::Bool(true)),
            Instruction::IfJump(3),
            Instruction::Push(Constant::Int(1)),
            Instruction::StoreLocalKeep(Symbol::intern("x")),
            Instruction::Pop,
            Instruction::Return,
        ]
    );
    if let Instruction::IfJump(off) = out[1] {
        assert!(matches!(out[(1 + off) as usize], Instruction::Pop));
    }
}

#[test]
fn fuse_dup_store_not_fused_when_store_is_jump_target() {
    // A jump targets the store itself (skipping the Dup) -> no fusion at all.
    let code = vec![
        Instruction::Push(Constant::Bool(true)),      // 0
        Instruction::IfJump(3),                       // 1 -> target 4 (the store)
        Instruction::Push(Constant::Int(1)),          // 2
        Instruction::Dup,                             // 3
        Instruction::StoreLocal(Symbol::intern("x")), // 4  (jump target)
        Instruction::Return,                          // 5
    ];
    let (out, _) = fuse_bytecode(code.clone(), vec![None; 6]);
    assert_eq!(out, code);
}

#[test]
fn fuse_3instr_send_local_local() {
    let sel = Symbol::intern("foo:");
    let code = vec![
        Instruction::LoadLocal(Symbol::intern("a")),
        Instruction::LoadLocal(Symbol::intern("b")),
        Instruction::Send(sel, 1),
        Instruction::Return,
    ];
    let (out, _) = fuse_bytecode(code, vec![None; 4]);
    assert_eq!(
        out,
        vec![
            Instruction::SendLocalLocal(Symbol::intern("a"), Symbol::intern("b"), sel, 1),
            Instruction::Return,
        ]
    );
}

#[test]
fn fuse_3instr_send_local_const() {
    let sel = Symbol::intern("-:");
    let code = vec![
        Instruction::LoadLocal(Symbol::intern("n")),
        Instruction::Push(Constant::Int(1)),
        Instruction::Send(sel, 1),
        Instruction::Return,
    ];
    let (out, _) = fuse_bytecode(code, vec![None; 4]);
    assert_eq!(
        out,
        vec![
            Instruction::SendLocalConst(Symbol::intern("n"), Constant::Int(1), sel, 1),
            Instruction::Return,
        ]
    );
}

#[test]
fn fuse_3instr_absorbs_only_the_last_two_operands() {
    // A 2-arg send: the receiver load stays, the last two operand loads fuse.
    let sel = Symbol::intern("at:put:");
    let code = vec![
        Instruction::LoadLocal(Symbol::intern("list")),
        Instruction::LoadLocal(Symbol::intern("i")),
        Instruction::LoadLocal(Symbol::intern("v")),
        Instruction::Send(sel, 2),
        Instruction::Return,
    ];
    let (out, _) = fuse_bytecode(code, vec![None; 5]);
    assert_eq!(
        out,
        vec![
            Instruction::LoadLocal(Symbol::intern("list")),
            Instruction::SendLocalLocal(Symbol::intern("i"), Symbol::intern("v"), sel, 2),
            Instruction::Return,
        ]
    );
}

#[test]
fn fuse_3instr_fixes_jump_offset() {
    let sel = Symbol::intern("f");
    // Jump forward over a 3->1 collapse: offset shrinks by two.
    let code = vec![
        Instruction::Push(Constant::Bool(true)),     // 0
        Instruction::IfJump(5),                      // 1 -> target 6 (Return)
        Instruction::LoadLocal(Symbol::intern("a")), // 2 \
        Instruction::LoadLocal(Symbol::intern("b")), // 3  > fuse
        Instruction::Send(sel, 1),                   // 4 /
        Instruction::Pop,                            // 5
        Instruction::Return,                         // 6  (target)
    ];
    let (out, _) = fuse_bytecode(code, vec![None; 7]);
    assert_eq!(
        out,
        vec![
            Instruction::Push(Constant::Bool(true)),
            Instruction::IfJump(3),
            Instruction::SendLocalLocal(Symbol::intern("a"), Symbol::intern("b"), sel, 1),
            Instruction::Pop,
            Instruction::Return,
        ]
    );
    if let Instruction::IfJump(off) = out[1] {
        assert!(matches!(out[(1 + off) as usize], Instruction::Return));
    }
}
