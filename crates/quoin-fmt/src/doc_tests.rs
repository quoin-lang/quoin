//! Unit tests for the doc engine. These directly exercise `Group`/`Align`/`Nest`/
//! `SoftLine`/`HardLine` — combinators Phase 0's formatter doesn't use yet — so the
//! layout engine is proven before later phases build width-driven wrapping on it.

use super::*;

fn doc_text(s: &str) -> Doc {
    Doc::text(s)
}

/// `a<sep>b<sep>c` with a group around it; flat uses `Line` as a space.
fn triple() -> Doc {
    Doc::group(Doc::concat(vec![
        doc_text("a"),
        Doc::Line,
        doc_text("b"),
        Doc::Line,
        doc_text("c"),
    ]))
}

#[test]
fn group_stays_flat_when_it_fits() {
    assert_eq!(render(&triple(), 80), "a b c");
}

#[test]
fn group_breaks_when_it_would_overflow() {
    // Width 3 can't hold "a b c" (5 cols), so every `Line` becomes a newline.
    assert_eq!(render(&triple(), 3), "a\nb\nc");
}

#[test]
fn nest_indents_broken_lines() {
    let d = Doc::concat(vec![
        doc_text("head"),
        Doc::nest(
            4,
            Doc::group(Doc::concat(vec![
                Doc::Line,
                doc_text("x"),
                Doc::Line,
                doc_text("y"),
            ])),
        ),
    ]);
    assert_eq!(render(&d, 3), "head\n    x\n    y");
}

#[test]
fn align_sets_indent_to_current_column() {
    // After "kw." (3 cols), align pins the broken lines under column 3.
    let d = Doc::concat(vec![
        doc_text("kw."),
        Doc::align(Doc::group(Doc::concat(vec![
            doc_text("if:"),
            Doc::HardLine,
            doc_text("else:"),
        ]))),
    ]);
    assert_eq!(render(&d, 100), "kw.if:\n   else:");
}

#[test]
fn hardline_forces_break_even_inside_a_fitting_group() {
    // The content would fit flat, but a HardLine forces the group to break.
    let d = Doc::group(Doc::concat(vec![
        doc_text("a"),
        Doc::HardLine,
        doc_text("b"),
    ]));
    assert_eq!(render(&d, 80), "a\nb");
}

#[test]
fn softline_is_nothing_when_flat_and_a_break_when_not() {
    let d = Doc::group(Doc::concat(vec![
        doc_text("a"),
        Doc::SoftLine,
        doc_text("b"),
    ]));
    assert_eq!(render(&d, 80), "ab");
    assert_eq!(render(&d, 1), "a\nb");
}

#[test]
fn verbatim_is_emitted_as_is_and_resets_the_column() {
    let d = Doc::concat(vec![
        Doc::verbatim("x = {\n    1\n}"),
        Doc::HardLine,
        doc_text("next"),
    ]);
    assert_eq!(render(&d, 80), "x = {\n    1\n}\nnext");
}

#[test]
fn newline_trims_trailing_spaces() {
    let d = Doc::concat(vec![doc_text("a "), Doc::HardLine, doc_text("b")]);
    assert_eq!(render(&d, 80), "a\nb");
}
