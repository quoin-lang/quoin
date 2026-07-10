//! The §4 adjacency rules (docs/DOCS_ARCH.md), pinned: what attaches, what detaches, and
//! exactly how the marker is stripped.

use super::{doc_above, summary};

const SRC: &str = r#""* File commentary: not attached to anything -- the blank line below detaches it.

"* A 2-D point.
"* Immutable once made.
Point <- { |@x @y|
    x -> { @x };

    "* The point mirrored through the origin.
    negated -> { Point.new:{ x = 0 - @x; y = 0 - @y } };
    "*Tight against the marker, no space.
    "*   ...and an indented continuation line.
    y -> { @y }
}
"#;

#[test]
fn a_contiguous_block_attaches() {
    // `Point <- {` is line 5.
    assert_eq!(
        doc_above(SRC, 5).as_deref(),
        Some("A 2-D point.\nImmutable once made.")
    );
}

#[test]
fn a_blank_line_detaches() {
    // The file commentary on line 1 is separated from the class block by a blank line, so the
    // class doc must not swallow it -- and line 3 (the doc's own first line) has nothing above
    // but that blank line, so it has no doc of its own.
    let doc = doc_above(SRC, 5).unwrap();
    assert!(!doc.contains("File commentary"));
    assert_eq!(doc_above(SRC, 3), None);
}

#[test]
fn code_above_means_no_doc() {
    // `x -> { @x };` is line 6; above it sits the class-definition line.
    assert_eq!(doc_above(SRC, 6), None);
}

#[test]
fn a_method_mid_class_attaches() {
    // `negated ->` is line 9.
    assert_eq!(
        doc_above(SRC, 9).as_deref(),
        Some("The point mirrored through the origin.")
    );
}

#[test]
fn marker_stripping_takes_at_most_one_space() {
    // `y ->` is line 12, under two doc lines: one tight against the marker, one indented.
    // Only ONE space is stripped, so the continuation keeps its extra indentation (fenced
    // examples depend on that).
    assert_eq!(
        doc_above(SRC, 12).as_deref(),
        Some("Tight against the marker, no space.\n  ...and an indented continuation line.")
    );
}

#[test]
fn edges_answer_none() {
    assert_eq!(doc_above(SRC, 1), None, "top of file");
    assert_eq!(doc_above("", 1), None, "empty source");
    assert_eq!(doc_above(SRC, 10_000), None, "line past the end");
}

#[test]
fn summary_is_the_first_line() {
    assert_eq!(
        summary("A 2-D point.\nImmutable once made."),
        "A 2-D point."
    );
    assert_eq!(summary(""), "");
}

use super::method_doc_above;

/// `qn fmt` wraps a long definition after `->`, putting the block literal — whose location
/// introspection reports — a line below the selector. The doc block sits above the selector.
const WRAPPED: &str = r#"X <- {
    "* Docs for the long one.
    aVeryLong:selector:with:many:parts: ->
    { |a b c d e|
        a
    };

    plain -> { 1 }
}
"#;

#[test]
fn a_wrapped_header_anchors_on_its_selector_line() {
    // The block literal is line 4; the selector is line 3; the doc is line 2.
    assert_eq!(
        method_doc_above(WRAPPED, 4, "aVeryLong:selector:with:many:parts:").as_deref(),
        Some("Docs for the long one.")
    );
}

#[test]
fn an_unwrapped_header_behaves_exactly_like_doc_above() {
    // `plain -> { 1 }` is line 8, block on the same line; line 7 is blank -> no doc.
    assert_eq!(method_doc_above(WRAPPED, 8, "plain"), None);
    // And a documented single-line header still attaches (Point fixture from above).
    assert_eq!(
        method_doc_above(SRC, 9, "negated").as_deref(),
        Some("The point mirrored through the origin.")
    );
}

#[test]
fn the_header_scan_requires_the_arrow_signature() {
    // A line that merely *mentions* the selector (no `->`) is not a header: from the block
    // line of `plain`, scanning up must not treat `a` (line 5, body code) as an anchor for
    // selector "a" — the arrow requirement rejects it, and the blank line above `plain`
    // means no doc either way.
    assert_eq!(method_doc_above(WRAPPED, 8, "a"), None);
}
