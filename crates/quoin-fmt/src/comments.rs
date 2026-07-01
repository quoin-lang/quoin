//! Comment recovery. Quoin's pest grammar treats `WHITESPACE`/`COMMENT` as silent
//! rules, so comments never reach the AST — a formatter must re-scan the raw source
//! and re-attach them by byte position. This scanner mirrors the state machine in
//! `quoin_syntax::complete` (Normal / Str / Regex / line- and block-comment), so a
//! `"` inside a `'…'` string or `#/…/` regex is never mistaken for a comment.
//!
//! Quoin has two comment forms, both double-quote delimited (single quotes are
//! strings): a line comment `"* … ` to end of line, and a block comment `" … "`.
//! The empty `""` is a (degenerate block) comment too.

/// Which of Quoin's two comment syntaxes a [`Comment`] is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CommentKind {
    /// `"* … ` up to (not including) the end of the line.
    Line,
    /// `" … "` — double-quote delimited, may span lines.
    Block,
}

/// A comment found in the source, as an exact byte range and its verbatim text
/// (delimiters included). `end` is exclusive.
#[derive(Clone, Debug)]
pub struct Comment {
    pub start: usize,
    pub end: usize,
    pub kind: CommentKind,
    pub text: String,
}

/// Scan `source` and return every comment in source order.
///
/// All comment/string/regex delimiters are ASCII, so this walks bytes directly; the
/// recorded ranges are valid UTF-8 boundaries because the delimiters are ASCII and any
/// multibyte content sits strictly between them.
pub fn scan_comments(source: &str) -> Vec<Comment> {
    #[derive(PartialEq)]
    enum St {
        Normal,
        Str,
        Regex,
        Line,
        Block,
    }

    let bytes = source.as_bytes();
    let n = bytes.len();
    let mut comments = Vec::new();
    let mut st = St::Normal;
    let mut start = 0usize;
    let mut i = 0usize;

    while i < n {
        let c = bytes[i];
        match st {
            St::Normal => match c {
                b'\'' => st = St::Str,
                b'"' => match bytes.get(i + 1) {
                    Some(b'*') => {
                        st = St::Line;
                        start = i;
                        i += 1; // consume the '*'
                    }
                    Some(b'"') => {
                        // Empty `""` comment.
                        comments.push(Comment {
                            start: i,
                            end: i + 2,
                            kind: CommentKind::Block,
                            text: source[i..i + 2].to_string(),
                        });
                        i += 1; // consume the second '"'
                    }
                    _ => {
                        st = St::Block;
                        start = i;
                    }
                },
                // `#/` opens a regex; a bare `/` is division. `#` is the only prefix
                // that opens a scan-relevant context here (others are `#(`, `#{`, etc.).
                b'/' if i > 0 && bytes[i - 1] == b'#' => st = St::Regex,
                _ => {}
            },
            St::Str => match c {
                b'\\' => i += 1, // skip the escaped char
                b'\'' => st = St::Normal,
                _ => {}
            },
            St::Regex => match c {
                b'\\' => i += 1,
                b'/' => st = St::Normal,
                _ => {}
            },
            St::Line => {
                if c == b'\n' {
                    comments.push(Comment {
                        start,
                        end: i,
                        kind: CommentKind::Line,
                        text: source[start..i].to_string(),
                    });
                    st = St::Normal;
                }
            }
            St::Block => match c {
                b'\\' => i += 1,
                b'"' => {
                    comments.push(Comment {
                        start,
                        end: i + 1,
                        kind: CommentKind::Block,
                        text: source[start..i + 1].to_string(),
                    });
                    st = St::Normal;
                }
                _ => {}
            },
        }
        i += 1;
    }

    // A line comment with no trailing newline runs to EOF.
    if st == St::Line {
        comments.push(Comment {
            start,
            end: n,
            kind: CommentKind::Line,
            text: source[start..n].to_string(),
        });
    }
    comments
}

#[cfg(test)]
#[path = "comments_tests.rs"]
mod comments_tests;
