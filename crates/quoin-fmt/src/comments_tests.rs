//! Unit tests for the comment scanner: both comment forms, and correct skipping of
//! string/regex contexts that contain a `"`.

use super::*;

fn texts(source: &str) -> Vec<String> {
    scan_comments(source).into_iter().map(|c| c.text).collect()
}

#[test]
fn finds_a_line_comment() {
    let c = scan_comments("x = 1  \"* a note\ny = 2");
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].kind, CommentKind::Line);
    assert_eq!(c[0].text, "\"* a note");
    // The range excludes the terminating newline.
    assert_eq!(
        &"x = 1  \"* a note\ny = 2"[c[0].start..c[0].end],
        "\"* a note"
    );
}

#[test]
fn line_comment_without_trailing_newline_runs_to_eof() {
    let c = scan_comments("x = 1 \"* trailing");
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].text, "\"* trailing");
}

#[test]
fn finds_a_block_comment() {
    let c = scan_comments("a \"a block\" b");
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].kind, CommentKind::Block);
    assert_eq!(c[0].text, "\"a block\"");
}

#[test]
fn empty_comment_is_recognized() {
    let c = scan_comments("a \"\" b");
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].text, "\"\"");
}

#[test]
fn double_quote_inside_a_string_is_not_a_comment() {
    // The `"` lives inside a single-quoted string, so it must not open a comment.
    assert!(
        texts("x = 'a \" b' \"* real")
            .iter()
            .eq(["\"* real".to_string()].iter())
    );
}

#[test]
fn double_quote_inside_a_regex_is_not_a_comment() {
    assert!(
        texts("r = #/a\"b/ \"* real")
            .iter()
            .eq(["\"* real".to_string()].iter())
    );
}

#[test]
fn escaped_quote_in_string_does_not_end_it_early() {
    // The `\'` keeps the string open, so the later `"*` is still inside the string.
    assert_eq!(texts("x = 'a\\' \"* not a comment'"), Vec::<String>::new());
}

#[test]
fn finds_multiple_comments_in_order() {
    let c = scan_comments("\"* one\nx = 1  \"* two\n\"three\"");
    assert_eq!(
        c.iter().map(|c| c.text.as_str()).collect::<Vec<_>>(),
        vec!["\"* one", "\"* two", "\"three\""]
    );
}
