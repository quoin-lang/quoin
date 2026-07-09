//! Unit tests for the pure, GC-free half of `[OS]Path` — the lexical `normalize`.
//! The dispatched surface (`join:`, `dirname:`, …) is covered end-to-end in
//! `qnlib/tests/57-os-path.qn`, where the argument typing and nil returns are visible.

use super::normalize;

#[test]
fn normalize_collapses_current_dir_and_repeated_separators() {
    assert_eq!(normalize("a/./b"), "a/b");
    assert_eq!(normalize("a//b"), "a/b");
    assert_eq!(normalize("./a"), "a");
    assert_eq!(normalize("/a/./b//c"), "/a/b/c");
}

#[test]
fn normalize_resolves_parent_against_the_preceding_segment() {
    assert_eq!(normalize("a/b/.."), "a");
    assert_eq!(normalize("a/b/../c"), "a/c");
    assert_eq!(normalize("a/../b"), "b");
}

#[test]
fn normalize_keeps_a_leading_parent_in_a_relative_path() {
    // Nothing to resolve `..` against without touching the filesystem, so it stays.
    assert_eq!(normalize("../a"), "../a");
    assert_eq!(normalize("../../a"), "../../a");
    assert_eq!(normalize("a/../../b"), "../b");
}

#[test]
fn normalize_drops_a_parent_that_would_escape_the_root() {
    // `/..` is `/` on every POSIX system.
    assert_eq!(normalize("/.."), "/");
    assert_eq!(normalize("/../.."), "/");
    assert_eq!(normalize("/a/../.."), "/");
}

#[test]
fn normalize_of_nothing_is_the_current_directory() {
    assert_eq!(normalize(""), ".");
    assert_eq!(normalize("."), ".");
    assert_eq!(normalize("a/.."), ".");
    assert_eq!(normalize("./."), ".");
}

#[test]
fn normalize_preserves_absoluteness_and_does_not_add_a_trailing_slash() {
    assert_eq!(normalize("/"), "/");
    assert_eq!(normalize("/a/"), "/a");
    assert_eq!(normalize("a/"), "a");
}
