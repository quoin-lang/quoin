//! Unit tests for the Phase 0 formatter: top-level layout, `;` policy, blank lines,
//! and comment attachment. Every case also asserts the two core invariants.

use super::*;
use crate::verify::{ast_equal, comments_preserved};

/// Format `src` and assert it round-trips: same AST, comments preserved, idempotent.
fn fmt(src: &str) -> String {
    let out = format_source(src, "<test>").expect("formats");
    assert_eq!(ast_equal(src, &out), Some(true), "AST changed:\n{out}");
    assert!(
        comments_preserved(src, &out),
        "comment dropped/added:\n{out}"
    );
    let twice = format_source(&out, "<test>").expect("formats again");
    assert_eq!(
        out, twice,
        "not idempotent:\n--- once ---\n{out}\n--- twice ---\n{twice}"
    );
    out
}

#[test]
fn separates_statements_with_semicolons_but_not_the_last() {
    assert_eq!(fmt("x = 1; y = 2; z = 3"), "x = 1;\ny = 2;\nz = 3\n");
}

#[test]
fn splits_same_line_statements_onto_their_own_lines() {
    assert_eq!(fmt("a = 1 b = 2"), "a = 1;\nb = 2\n");
}

#[test]
fn drops_a_redundant_trailing_semicolon() {
    assert_eq!(fmt("x = 1;"), "x = 1\n");
}

#[test]
fn preserves_a_leading_doc_comment_hugging_its_statement() {
    assert_eq!(fmt("\"* doc\nx = 1"), "\"* doc\nx = 1\n");
}

#[test]
fn keeps_a_trailing_comment_on_the_statement_line() {
    assert_eq!(fmt("x = 1  \"* note\ny = 2"), "x = 1;  \"* note\ny = 2\n");
}

#[test]
fn keeps_one_blank_line_between_statements_that_had_one() {
    assert_eq!(fmt("x = 1\n\ny = 2"), "x = 1;\n\ny = 2\n");
}

#[test]
fn collapses_multiple_blank_lines_to_one() {
    assert_eq!(fmt("x = 1\n\n\n\ny = 2"), "x = 1;\n\ny = 2\n");
}

#[test]
fn preserves_a_multiline_statement_body_verbatim() {
    // Already-canonical input is a fixed point (class body lowered, indentation preserved).
    let src = "Point <- { |@x @y|\n    x -> { @x }\n};\ndone = 1";
    assert_eq!(
        fmt(src),
        "Point <- { |@x @y|\n    x -> { @x }\n};\ndone = 1\n"
    );
}

#[test]
fn comment_inside_a_block_is_left_in_place_not_duplicated() {
    let src = "C <- {\n    \"* inner\n    m -> { 1 }\n};\nx = 2";
    let out = fmt(src);
    // Exactly one occurrence of the inner comment (it rode along in the verbatim body).
    assert_eq!(out.matches("\"* inner").count(), 1);
}

#[test]
fn trailing_comment_after_last_statement_drops_below() {
    assert_eq!(fmt("x = 1\n\"* tail"), "x = 1\n\"* tail\n");
}

#[test]
fn single_line_block_stays_inline() {
    assert_eq!(fmt("m -> { @x }"), "m -> { @x }\n");
}

#[test]
fn keyword_send_has_no_space_after_colon() {
    assert_eq!(fmt("x.foo: a bar: b"), "x.foo:a bar:b\n");
}

#[test]
fn keyword_arg_keeps_its_parentheses() {
    // The arg is sliced from raw source, so its parens survive (they aren't in the AST).
    assert_eq!(fmt("x.foo: (a + b)"), "x.foo:(a + b)\n");
}

#[test]
fn method_def_body_indents_canonically() {
    let src = "C <- {\n  greet -> {\n  x = 1;\n  x\n  }\n}";
    assert_eq!(
        fmt(src),
        "C <- {\n    greet -> {\n        x = 1;\n        x\n    }\n}\n"
    );
}

#[test]
fn single_keyword_send_indents_its_block_body() {
    let src = "list.each:{\n  a;\n  b\n}";
    assert_eq!(fmt(src), "list.each:{\n    a;\n    b\n}\n");
}

#[test]
fn multi_keyword_send_breaks_aligned_under_first_keyword() {
    let src = "result.if:{\n    doThing\n} else:{\n    fallback\n}";
    // `else:` aligns under `if:` (column 7, after `result.`); block bodies at +4.
    let expected =
        "result.if:{\n           doThing\n       }\n       else:{\n           fallback\n       }\n";
    assert_eq!(fmt(src), expected);
}
