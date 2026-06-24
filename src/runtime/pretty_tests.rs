//! Tests for `.pp` pretty-printing. The layout engine (`best`/`bracket`) is pure and tested
//! directly; the value walk is tested end-to-end by running `<expr>.pp` through a VM and
//! comparing the resulting string.

use super::{best, bracket, text};
use crate::parser::NodeValue;
use crate::value::{ObjectPayload, Value};
use crate::vm::{VmOptions, VmState};
use gc_arena::{Arena, Rootable};

/// Build a VM (native builtins, no qnlib), run `src`, and return its result — which the tests
/// arrange to be a `String` (the output of a `.pp` call).
fn pp(src: &str) -> String {
    let mut arena = Arena::<Rootable![VmState<'_>]>::new(|mc| {
        let mut vm = VmState::new(mc, VmOptions::default());
        crate::runner::register_builtins(mc, &mut vm);
        vm
    });
    arena.mutate_root(|mc, vm| {
        let node = crate::parser::parse_quoin_string(src);
        let NodeValue::Program(p) = &node.value else {
            panic!("not a program");
        };
        let sb = crate::compiler::Compiler::new().compile_program(p).unwrap();
        let block = crate::runtime::runtime::build_block(mc, &sb);
        let result = vm.execute_block(mc, block, Vec::new(), None).unwrap();
        match result {
            Value::Object(o) => match &o.borrow().payload {
                ObjectPayload::String(s) => s.to_string(),
                other => panic!("pp result is not a string: {other:?}"),
            },
            other => panic!("pp result is not a string: {other}"),
        }
    })
}

// ---- layout engine (pure) ----

#[test]
fn layout_flat_when_it_fits_broken_when_not() {
    let doc = || bracket("", "#(", ")", vec![text("1"), text("2"), text("3")]);
    assert_eq!(best(80, &doc()), "#(1 2 3)");
    // Width 5 can't hold "#(1 2 3)" (8 cols) → break one per indented line.
    assert_eq!(best(5, &doc()), "#(\n  1\n  2\n  3\n)");
    // Empty collection is always compact.
    assert_eq!(best(1, &bracket("", "#(", ")", vec![])), "#()");
}

#[test]
fn layout_nests_inner_groups_independently() {
    // Outer fits → all flat.
    let inner = || bracket("", "#(", ")", vec![text("1"), text("2")]);
    let outer = bracket("", "#(", ")", vec![text("0"), inner()]);
    assert_eq!(best(80, &outer), "#(0 #(1 2))");
    // Outer forced to break, but each inner that still fits stays flat on its own line.
    assert_eq!(best(8, &outer), "#(\n  0\n  #(1 2)\n)");
}

// ---- value walk (end-to-end through `.pp`) ----

#[test]
fn pp_scalars_and_strings() {
    assert_eq!(pp("5.pp"), "5");
    assert_eq!(pp("true.pp"), "true");
    assert_eq!(pp("nil.pp"), "nil");
    // A string is escaped and quoted, even at top level (unlike `.s`).
    assert_eq!(pp("'hi'.pp"), "'hi'");
    assert_eq!(pp("'it\\'s\\n'.pp"), "'it\\'s\\n'");
}

#[test]
fn pp_collections_quote_string_elements() {
    assert_eq!(pp("#(1 2 3).pp"), "#(1 2 3)");
    assert_eq!(pp("#('a' 'b').pp"), "#('a' 'b')");
    // Forced break via an explicit narrow width.
    assert_eq!(pp("#(1 2 3).pp: 5"), "#(\n  1\n  2\n  3\n)");
    assert_eq!(pp("#<1 2>.pp"), "#<1 2>");
}

#[test]
fn pp_map_keys_quoted_and_sorted() {
    // Keys sorted for determinism; entries render `'key': value`.
    assert_eq!(pp("#{ 'b': 2 'a': 1 }.pp"), "#{'a': 1 'b': 2}");
}

#[test]
fn pp_object_shows_instance_vars() {
    // ivars in declaration (slot) order, prefixed with `@`, default-nil.
    assert_eq!(
        pp("Animal <- { |@legs @sound| }; Animal.new.pp"),
        "Animal{@legs: nil @sound: nil}"
    );
}

#[test]
fn pp_elides_cycles() {
    // `n.next = n` — the cycle guard renders the revisited node as `Node{…}`.
    assert_eq!(
        pp("Node <- { |@next| setNext: -> { |x| @next = x } }; n = Node.new; n.setNext: n; n.pp"),
        "Node{@next: Node{…}}"
    );
}
