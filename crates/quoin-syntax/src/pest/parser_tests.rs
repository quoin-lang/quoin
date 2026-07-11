//! Collection-literal element splitting — see `parse_literal_elements`.
//!
//! Elements are juxtaposed expressions, so an operator whose lexeme is also a prefix
//! operator (`+`, `-`, `%`) is ambiguous: `#(1 -2)` could be `1 - 2` or two elements.
//! Spacing decides, as in Ruby: detached from the left operand and glued to the right
//! means prefix. These tests pin every combination of the three operators against the
//! four spacings, plus precedence, spans, and the containers the rule applies to.

use super::*;
use crate::ast::{BinaryOperatorType, UnaryOperatorType};

/// The elements of the collection literal that is the program's single expression.
fn elements(source: &str) -> Vec<Node> {
    let program = try_parse_quoin_string_named(source, "<test>").expect("parses");
    let NodeValue::Program(program) = program.value else {
        panic!("expected a program");
    };
    let expr = program.expressions.first().expect("one expression");
    match &expr.value {
        NodeValue::List(list) => list.values.iter().map(|v| (**v).clone()).collect(),
        NodeValue::Set(set) => set.values.iter().map(|v| (**v).clone()).collect(),
        NodeValue::UserList(list) => list.values.iter().map(|v| (**v).clone()).collect(),
        other => panic!("expected a collection literal, got {other:?}"),
    }
}

/// A literal integer, folding a prefix `-` / `+` so `-2` reads as `-2` rather than a tree.
fn int(node: &Node) -> i64 {
    match &node.value {
        NodeValue::Integer(n) => n.value,
        NodeValue::UnaryOperator(u) => match &u.operator {
            UnaryOperatorType::Sub => -int(&u.right),
            UnaryOperatorType::Add => int(&u.right),
            other => panic!("unexpected unary {other:?}"),
        },
        other => panic!("expected an integer, got {other:?}"),
    }
}

fn ints(source: &str) -> Vec<i64> {
    elements(source).iter().map(int).collect()
}

/// The infix operator of a single-element literal, i.e. the literal did NOT split.
fn single_infix(source: &str) -> BinaryOperatorType {
    let els = elements(source);
    assert_eq!(els.len(), 1, "expected one element in `{source}`");
    match &els[0].value {
        NodeValue::BinaryOperator(b) => b.operator.clone(),
        other => panic!("expected a binary operator, got {other:?}"),
    }
}

// --- the four spacings, for each prefix-capable operator ---------------------------
//
// `a OP b` and `aOPb` and `aOP b` are infix; only `a OPb` (detached left, glued right)
// is a prefix that starts a new element.

#[test]
fn minus_splits_only_when_detached_left_and_glued_right() {
    assert_eq!(ints("#(1 -2)"), vec![1, -2], "detached left, glued right");
    assert_eq!(
        single_infix("#(1-2)"),
        BinaryOperatorType::Sub,
        "glued both"
    );
    assert_eq!(single_infix("#(1 - 2)"), BinaryOperatorType::Sub, "spaced");
    assert_eq!(
        single_infix("#(1- 2)"),
        BinaryOperatorType::Sub,
        "glued left"
    );
}

#[test]
fn plus_splits_only_when_detached_left_and_glued_right() {
    assert_eq!(ints("#(1 +2)"), vec![1, 2]);
    assert_eq!(single_infix("#(1+2)"), BinaryOperatorType::Add);
    assert_eq!(single_infix("#(1 + 2)"), BinaryOperatorType::Add);
    assert_eq!(single_infix("#(1+ 2)"), BinaryOperatorType::Add);
}

#[test]
fn percent_splits_only_when_detached_left_and_glued_right() {
    // `%x` is the prefix (interpolation) form; `a % b` is format / modulo.
    assert_eq!(elements("#(10 %3)").len(), 2);
    assert_eq!(single_infix("#(10%3)"), BinaryOperatorType::Mod);
    assert_eq!(single_infix("#(10 % 3)"), BinaryOperatorType::Mod);
    assert_eq!(single_infix("#(10% 3)"), BinaryOperatorType::Mod);
}

#[test]
fn bang_is_never_infix_so_it_needs_no_rule() {
    // The infix form is the two-character `!=`, so a prefix `!` can't be confused with it
    // and splits without any help: greedy parsing simply cannot extend across it.
    assert_eq!(elements("#(true !true)").len(), 2);
    assert_eq!(single_infix("#(1 != 2)"), BinaryOperatorType::NotEq);
}

// --- the bug this fixes ------------------------------------------------------------

#[test]
fn a_list_of_negative_numbers_is_a_list_of_negative_numbers() {
    assert_eq!(ints("#(-1 -2)"), vec![-1, -2]);
    assert_eq!(ints("#(5 -10)"), vec![5, -10]);
    assert_eq!(ints("#(-3 -1 -2 0 5 -10)"), vec![-3, -1, -2, 0, 5, -10]);
}

#[test]
fn subtraction_between_computed_operands_is_preserved() {
    // The regression a grammar-only `-`-followed-by-digit rule introduced: these have no
    // whitespace before the operator, so they are subtraction, not two elements.
    assert_eq!(single_infix("#((a)-1)"), BinaryOperatorType::Sub);
    assert_eq!(single_infix("#(x.abs-1)"), BinaryOperatorType::Sub);
    assert_eq!(single_infix("#(5-3)"), BinaryOperatorType::Sub);
}

#[test]
fn identifiers_split_like_numbers() {
    let els = elements("#(a -b)");
    assert_eq!(els.len(), 2);
    assert!(matches!(els[0].value, NodeValue::Identifier(_)));
    let NodeValue::UnaryOperator(ref u) = els[1].value else {
        panic!("expected a prefix `-`, got {:?}", els[1].value);
    };
    assert_eq!(u.operator, UnaryOperatorType::Sub);
    assert_eq!(single_infix("#(a - b)"), BinaryOperatorType::Sub);
}

#[test]
fn interpolation_is_its_own_element() {
    let els = elements("#( 'a' %'b' )");
    assert_eq!(els.len(), 2);
    let NodeValue::UnaryOperator(ref u) = els[1].value else {
        panic!("expected a prefix `%`, got {:?}", els[1].value);
    };
    assert_eq!(u.operator, UnaryOperatorType::Mod);
    // Spaced, it is the format operator applied to the string.
    assert_eq!(single_infix("#( 'a' % 'b' )"), BinaryOperatorType::Mod);
}

// --- precedence must be DERIVED from the split token sequence -----------------------

#[test]
fn the_prefix_binds_tighter_than_the_infix_that_follows_it() {
    // `#(1 -2 + 3)` parses greedily as `(1 - 2) + 3`. The second element is `(-2) + 3`,
    // NOT `-(2 + 3)` — which is what re-associating the parsed tree would have produced.
    let els = elements("#(1 -2 + 3)");
    assert_eq!(els.len(), 2);
    let NodeValue::BinaryOperator(ref add) = els[1].value else {
        panic!("expected `(-2) + 3`, got {:?}", els[1].value);
    };
    assert_eq!(add.operator, BinaryOperatorType::Add);
    assert_eq!(
        int(&add.left),
        -2,
        "the prefix applies to `2`, not to `2 + 3`"
    );
    assert_eq!(int(&add.right), 3);
}

#[test]
fn a_higher_precedence_operator_after_the_boundary_still_binds_first() {
    // `%` binds tighter than `-`, so the greedy tree is `1 - (2 % 3)`. The element is
    // `(-2) % 3`: the prefix attaches to the leftmost operand, not to the `%` subtree.
    let els = elements("#(1 -2 % 3)");
    assert_eq!(els.len(), 2);
    let NodeValue::BinaryOperator(ref m) = els[1].value else {
        panic!("expected `(-2) % 3`, got {:?}", els[1].value);
    };
    assert_eq!(m.operator, BinaryOperatorType::Mod);
    assert_eq!(int(&m.left), -2);
}

// --- containers, nesting, spans -----------------------------------------------------

#[test]
fn the_rule_applies_to_every_juxtaposing_literal() {
    assert_eq!(ints("#(-1 -2)"), vec![-1, -2], "list");
    assert_eq!(ints("#<-1 -2>"), vec![-1, -2], "set");
    assert_eq!(ints("#Foo(-1 -2)"), vec![-1, -2], "user list");
    assert_eq!(elements("#<5-3>").len(), 1, "set keeps subtraction");
}

#[test]
fn nested_literals_split_independently() {
    let els = elements("#(#(-1 -2) #(5-3))");
    assert_eq!(els.len(), 2);
    let NodeValue::List(ref inner) = els[0].value else {
        panic!("expected a nested list");
    };
    assert_eq!(inner.values.len(), 2, "inner negatives split");
    let NodeValue::List(ref tight) = els[1].value else {
        panic!("expected a nested list");
    };
    assert_eq!(tight.values.len(), 1, "inner subtraction does not");
}

#[test]
fn split_elements_keep_their_original_source_positions() {
    // Elements are re-parsed out of a buffer left-padded with spaces; if the padding were
    // wrong, spans would drift and every runtime error inside a literal — and every `qn fmt`
    // span — would point at the wrong place.
    let els = elements("#(10\n  -2)");
    assert_eq!(els.len(), 2);

    let first = els[0].source_info.as_ref().expect("source info");
    assert_eq!((first.line, first.column), (1, 2), "`10` on line 1");

    let second = els[1].source_info.as_ref().expect("source info");
    assert_eq!(
        (second.line, second.column),
        (2, 2),
        "`-2` on line 2, at its `-`"
    );
    assert_eq!(second.source_text.as_deref(), Some("-2"));
}

// --- shebang -----------------------------------------------------------------

/// A leading `#!` line is grammar trivia: no AST node, and — because it PARSES
/// rather than being stripped — every later statement keeps its true position.
#[test]
fn a_shebang_line_is_trivia_with_positions_preserved() {
    let src = "#!/usr/bin/env qn\n'hi'.print\n";
    let program = try_parse_quoin_string_named(src, "<test>").expect("parses");
    let NodeValue::Program(program) = program.value else {
        panic!("expected a program");
    };
    assert_eq!(program.expressions.len(), 1);
    let si = program.expressions[0]
        .source_info
        .as_ref()
        .expect("statement has a span");
    assert_eq!(si.line, 2, "the statement after the shebang is on line 2");
}

/// Only the very start of the file is a shebang — `#!` later is the parse error
/// it always was (there is no `#!` construct in the language).
#[test]
fn a_shebang_anywhere_else_stays_an_error() {
    assert!(try_parse_quoin_string_named("'x'.print\n#! not a shebang\n", "<test>").is_err());
}

/// The shebang consumes exactly its own line: arbitrary content (flags, spaces,
/// even quotes) never leaks into the program.
#[test]
fn shebang_content_is_opaque() {
    let src = "#!/usr/bin/env -S qn \"weird ' content\n1 + 1\n";
    assert!(try_parse_quoin_string_named(src, "<test>").is_ok());
}
