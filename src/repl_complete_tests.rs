//! Tests for `repl_complete`. `complete_input` is pure, so most tests build a hand-rolled
//! `CompletionIndex` and assert the (start, candidates) result directly — no VM needed. One
//! test exercises `build_completion_index` end-to-end against a real VM snapshot.

use super::*;

/// A small fixed index: class `Map` (class-side `new`/`withCapacity:`, instance `at:`/
/// `at:put:`/`count`), class `Animal` (instance `sound`/`legs`), a local `spot: Animal`,
/// namespaces `IO`/`HTTP`, and bare words.
fn idx() -> CompletionIndex {
    let mut class_side = HashMap::new();
    class_side.insert(
        "Map".to_string(),
        vec!["new".to_string(), "withCapacity:".to_string()],
    );
    class_side.insert("Animal".to_string(), vec!["new".to_string()]);

    let mut instance_side = HashMap::new();
    instance_side.insert(
        "Map".to_string(),
        vec![
            "at:".to_string(),
            "at:put:".to_string(),
            "count".to_string(),
            "keys".to_string(),
        ],
    );
    instance_side.insert(
        "Animal".to_string(),
        vec!["legs".to_string(), "sound".to_string()],
    );
    // Classes behind syntactically-typed literals.
    instance_side.insert(
        "String".to_string(),
        vec![
            "split:".to_string(),
            "splitString:".to_string(),
            "upcase".to_string(),
        ],
    );
    instance_side.insert("Integer".to_string(), vec!["times:".to_string()]);
    instance_side.insert("Boolean".to_string(), vec!["not".to_string()]);
    instance_side.insert("Nil".to_string(), vec!["nil?".to_string()]);
    // Classes behind `#`-sigil / delimited literals.
    instance_side.insert(
        "List".to_string(),
        vec!["each:".to_string(), "map:".to_string()],
    );
    instance_side.insert("Set".to_string(), vec!["add:".to_string()]);
    instance_side.insert("Regex".to_string(), vec!["match:".to_string()]);
    instance_side.insert("Symbol".to_string(), vec!["asString".to_string()]);
    instance_side.insert("Block".to_string(), vec!["value".to_string()]);

    let mut local_class = HashMap::new();
    local_class.insert("spot".to_string(), "Animal".to_string());

    CompletionIndex {
        words: vec![
            "Animal".to_string(),
            "Map".to_string(),
            "self".to_string(),
            "spot".to_string(),
        ],
        namespaces: vec!["HTTP".to_string(), "IO".to_string()],
        namespaced: vec![
            "[HTTP]Parser".to_string(),
            "[IO]File".to_string(),
            "[IO]Folder".to_string(),
            "[IO]Handle".to_string(),
        ],
        class_side,
        instance_side,
        local_class,
    }
}

#[test]
fn bare_word_completes_globals_and_locals() {
    assert_eq!(
        complete_input("Ma", 2, &idx()),
        (0, vec!["Map".to_string()])
    );
    assert_eq!(
        complete_input("sp", 2, &idx()),
        (0, vec!["spot".to_string()])
    );
    // Empty fragment offers everything (start at the cursor).
    assert_eq!(complete_input("", 0, &idx()).0, 0);
    assert_eq!(complete_input("", 0, &idx()).1.len(), 4);
    // No match → empty list, start at the fragment.
    assert_eq!(complete_input("Zz", 2, &idx()), (0, Vec::<String>::new()));
}

#[test]
fn class_receiver_completes_class_side() {
    assert_eq!(
        complete_input("Map.w", 5, &idx()),
        (4, vec!["withCapacity:".to_string()])
    );
    // Empty fragment after the dot lists all class-side selectors, sorted.
    assert_eq!(
        complete_input("Map.", 4, &idx()),
        (4, vec!["new".to_string(), "withCapacity:".to_string()])
    );
    // Class receivers do NOT offer instance selectors.
    assert_eq!(
        complete_input("Map.at", 6, &idx()),
        (4, Vec::<String>::new())
    );
}

#[test]
fn local_receiver_completes_instance_side() {
    assert_eq!(
        complete_input("spot.so", 7, &idx()),
        (5, vec!["sound".to_string()])
    );
    assert_eq!(
        complete_input("spot.", 5, &idx()),
        (5, vec!["legs".to_string(), "sound".to_string()])
    );
}

#[test]
fn literal_receivers_complete_instance_side() {
    // A string literal receiver → String instance selectors (the reported `'abcd'.spl` case).
    assert_eq!(
        complete_input("'abcd'.sp", 9, &idx()),
        (7, vec!["split:".to_string(), "splitString:".to_string()])
    );
    // Empty string literal, and a `#tag'…'` user string, both resolve to String.
    assert_eq!(
        complete_input("''.up", 5, &idx()),
        (3, vec!["upcase".to_string()])
    );
    assert_eq!(
        complete_input("#t'abc'.up", 10, &idx()),
        (8, vec!["upcase".to_string()])
    );
    // Bare keyword / integer literals.
    assert_eq!(
        complete_input("123.t", 5, &idx()),
        (4, vec!["times:".to_string()])
    );
    assert_eq!(
        complete_input("true.n", 6, &idx()),
        (5, vec!["not".to_string()])
    );
    assert_eq!(
        complete_input("nil.n", 5, &idx()),
        (4, vec!["nil?".to_string()])
    );
}

#[test]
fn delimited_literal_receivers_complete_instance_side() {
    let i = idx();
    // `#`-sigil collections and regex — class is fully determined by the construction syntax,
    // no evaluation needed.
    assert_eq!(
        complete_input("#(1 2 3).ma", 11, &i),
        (9, vec!["map:".to_string()])
    );
    assert_eq!(
        complete_input("#{ 'a': 1 }.k", 13, &i),
        (12, vec!["keys".to_string()])
    );
    assert_eq!(
        complete_input("#<1 2>.ad", 9, &i),
        (7, vec!["add:".to_string()])
    );
    assert_eq!(
        complete_input("#/ab/.ma", 8, &i),
        (6, vec!["match:".to_string()])
    );
    // Symbols, both bareword and quoted.
    assert_eq!(
        complete_input("#sym.as", 7, &i),
        (5, vec!["asString".to_string()])
    );
    assert_eq!(
        complete_input("#'a b'.as", 9, &i),
        (7, vec!["asString".to_string()])
    );
    // A bare block literal is a Block value.
    assert_eq!(
        complete_input("{ 1 }.va", 8, &i),
        (6, vec!["value".to_string()])
    );
    // A plain `(…)` grouping's value type isn't syntactic → nothing.
    assert_eq!(
        complete_input("(1 + 2).x", 9, &i),
        (8, Vec::<String>::new())
    );
    // A `)` inside a string doesn't fool the bracket matcher (still a List).
    assert_eq!(
        complete_input("#('a)b').ma", 11, &i),
        (9, vec!["map:".to_string()])
    );
}

#[test]
fn namespace_position_completes_paths() {
    assert_eq!(complete_input("[I", 2, &idx()), (1, vec!["IO".to_string()]));
    // Empty fragment lists all namespaces.
    assert_eq!(
        complete_input("[", 1, &idx()),
        (1, vec!["HTTP".to_string(), "IO".to_string()])
    );
}

#[test]
fn namespaced_name_completes_after_close_bracket() {
    // After a closed `[ns]`, complete the fully-qualified name, anchored at the `[`.
    assert_eq!(
        complete_input("[IO]Fi", 6, &idx()),
        (0, vec!["[IO]File".to_string()])
    );
    assert_eq!(
        complete_input("[HTTP]Pa", 8, &idx()),
        (0, vec!["[HTTP]Parser".to_string()])
    );
    // Empty fragment right after `]` lists every name in that namespace.
    assert_eq!(
        complete_input("[IO]", 4, &idx()),
        (
            0,
            vec![
                "[IO]File".to_string(),
                "[IO]Folder".to_string(),
                "[IO]Handle".to_string()
            ]
        )
    );
    // No match → empty, still anchored at the `[`.
    assert_eq!(
        complete_input("[IO]Zz", 6, &idx()),
        (0, Vec::<String>::new())
    );
}

#[test]
fn unresolvable_positions_yield_nothing() {
    // Unknown receiver (fragment `b` starts at 4).
    assert_eq!(
        complete_input("foo.b", 5, &idx()),
        (4, Vec::<String>::new())
    );
    // `@ivar` receiver is complex — not resolved (fragment `s` starts at 3).
    assert_eq!(complete_input("@x.s", 4, &idx()), (3, Vec::<String>::new()));
    // Chained send `Map.x.` — the inner receiver's class is unknown.
    assert_eq!(
        complete_input("Map.x.s", 7, &idx()),
        (6, Vec::<String>::new())
    );
}

#[test]
fn range_rhs_is_a_bare_word_not_a_send() {
    // `1..` ends in `..` (a range op), so the RHS completes as a bare word, not a `.` send —
    // an empty fragment, so it offers the bare-word list rather than nothing.
    let (start, cands) = complete_input("1..", 3, &idx());
    assert_eq!(start, 3);
    assert_eq!(cands.len(), 4);
}

#[test]
fn build_index_snapshots_the_live_vm() {
    use crate::parser::NodeValue;
    use gc_arena::{Arena, Rootable};

    let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
        let mut vm = VmState::new(mc, crate::vm::VmOptions::default());
        crate::runner::register_builtins(mc, &mut vm);
        vm
    });
    arena.mutate_root(|mc, vm| {
        let src = "Animal <- { |@legs| sound -> { 'woof' } }; \
                   Animal <- Dog <- { fetch: -> { |x:String| x } }; \
                   spot = Dog.new;";
        let node = crate::parser::parse_quoin_string(src);
        let NodeValue::Program(p) = &node.value else {
            panic!("not a program");
        };
        let sb = crate::compiler::Compiler::new().compile_program(p).unwrap();
        let block = crate::runtime::runtime::build_block(mc, &sb);
        vm.execute_block(mc, block, Vec::new(), None).unwrap();

        let index = build_completion_index(vm);

        // Bare words include the user class and the `Animal.new` global is a class.
        assert!(index.words.contains(&"Dog".to_string()));
        assert!(index.words.contains(&"Animal".to_string()));
        // `spot` is a top-level `=` assignment → a global value, not a session local here
        // (session locals come from `vm.repl_env`, unused in this non-REPL harness).

        // `Dog`'s instance selectors include its own `fetch:` and inherited `sound`.
        let dog = &index.instance_side["Dog"];
        assert!(dog.contains(&"fetch:".to_string()), "{dog:?}");
        assert!(dog.contains(&"sound".to_string()), "{dog:?}");

        // Class-side has an entry for every class (here, at least `new` on Dog/Animal).
        assert!(index.class_side.contains_key("Dog"));

        // Completing `Dog.fe` against the live snapshot — wait, Dog is a class, so `.`
        // resolves to class-side; instance `fetch:` is reached via a value receiver.
        let (start, cands) = complete_input("Do", 2, &index);
        assert_eq!(start, 0);
        assert!(cands.contains(&"Dog".to_string()));
    });
}
