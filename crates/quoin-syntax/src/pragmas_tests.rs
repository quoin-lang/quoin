use super::scan_allow_pragmas;

fn scan(src: &str) -> Vec<(usize, Vec<String>, bool)> {
    scan_allow_pragmas(src, "<test>")
        .into_iter()
        .map(|p| (p.line, p.kinds, p.trailing))
        .collect()
}

#[test]
fn a_trailing_pragma_is_found_on_its_line() {
    let got = scan("x = 1;\ny.foo; \"* allow: nil-receiver\nz = 2;\n");
    assert_eq!(got, vec![(2, vec!["nil-receiver".to_string()], true)]);
}

#[test]
fn several_kinds_split_on_commas_and_spaces() {
    let got = scan("y.foo; \"* allow: nil-receiver, caret-discard mnu\n");
    assert_eq!(
        got,
        vec![(
            1,
            vec![
                "nil-receiver".to_string(),
                "caret-discard".to_string(),
                "mnu".to_string()
            ],
            true
        )]
    );
}

#[test]
fn a_whole_line_pragma_is_recorded_as_non_trailing() {
    // The checker warns on these (a doc-block collision waiting to happen); the
    // scanner just records the fact.
    let got = scan("\"* allow: mnu\nx.foo;\n");
    assert_eq!(got, vec![(1, vec!["mnu".to_string()], false)]);
}

#[test]
fn ordinary_comments_and_docs_are_not_pragmas() {
    assert_eq!(scan("\"* just a note\nx = 1; \"* trailing note\n"), vec![]);
    // Prose mentioning allow: mid-comment is not a pragma either — only a comment
    // that *starts* with `allow:` counts.
    assert_eq!(scan("x = 1; \"* we allow: nothing here\n"), vec![]);
}

#[test]
fn string_regex_and_block_comment_content_never_match() {
    assert_eq!(scan("x = '\"* allow: mnu';\n"), vec![]);
    assert_eq!(scan("r = #/\"+/; y = 1;\n"), vec![]);
    // A block comment's interior is not code (it cannot contain `"*` — a `"` would
    // close it, exactly as in the pest grammar).
    assert_eq!(scan("\"see allow: docs\" x = 1;\n"), vec![]);
    // …and an escaped quote inside a string doesn't end it early.
    assert_eq!(scan("x = 'it\\'s \"* allow: mnu';\n"), vec![]);
}

#[test]
fn a_parenthesized_rationale_is_not_a_kind() {
    let got = scan("y.foo; \"* allow: nil-receiver (the nil case is under test)\n");
    assert_eq!(got, vec![(1, vec!["nil-receiver".to_string()], true)]);
}

#[test]
fn a_pragma_on_the_last_line_without_a_newline_still_closes() {
    let got = scan("y.foo; \"* allow: caret-discard");
    assert_eq!(got, vec![(1, vec!["caret-discard".to_string()], true)]);
}

#[test]
fn an_empty_kind_list_is_preserved_for_the_checker_to_flag() {
    let got = scan("y.foo; \"* allow:\n");
    assert_eq!(got, vec![(1, Vec::<String>::new(), true)]);
}

#[test]
fn spans_carry_the_comment_range() {
    let src = "y.foo; \"* allow: mnu\n";
    let p = &scan_allow_pragmas(src, "f.qn")[0];
    assert_eq!(p.span.filename, "f.qn");
    assert_eq!(p.span.column, 7);
    assert_eq!(&src[p.span.start..p.span.end], "\"* allow: mnu");
}
