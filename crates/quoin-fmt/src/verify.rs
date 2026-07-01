//! Correctness guardrails a formatter must never violate, factored out so both the
//! unit tests and the corpus round-trip test share them:
//!
//! * **Semantics preserved** — `parse(src)` and `parse(format(src))` are the same AST.
//!   `Node`'s derived `PartialEq` includes `source_info`, so we strip it from both
//!   sides first (via the crate's own `clear_source_info` walker); `IdentifierNode`/
//!   `NamespaceNode` already exclude it from equality.
//! * **Comments preserved** — every comment in the input survives in the output.
//!   Compared on trailing-trimmed text, since the renderer trims trailing whitespace.

use crate::comments::scan_comments;
use quoin_syntax::try_parse_quoin_string_named;

/// Do `a` and `b` parse to the same AST (ignoring source positions)? Returns `None`
/// if either fails to parse (a caller that already parsed `a` should treat that as a
/// bug in the formatter's output rather than a property violation).
pub fn ast_equal(a: &str, b: &str) -> Option<bool> {
    let mut na = try_parse_quoin_string_named(a, "<a>").ok()?;
    let mut nb = try_parse_quoin_string_named(b, "<b>").ok()?;
    na.clear_source_info();
    nb.clear_source_info();
    Some(na == nb)
}

/// The multiset of comment texts (trailing-trimmed) in source order.
fn comment_bag(source: &str) -> Vec<String> {
    let mut bag: Vec<String> = scan_comments(source)
        .into_iter()
        .map(|c| c.text.trim_end().to_string())
        .collect();
    bag.sort();
    bag
}

/// Does `after` contain exactly the same comments as `before` (no additions, drops,
/// or edits beyond trailing whitespace)?
pub fn comments_preserved(before: &str, after: &str) -> bool {
    comment_bag(before) == comment_bag(after)
}
