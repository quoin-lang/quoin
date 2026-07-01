//! Tests for the safety-net checks themselves: they must actually detect the hazards the
//! formatter's self-verification relies on catching.

use super::*;

#[test]
fn ast_equal_detects_a_semicolon_that_rebinds_a_leading_dot() {
    // `foo -> { 1 }; .mix:Bar` is two statements; drop the `;` and `.mix:` binds to the block,
    // making it one. The self-check must see these as different so it never removes that `;`.
    let two = "A <- { foo -> { 1 }; .mix:Bar }";
    let one = "A <- { foo -> { 1 } .mix:Bar }";
    assert_eq!(ast_equal(two, one), Some(false));

    // The same program, only re-spaced, stays equal.
    assert_eq!(
        ast_equal(two, "A <- {\n    foo -> { 1 };\n    .mix:Bar\n}"),
        Some(true)
    );
}

#[test]
fn ast_equal_is_none_when_the_output_does_not_parse() {
    assert_eq!(ast_equal("x = 1", "x = ((("), None);
}

#[test]
fn comments_preserved_detects_a_dropped_comment() {
    assert!(!comments_preserved("x = 1  \"* note", "x = 1"));
    // Trailing whitespace on the comment doesn't count as a change.
    assert!(comments_preserved("x = 1  \"* note", "x = 1  \"* note   "));
}
