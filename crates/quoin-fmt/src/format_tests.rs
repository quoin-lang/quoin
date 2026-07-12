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
    assert_eq!(fmt("x = 1; y = 2; z = 3"), "x = 1\ny = 2\nz = 3\n");
}

#[test]
fn splits_same_line_statements_onto_their_own_lines() {
    assert_eq!(fmt("a = 1 b = 2"), "a = 1\nb = 2\n");
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
    assert_eq!(fmt("x = 1  \"* note\ny = 2"), "x = 1  \"* note\ny = 2\n");
}

#[test]
fn keeps_one_blank_line_between_statements_that_had_one() {
    assert_eq!(fmt("x = 1\n\ny = 2"), "x = 1\n\ny = 2\n");
}

#[test]
fn collapses_multiple_blank_lines_to_one() {
    assert_eq!(fmt("x = 1\n\n\n\ny = 2"), "x = 1\n\ny = 2\n");
}

#[test]
fn preserves_a_multiline_statement_body_verbatim() {
    // Already-canonical input is a fixed point (class body lowered, indentation preserved).
    let src = "Point <- { |@x @y|\n    x -> { @x }\n};\ndone = 1";
    assert_eq!(
        fmt(src),
        "Point <- { |@x @y|\n    x -> { @x }\n}\ndone = 1\n"
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
        "C <- {\n    greet -> {\n        x = 1\n        x\n    }\n}\n"
    );
}

#[test]
fn single_keyword_send_indents_its_block_body() {
    let src = "list.each:{\n  a;\n  b\n}";
    assert_eq!(fmt(src), "list.each:{\n    a\n    b\n}\n");
}

#[test]
fn multi_keyword_send_breaks_with_continuation_at_statement_base() {
    // Multi-statement block args force the blocks (and so the send) to break; `else:` drops to the
    // statement's base column, block bodies nest +4, and closing braces return to the base.
    let src = "result.if:{\n    a;\n    b\n} else:{\n    c;\n    d\n}";
    let expected = "result.if:{\n    a\n    b\n}\nelse:{\n    c\n    d\n}\n";
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
        "C <- {\n    name -> { @name }\n    age -> { @age }\n}\n"
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
    assert_eq!(fmt("x.foo:1 bar:2\ny = 3"), "x.foo:1 bar:2\ny = 3\n");
}

#[test]
fn long_multi_keyword_send_receiver_breaks() {
    // Authored on one line but over 100 columns. No block args are force-broken, so it takes a
    // receiver break: the receiver drops to the opening line and continuation keyword names align
    // under the first (`.firstKeyword` at +4, its name and the others at +5).
    let src = "objectName.firstKeyword:someArgument secondKeyword:anotherArgument \
               thirdKeyword:yetMoreStuffHere fourth:more";
    let expected = "objectName\n    .firstKeyword:someArgument\n     secondKeyword:anotherArgument\n     thirdKeyword:yetMoreStuffHere\n     fourth:more\n";
    assert_eq!(fmt(src), expected);
}

#[test]
fn receiver_break_keeps_inline_blocks_inline() {
    // Over 100 columns, but the `if:`/`else:` args are inline value blocks. Breaking before the `.`
    // gives the long receiver its own line, which leaves the keyword lines short enough that the
    // blocks stay inline — no block is broken open.
    let src = "framing = (someCondition.checkThatIsFairlyLong:'chunked').if:{ 'chunked' } else:{ other.if:{ 'length' } else:{ 'close' } }";
    assert!(src.len() > 100);
    assert_eq!(
        fmt(src),
        "framing = (someCondition.checkThatIsFairlyLong:'chunked')\n    .if:{ 'chunked' }\n     else:{ other.if:{ 'length' } else:{ 'close' } }\n"
    );
}

#[test]
fn no_subject_send_aligns_continuation_under_the_first_keyword() {
    // A leading-`.` (no-subject) send that wraps keeps `.kw0` on the opening line at the base and
    // aligns the continuation keyword names one column past it (`.` at +4, names at +5).
    let src = "m -> {\n    .recordResult:{ (actual - expected).abs < tolerance } evidence:#( expected 'not within tolerance of' actual ) block:block\n}";
    assert_eq!(
        fmt(src),
        "m -> {\n    .recordResult:{ (actual - expected).abs < tolerance }\n     evidence:#( expected 'not within tolerance of' actual )\n     block:block\n}\n"
    );
}

#[test]
fn receiver_break_falls_back_to_base_column_when_a_block_must_break() {
    // Same wide send, but the `if:` body is a hand-broken multi-statement block that can't be
    // inlined — so the receiver stays with the first keyword and continuation keywords drop to the
    // statement base (isolating the receiver above breaking blocks would buy nothing).
    let src = "framing = (someCondition.checkThatIsFairlyLong:'chunked').if:{\n    a;\n    b\n} else:{ 'close' }";
    let out = fmt(src);
    assert!(
        out.starts_with("framing = (someCondition.checkThatIsFairlyLong:'chunked').if:{\n    a\n    b\n}\nelse:{ 'close' }"),
        "expected base-column fallback:\n{out}"
    );
}

#[test]
fn too_wide_block_takes_base_column_not_a_stranded_receiver() {
    // The `if:` block is too wide to fit inline even at the receiver-break column, so the send takes
    // base-column (receiver stays with `.if:`, `else:` at the base) rather than stranding `cond` on
    // its own line above a block that then breaks anyway.
    let src = "m -> {\n    result = cond.if:{ aBlockWhoseSingleStatementIsSoExtremelyWideThatEvenAfterAReceiverBreakItCannotPossiblyFitInlineXXXX } else:{ fallbackValue }\n}";
    assert_eq!(
        fmt(src),
        "m -> {\n    result = cond.if:{\n        aBlockWhoseSingleStatementIsSoExtremelyWideThatEvenAfterAReceiverBreakItCannotPossiblyFitInlineXXXX\n    }\n    else:{ fallbackValue }\n}\n"
    );
}

#[test]
fn receiver_break_layout_is_re_lowerable_not_bailed() {
    // A send already in receiver-break layout (receiver on its own line, `.` on the next) must
    // re-lower, not bail: `subject_text` is trimmed of the break's newline so it isn't mistaken for a
    // multi-line receiver. Here re-lowering finds the block fits at the receiver-break column, so it
    // stays a receiver break with the block inline (idempotent).
    let src = "m -> {\n    sock = (scheme == 'https')\n        .if:{\n             theBodyStatementIsLongEnoughToKeepTheWholeSendFromFittingOnOneLineXXXXXX\n         }\n         else:{ fallback }\n}";
    assert_eq!(
        fmt(src),
        "m -> {\n    sock = (scheme == 'https')\n        .if:{ theBodyStatementIsLongEnoughToKeepTheWholeSendFromFittingOnOneLineXXXXXX }\n         else:{ fallback }\n}\n"
    );
}

#[test]
fn blank_line_between_two_comment_paragraphs_is_preserved() {
    // Two distinct leading comment paragraphs separated by a blank line stay separated, not fused.
    assert_eq!(
        fmt("\"* comment A\n\n\"* comment B\nx = 1"),
        "\"* comment A\n\n\"* comment B\nx = 1\n"
    );
}

#[test]
fn lowers_assignment_with_a_block_rhs() {
    // The RHS is lowered (so a multi-line RHS no longer bails the enclosing block to verbatim).
    let src = "m -> {\n    x = t.time:{ work };\n    x\n}";
    assert_eq!(fmt(src), "m -> {\n    x = t.time:{ work }\n    x\n}\n");
}

#[test]
fn lowers_a_return_value() {
    assert_eq!(fmt("m -> { ^^ foo.bar:5 }"), "m -> { ^^ foo.bar:5 }\n");
}

#[test]
fn multi_line_operator_expression_lowers_instead_of_bailing() {
    // A binary-operator expression whose operand spans lines (`pre % #( … )`) is lowered by its
    // final operand — `pre %` sliced verbatim, the `#( … )` lowered (here it collapses inline). Was a
    // cascade-to-verbatim: no arm for `BinaryOperator` meant the enclosing block bailed.
    let src = "m -> {\n    r = pre % #(\n        aaaa\n        bbbb\n    )\n    z = 2\n}";
    assert_eq!(
        fmt(src),
        "m -> {\n    r = pre % #( aaaa bbbb )\n    z = 2\n}\n"
    );
}

#[test]
fn single_line_operator_expression_stays_verbatim() {
    // A single-line operator expression is left as its exact source slice — no reformatting churn.
    assert_eq!(fmt("x = a  %  b"), "x = a  %  b\n");
}

#[test]
fn multi_line_send_argument_wraps_instead_of_bailing() {
    // A keyword arg that is itself a multi-line send (`Foo.new:{ … }`) is lowered structurally.
    // Regression: this used to bail (a multi-line non-collection arg had no layout), which forced
    // the whole enclosing statement — and, cascading up, its class — to fall back to verbatim.
    let src = "m -> {\n    r = self.addResult:Foo.new:{\n        a = 1;\n        b = 2\n    };\n    z = 2\n}";
    assert_eq!(
        fmt(src),
        "m -> {\n    r = self.addResult:Foo.new:{\n        a = 1\n        b = 2\n    }\n    z = 2\n}\n"
    );
}

#[test]
fn postfix_bang_selector_is_not_doubled() {
    // `!` is a separate token folded into the selector name but not its span; the tail must not
    // re-append it.
    let src = "C <- {\n    .sealed!;\n    m -> { 1 }\n}";
    assert_eq!(fmt(src), "C <- {\n    .sealed!\n    m -> { 1 }\n}\n");
}

#[test]
fn preserves_a_named_block() {
    // The `#tag` block name sits between `{` and the pipe; it must be kept.
    let src = "s.collect:{ #tag |t|\n    t.print;\n    ^t\n}";
    assert_eq!(fmt(src), "s.collect:{ #tag |t|\n    t.print\n    ^t\n}\n");
}

#[test]
fn preserves_a_return_only_block_header() {
    // A no-arg method with just a declared return type — `|^Integer|` — is header-only
    // content that must be kept; if dropped it gets swept into the body and reparses
    // differently (regression: return_type was missing from the header's has-pipe test).
    assert_eq!(fmt("m -> { |^Integer| 5 }"), "m -> { |^Integer| 5 }\n");
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
        out.starts_with("x.foo:{\n    aaa = 111\n    bbb = 222"),
        "not expanded:\n{out}"
    );
}

#[test]
fn hand_broken_multi_statement_value_block_stays_broken() {
    // A short body the author split across lines is left broken (not collapsed to one line).
    assert_eq!(fmt("m -> {\n    a;\n    b\n}"), "m -> {\n    a\n    b\n}\n");
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

#[test]
fn namespaced_type_annotations_round_trip() {
    // Namespaced types in every annotation position survive formatting (block headers
    // slice the source verbatim; the invariant that matters is the fmt() round-trip:
    // same AST after the grammar learned `namespace? ~ ident` in type positions).
    let out = fmt("run = { |e:[Web]Halt ^[A/B]Gadget - g:[IO]File| e }");
    assert!(out.contains("e:[Web]Halt"), "{out}");
    assert!(out.contains("^[A/B]Gadget"), "{out}");
    assert!(out.contains("g:[IO]File"), "{out}");

    let out = fmt("var x: [IO]File = openIt.value");
    assert!(out.contains("var x: [IO]File"), "{out}");
}

#[test]
fn block_type_annotations_round_trip() {
    // `Block(args ^Ret)` types (GENERICS_ARCH §11) survive formatting in
    // params, returns, and var declarations — headers slice verbatim, and the
    // self-verify re-parse must accept the `^`-tail grammar.
    let out = fmt("run = { |b:Block(Integer Integer ^Integer) ^Block(^Boolean)| b }");
    assert!(out.contains("b:Block(Integer Integer ^Integer)"), "{out}");
    assert!(out.contains("^Block(^Boolean)"), "{out}");

    let out = fmt("var f: Block(List(Integer) ^Boolean) = { |xs| true }");
    assert!(
        out.contains("var f: Block(List(Integer) ^Boolean)"),
        "{out}"
    );

    let out = fmt("var t: Block() = { 1 }");
    assert!(out.contains("var t: Block()"), "{out}");
}

#[test]
fn semicolons_survive_exactly_the_gluing_boundaries() {
    // The minimal-`;` rule (needs_separator): a `;` survives a line break only
    // where the next statement would otherwise GLUE onto the previous one —
    // newlines are trivia to the parser. Each kept case here was probed
    // against the live grammar; each dropped case parses as two statements.

    // Operator-leading statements extend the previous expression…
    assert_eq!(fmt("a;\n%'v %{a}'"), "a;\n%'v %{a}'\n");
    assert_eq!(fmt("a;\n-3.print"), "a;\n-3.print\n");
    // …and `.`-leading self-sends chain onto it.
    assert_eq!(fmt("a;\n.print"), "a;\n.print\n");
    // A bare identifier followed by a declaration/assignment fuses into one
    // destructuring assignment ("undeclared local `var`").
    assert_eq!(fmt("grade;\nvar name = 7"), "grade;\nvar name = 7\n");
    assert_eq!(fmt("a;\nb = 9"), "a;\nb = 9\n");
    assert_eq!(fmt("a;\n@f = 2"), "a;\n@f = 2\n");
    // But a send before a declaration stays a statement of its own…
    assert_eq!(fmt("x.print;\nvar y = 1"), "x.print\nvar y = 1\n");
    // …as do ordinary sequences: sends, literals, parens, namespaces.
    assert_eq!(fmt("a;\nb.print"), "a\nb.print\n");
    assert_eq!(fmt("a;\n(3).print"), "a\n(3).print\n");
    assert_eq!(fmt("a;\n#( 1 2 ).print"), "a\n#( 1 2 ).print\n");

    // Class bodies: `.`-leading members (`.meta`, `.sealed!`) and `|@f|`
    // field declarations keep their `;` — a block would absorb them —
    // while plain member defs separate on the newline alone.
    assert_eq!(
        fmt("C <- { m -> { 1 }; .sealed! }"),
        "C <- {\n    m -> { 1 };\n    .sealed!\n}\n"
    );

    // Inline (soft) blocks keep the `;` — there the separator may render as
    // a space on one shared line, where it is always load-bearing.
    assert_eq!(fmt("x.foo:{ a; b }"), "x.foo:{ a; b }\n");
}

/// A shebang is grammar trivia the lowering never sees — the formatter re-emits
/// it verbatim as the first line (and stays idempotent).
#[test]
fn a_shebang_line_survives_formatting() {
    let out = fmt("#!/usr/bin/env qn\nvar x = 1\nx.print\n");
    assert!(
        out.starts_with("#!/usr/bin/env qn\n"),
        "shebang dropped:\n{out}"
    );
}

#[test]
fn keeps_wrapping_parens_around_a_relowered_multiline_list() {
    // The closing `)` of `throw:( … )` lives in the REGION, not the list's span; the
    // collection re-lowering used to drop it (the reformatted source didn't parse).
    let src = "Foo <- {\n    m: -> { |entry|\n        ParseError.throw:('zip: %1 method %2' % #(\n            entry.name\n            entry.methodCode\n        ))\n    }\n}\n";
    let out = fmt(src);
    assert!(
        out.contains("% #( entry.name entry.methodCode ))"),
        "outer paren lost:\n{out}"
    );
}

#[test]
fn keeps_wrapping_parens_around_a_parenthesized_multiline_list_rvalue() {
    // Both sides at once: the leading `(` is captured by statement_content_start, the
    // trailing `)` sits between the list's span end and the statement end.
    let src = "x = (#(\n    1\n    2\n))\n";
    assert_eq!(fmt(src), "x = (#( 1 2 ))\n");
}

#[test]
fn keeps_wrapping_parens_around_a_multiline_list_mid_send() {
    let src = "foo.bar:('%1-%2' % #(\n    a\n    b\n)) baz:x\n";
    let out = fmt(src);
    assert!(out.contains(")) baz:x"), "mid-send paren lost:\n{out}");
}
