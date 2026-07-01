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
fn multi_keyword_send_breaks_with_continuation_at_statement_base() {
    // Multi-statement block args force the blocks (and so the send) to break; `else:` drops to the
    // statement's base column, block bodies nest +4, and closing braces return to the base.
    let src = "result.if:{\n    a;\n    b\n} else:{\n    c;\n    d\n}";
    let expected = "result.if:{\n    a;\n    b\n}\nelse:{\n    c;\n    d\n}\n";
    assert_eq!(fmt(src), expected);
}

#[test]
fn single_statement_value_block_collapses_when_it_fits() {
    // A needlessly-broken short method body is pulled back onto one line.
    assert_eq!(fmt("name -> {\n    @name\n}"), "name -> { @name }\n");
    // Single-statement block args collapse too, so the whole send fits.
    assert_eq!(fmt("x.if:{ a } else:{ b }"), "x.if:{ a } else:{ b }\n");
}

#[test]
fn over_long_value_block_wraps() {
    // A single-statement body that can't fit the width breaks onto its own indented line. The body
    // is a single-keyword send (no continuation keyword to wrap), so the block is what breaks.
    let long = "m -> { obj.call:'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa' }";
    assert!(long.len() > 100);
    let out = fmt(long);
    assert!(
        out.starts_with("m -> {\n    obj.call:"),
        "expected a wrapped body:\n{out}"
    );
    assert!(out.ends_with("}\n"));
}

#[test]
fn declaration_block_always_breaks_one_member_per_line() {
    // A class body is a member declaration block: it stays expanded even written on one line.
    assert_eq!(
        fmt("C <- { name -> { @name }; age -> { @age } }"),
        "C <- {\n    name -> { @name };\n    age -> { @age }\n}\n"
    );
    // Even a single-method class body breaks (the method itself stays inline).
    assert_eq!(
        fmt("C <- { name -> { @name } }"),
        "C <- {\n    name -> { @name }\n}\n"
    );
}

#[test]
fn short_multi_keyword_send_flattens_when_it_fits() {
    // Authored across lines, but it fits the width budget — width-driven wrapping flattens it.
    assert_eq!(
        fmt("obj.foo:1\n    bar:2\n    baz:3"),
        "obj.foo:1 bar:2 baz:3\n"
    );
}

#[test]
fn short_send_stays_flat_before_a_following_statement() {
    // Regression: the statement-separator line break after the send must not force its group to
    // break — a hard break in the trailing context only ends the line, it doesn't block flattening.
    assert_eq!(fmt("x.foo:1 bar:2\ny = 3"), "x.foo:1 bar:2;\ny = 3\n");
}

#[test]
fn long_multi_keyword_send_breaks_to_fit_the_width() {
    // Authored on one line but over 100 columns — it breaks, each continuation keyword dropping to
    // the statement's base column.
    let src = "objectName.firstKeyword:someArgument secondKeyword:anotherArgument \
               thirdKeyword:yetMoreStuffHere fourth:more";
    let expected = "objectName.firstKeyword:someArgument\nsecondKeyword:anotherArgument\nthirdKeyword:yetMoreStuffHere\nfourth:more\n";
    assert_eq!(fmt(src), expected);
}

#[test]
fn lowers_assignment_with_a_block_rhs() {
    // The RHS is lowered (so a multi-line RHS no longer bails the enclosing block to verbatim).
    let src = "m -> {\n    x = t.time:{ work };\n    x\n}";
    assert_eq!(fmt(src), "m -> {\n    x = t.time:{ work };\n    x\n}\n");
}

#[test]
fn lowers_a_return_value() {
    assert_eq!(fmt("m -> { ^^ foo.bar:5 }"), "m -> { ^^ foo.bar:5 }\n");
}

#[test]
fn postfix_bang_selector_is_not_doubled() {
    // `!` is a separate token folded into the selector name but not its span; the tail must not
    // re-append it.
    let src = "C <- {\n    .sealed!;\n    m -> { 1 }\n}";
    assert_eq!(fmt(src), "C <- {\n    .sealed!;\n    m -> { 1 }\n}\n");
}

#[test]
fn preserves_a_named_block() {
    // The `#tag` block name sits between `{` and the pipe; it must be kept.
    let src = "s.collect:{ #tag |t|\n    t.print;\n    ^t\n}";
    assert_eq!(fmt(src), "s.collect:{ #tag |t|\n    t.print;\n    ^t\n}\n");
}

#[test]
fn paren_wrapped_return_value_keeps_its_parens() {
    // The leading `(` is captured in the subject; the trailing `)` must be re-attached.
    assert_eq!(
        fmt("m -> { ^(.modes.first) }"),
        "m -> { ^(.modes.first) }\n"
    );
}

#[test]
fn single_line_multi_statement_value_block_inlines_when_short() {
    assert_eq!(fmt("x.foo:{ a; b }"), "x.foo:{ a; b }\n");
}

#[test]
fn single_line_multi_statement_value_block_breaks_when_long() {
    // Written on one line but over 100 columns, so the block expands one statement per line.
    let long = "x.foo:{ aaa = 111; bbb = 222; ccc = 333; ddd = 444; eee = 555; fff = 666; ggg = 777; hhh = 888; iii = 999 }";
    assert!(long.len() > 100);
    let out = fmt(long);
    assert!(
        out.starts_with("x.foo:{\n    aaa = 111;\n    bbb = 222;"),
        "not expanded:\n{out}"
    );
}

#[test]
fn hand_broken_multi_statement_value_block_stays_broken() {
    // A short body the author split across lines is left broken (not collapsed to one line).
    assert_eq!(
        fmt("m -> {\n    a;\n    b\n}"),
        "m -> {\n    a;\n    b\n}\n"
    );
}

#[test]
fn list_literal_inlines_when_short() {
    assert_eq!(fmt("x = #( a b c )"), "x = #( a b c )\n");
}

#[test]
fn empty_list_literal_is_preserved() {
    assert_eq!(fmt("x = #()"), "x = #()\n");
}

#[test]
fn long_list_arg_wraps_one_element_per_line_keeping_parens() {
    let long = "check.valueWithArgs:#( 'timeout' round (timeoutRound.value:round) expectedResult moreStuff evenMore )";
    assert!(long.len() > 100);
    let out = fmt(long);
    assert!(
        out.starts_with("check.valueWithArgs:#(\n    'timeout'\n    round\n"),
        "not wrapped:\n{out}"
    );
    // A paren-wrapped element keeps its parentheses (they're not in the element's AST span).
    assert!(
        out.contains("\n    (timeoutRound.value:round)\n"),
        "parens lost:\n{out}"
    );
}

#[test]
fn set_and_map_literals_inline_when_short() {
    assert_eq!(fmt("x = #< a b c >"), "x = #< a b c >\n");
    assert_eq!(fmt("x = #{ a: 1 b: 2 }"), "x = #{ a: 1 b: 2 }\n");
}

#[test]
fn user_list_literal_is_width_driven() {
    assert_eq!(fmt("x = #Foo( a b c )"), "x = #Foo( a b c )\n");
}

#[test]
fn long_map_wraps_one_pair_per_line() {
    let long = "cfg.set:#{ alpha: firstValue beta: secondValue gamma: thirdValue delta: fourthValue omega: lastValueHere }";
    assert!(long.len() > 100);
    let out = fmt(long);
    assert!(
        out.starts_with("cfg.set:#{\n    alpha: firstValue\n    beta: secondValue\n"),
        "not wrapped:\n{out}"
    );
    assert!(out.ends_with("\n}\n"));
}

#[test]
fn long_set_wraps_one_element_per_line() {
    let long = "check.of:#< elementOne elementTwo elementThree elementFour elementFive elementSix elementSeven elementEight >";
    assert!(long.len() > 100);
    let out = fmt(long);
    assert!(
        out.starts_with("check.of:#<\n    elementOne\n    elementTwo\n"),
        "not wrapped:\n{out}"
    );
    assert!(out.ends_with("\n>\n"));
}
