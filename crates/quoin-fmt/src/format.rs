//! Phase 0 formatter: lower the top level of a program to the [`Doc`] engine.
//!
//! P0 deliberately makes only *top-level* layout decisions — one statement per line,
//! an explicit `;` between statements (optional in the grammar, but emitting it removes
//! all boundary ambiguity) with none after the last, one blank line between definitions
//! when the source had one, and comments re-attached in the gaps. Each statement's own
//! body is emitted **verbatim** from its source slice, so nothing inside it can be
//! dropped or reordered. Later phases recurse into blocks and expressions, replacing the
//! verbatim slices with real lowerings; the verification harness guards every step.
//!
//! Verbatim slices are only ever emitted at column 0 (top level). We never re-indent a
//! verbatim block by shifting its lines, because a Quoin string literal may contain a
//! literal newline — line-shifting would corrupt it. Re-indentation therefore only ever
//! happens structurally, via the doc engine's `Nest`, once a node is truly lowered.

use crate::comments::Comment;
use crate::comments::scan_comments;
use crate::doc::{Doc, render};
use quoin_syntax::ast::{Node, NodeValue};
use quoin_syntax::{ParseError, try_parse_quoin_string_named};

/// Target maximum line width for the canonical style.
pub const DEFAULT_WIDTH: usize = 100;

/// Why formatting failed.
#[derive(Debug)]
pub enum FormatError {
    /// The input did not parse.
    Parse(ParseError),
}

impl std::fmt::Display for FormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FormatError::Parse(e) => write!(f, "parse error: {e}"),
        }
    }
}

impl std::error::Error for FormatError {}

/// Format `source`, using `filename` only for parse-error messages.
pub fn format_source(source: &str, filename: &str) -> Result<String, FormatError> {
    // The parser strips a leading BOM before computing byte offsets, so strip it here too —
    // otherwise every span is shifted by the BOM's 3 bytes and our slices desync.
    let source = source.strip_prefix('\u{FEFF}').unwrap_or(source);

    let program = try_parse_quoin_string_named(source, filename).map_err(FormatError::Parse)?;
    let comments = scan_comments(source);
    let doc = lower_program(&program, source, &comments);

    let mut out = render(&doc, DEFAULT_WIDTH);
    // Normalize the file's trailing whitespace to exactly one newline.
    out.truncate(out.trim_end().len());
    if !out.is_empty() {
        out.push('\n');
    }
    Ok(out)
}

fn lower_program(program: &Node, source: &str, comments: &[Comment]) -> Doc {
    let exprs = match &program.value {
        NodeValue::Program(p) => &p.expressions,
        // Not a program (shouldn't happen): emit the whole file verbatim rather than lose it.
        _ => return Doc::verbatim(source),
    };

    // A top-level statement's `source_info` start is reliable (the first token), but its end
    // runs on to the *next* statement's start — it swallows trailing whitespace, the `;`
    // separator, and trailing comments. So take the starts and re-derive each statement's
    // real content end by trimming that trailing trivia. If any statement lacks a span
    // (shouldn't happen at top level), fall back to emitting the file unchanged rather than
    // risk dropping code.
    let mut ast_starts = Vec::with_capacity(exprs.len());
    for e in exprs.iter() {
        match e.source_info.as_ref() {
            Some(si) => ast_starts.push(si.start),
            None => return Doc::verbatim(source),
        }
    }
    if ast_starts.is_empty() {
        return Doc::verbatim(source);
    }
    let n = ast_starts.len();

    // A parenthesized expression's node starts at the inner token, so `(expr).m` reports its
    // start *after* the `(`. Extend each start left over any leading `(` (and whitespace) so
    // the verbatim slice keeps them. The floor is the previous statement's start.
    let starts: Vec<usize> = (0..n)
        .map(|i| {
            let floor = if i > 0 { ast_starts[i - 1] } else { 0 };
            statement_content_start(source, ast_starts[i], floor)
        })
        .collect();
    let ends: Vec<usize> = (0..n)
        .map(|i| {
            let region_end = if i + 1 < n {
                starts[i + 1]
            } else {
                source.len()
            };
            statement_content_end(source, comments, starts[i], region_end)
        })
        .collect();

    let mut parts: Vec<Doc> = Vec::new();
    let mut prev_end: Option<usize> = None;

    for i in 0..n {
        let (start, end) = (starts[i], ends[i]);
        let text = &source[start..end];

        // Comments in the gap before this statement, split into those that trail the
        // previous statement (same line) and those that lead this one (own line).
        let (trailing_prev, leading_this, blank) = match prev_end {
            Some(pe) => {
                let gap: Vec<&Comment> = comments
                    .iter()
                    .filter(|c| c.start >= pe && c.end <= start)
                    .collect();
                let (tr, ld) = split_trailing_leading(source, pe, &gap);
                (tr, ld, has_blank_line(&source[pe..start]))
            }
            None => {
                let ld: Vec<&Comment> = comments.iter().filter(|c| c.end <= start).collect();
                (Vec::new(), ld, false)
            }
        };

        // Terminate the previous statement (its `;`, any same-line trailing comments, the
        // line break, and a blank line if the source had one).
        if prev_end.is_some() {
            parts.push(Doc::text(";"));
            for c in &trailing_prev {
                parts.push(Doc::text("  "));
                parts.push(Doc::verbatim(c.text.clone()));
            }
            parts.push(Doc::HardLine);
            if blank {
                parts.push(Doc::HardLine);
            }
        }

        // Leading comments (doc comments) sit on their own lines, hugging this statement.
        for c in &leading_this {
            parts.push(Doc::verbatim(c.text.clone()));
            parts.push(Doc::HardLine);
        }

        parts.push(Doc::verbatim(text.to_string()));
        prev_end = Some(end);
    }

    // Comments after the last statement: same-line ones trail it (no `;`), the rest drop
    // onto their own lines below.
    if let Some(pe) = prev_end {
        let tail: Vec<&Comment> = comments.iter().filter(|c| c.start >= pe).collect();
        let mut cur = pe;
        let mut broke = false;
        for c in &tail {
            let same_line = !broke && !source[cur..c.start].contains('\n');
            if same_line {
                parts.push(Doc::text("  "));
                parts.push(Doc::verbatim(c.text.clone()));
            } else {
                broke = true;
                parts.push(Doc::HardLine);
                parts.push(Doc::verbatim(c.text.clone()));
            }
            cur = c.end;
        }
    }

    Doc::Concat(parts)
}

/// Extend `ast_start` left over any leading `(` (and the whitespace around them), down to
/// `floor`, so a parenthesized statement keeps its opening parens in the verbatim slice.
fn statement_content_start(source: &str, ast_start: usize, floor: usize) -> usize {
    let bytes = source.as_bytes();
    let mut start = ast_start;
    loop {
        let mut j = start;
        while j > floor && bytes[j - 1].is_ascii_whitespace() {
            j -= 1;
        }
        if j > floor && bytes[j - 1] == b'(' {
            start = j - 1;
        } else {
            break;
        }
    }
    start
}

/// Walk back from `region_end` (the next statement's start, or EOF) to the end of this
/// statement's real code, skipping trailing whitespace, the `;` separator, and any trailing
/// comments. Comments *inside* the code aren't reached, so they stay in the verbatim slice.
fn statement_content_end(
    source: &str,
    comments: &[Comment],
    start: usize,
    region_end: usize,
) -> usize {
    let bytes = source.as_bytes();
    let mut end = region_end;
    loop {
        while end > start && bytes[end - 1].is_ascii_whitespace() {
            end -= 1;
        }
        if end <= start {
            break;
        }
        if bytes[end - 1] == b';' {
            end -= 1;
            continue;
        }
        if let Some(c) = comments.iter().find(|c| c.end == end && c.start >= start) {
            end = c.start;
            continue;
        }
        break;
    }
    end
}

/// Split the comments in a gap into those that trail the previous statement (on its line,
/// before any newline) and those that lead the next one (on their own line). Once a
/// newline is seen, everything after is "leading".
fn split_trailing_leading<'a>(
    source: &str,
    prev_end: usize,
    gap: &[&'a Comment],
) -> (Vec<&'a Comment>, Vec<&'a Comment>) {
    let mut trailing = Vec::new();
    let mut leading = Vec::new();
    let mut cur = prev_end;
    let mut broke = false;
    for c in gap {
        if !broke && !source[cur..c.start].contains('\n') {
            trailing.push(*c);
        } else {
            broke = true;
            leading.push(*c);
        }
        cur = c.end;
    }
    (trailing, leading)
}

/// Does `s` contain a blank line — a newline, then only spaces/tabs, then another newline?
fn has_blank_line(s: &str) -> bool {
    let mut seen_nl = false;
    for ch in s.chars() {
        match ch {
            '\n' => {
                if seen_nl {
                    return true;
                }
                seen_nl = true;
            }
            ' ' | '\t' | '\r' => {}
            _ => seen_nl = false,
        }
    }
    false
}

#[cfg(test)]
#[path = "format_tests.rs"]
mod format_tests;
