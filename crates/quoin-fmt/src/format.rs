//! Formatter: lower the AST into the [`Doc`] engine.
//!
//! Statements are formatted structurally where we know how, and fall back to their exact
//! source slice otherwise — so anything not yet handled is preserved verbatim rather than
//! risked. The recursion only ever descends into constructs whose exact source boundaries we
//! can determine (statement sequences, definition/block bodies, and the block arguments of a
//! message send); everything else — subjects, non-block arguments — is sliced from raw source,
//! which keeps parentheses and operator groupings byte-exact and sidesteps having to re-insert
//! precedence parens (the AST drops them).
//!
//! Two invariants make this safe: formatting never changes the AST, and never drops a comment.
//! They're not just tested over the corpus — `format_source` re-parses its own output and returns
//! a [`FormatError::Verification`] instead of the string if either would be violated, so a caller
//! (e.g. `qn fmt --write`) can never write meaning-changing output. A verbatim slice is only ever
//! emitted where its first line sits at the target column (top level, or a single-line slice),
//! never re-indented by shifting text — a Quoin string may contain a literal newline. Re-indentation
//! happens only structurally, through the doc engine's `Nest`/`Align`.

use crate::comments::Comment;
use crate::comments::scan_comments;
use crate::doc::{Doc, render};
use crate::verify;
use quoin_syntax::ast::{BlockNode, Node, NodeValue};
use quoin_syntax::{ParseError, try_parse_quoin_string_named};
use std::sync::Arc;

/// Target maximum line width for the canonical style.
pub const DEFAULT_WIDTH: usize = 100;
/// Body indentation, in columns.
const INDENT: isize = 4;

/// Why formatting failed.
#[derive(Debug)]
pub enum FormatError {
    /// The input did not parse.
    Parse(ParseError),
    /// The formatted output would change the program's meaning or drop a comment — a formatter
    /// bug caught by the self-check. The output is withheld so it can never be written.
    Verification(String),
}

impl std::fmt::Display for FormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FormatError::Parse(e) => write!(f, "parse error: {e}"),
            FormatError::Verification(detail) => {
                write!(
                    f,
                    "internal formatter error: {detail} — formatting aborted, please report this"
                )
            }
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

    // Self-verification: the reformatted source must parse to the identical AST and preserve every
    // comment. If it wouldn't (e.g. a dropped `;` that rebinds a `.`-leading statement, or any
    // other bug), withhold the output entirely — a caller must never write meaning-changing output.
    match verify::ast_equal(source, &out) {
        Some(true) => {}
        Some(false) => {
            return Err(FormatError::Verification(
                "the reformatted source would parse differently".into(),
            ));
        }
        None => {
            return Err(FormatError::Verification(
                "the reformatted source does not parse".into(),
            ));
        }
    }
    if !verify::comments_preserved(source, &out) {
        return Err(FormatError::Verification(
            "a comment would be dropped or altered".into(),
        ));
    }
    Ok(out)
}

fn lower_program(program: &Node, source: &str, comments: &[Comment]) -> Doc {
    let exprs = match &program.value {
        NodeValue::Program(p) => &p.expressions,
        _ => return Doc::verbatim(source),
    };
    // The top level always succeeds: multi-line statements are safe to emit verbatim at column 0.
    match emit_sequence(source, comments, 0, source.len(), exprs, true) {
        Some(parts) => Doc::Concat(parts),
        None => Doc::verbatim(source),
    }
}

/// Lower a sequence of statements (a program, or a block body) over `[region_start, region_end)`.
///
/// Reconstructs each statement's real content span (see `statement_content_start`/`_end`),
/// re-attaches comments found in the gaps, and joins statements with an explicit `;` and a line
/// break. Returns the `Doc` parts, or `None` if a statement can't be laid out at this indent and
/// `allow_multiline_verbatim` is false (which bubbles up so the caller falls back to verbatim).
fn emit_sequence(
    source: &str,
    comments: &[Comment],
    region_start: usize,
    region_end: usize,
    stmts: &[Arc<Node>],
    allow_multiline_verbatim: bool,
) -> Option<Vec<Doc>> {
    let mut ast_starts = Vec::with_capacity(stmts.len());
    for e in stmts.iter() {
        ast_starts.push(e.source_info.as_ref()?.start);
    }
    let n = ast_starts.len();
    if n == 0 {
        return Some(Vec::new());
    }
    let starts: Vec<usize> = (0..n)
        .map(|i| {
            let floor = if i > 0 {
                ast_starts[i - 1]
            } else {
                region_start
            };
            statement_content_start(source, ast_starts[i], floor)
        })
        .collect();
    let ends: Vec<usize> = (0..n)
        .map(|i| {
            let re = if i + 1 < n { starts[i + 1] } else { region_end };
            statement_content_end(source, comments, starts[i], re)
        })
        .collect();

    let mut parts: Vec<Doc> = Vec::new();
    let mut prev_end: Option<usize> = None;

    for i in 0..n {
        let (start, end) = (starts[i], ends[i]);

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
                let ld: Vec<&Comment> = comments
                    .iter()
                    .filter(|c| c.start >= region_start && c.end <= start)
                    .collect();
                (Vec::new(), ld, false)
            }
        };

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

        for c in &leading_this {
            parts.push(Doc::verbatim(c.text.clone()));
            parts.push(Doc::HardLine);
        }

        let doc = match lower_stmt(&stmts[i], start, end, source, comments) {
            Some(d) => d,
            None => {
                let slice = &source[start..end];
                if slice.contains('\n') && !allow_multiline_verbatim {
                    return None;
                }
                Doc::verbatim(slice.to_string())
            }
        };
        parts.push(doc);
        prev_end = Some(end);
    }

    // Comments after the last statement in the region.
    if let Some(pe) = prev_end {
        let tail: Vec<&Comment> = comments
            .iter()
            .filter(|c| c.start >= pe && c.end <= region_end)
            .collect();
        let mut cur = pe;
        let mut broke = false;
        for c in &tail {
            if !broke && !source[cur..c.start].contains('\n') {
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

    Some(parts)
}

/// Structurally lower one statement, or `None` to let the caller emit it verbatim.
fn lower_stmt(
    stmt: &Node,
    content_start: usize,
    content_end: usize,
    source: &str,
    comments: &[Comment],
) -> Option<Doc> {
    match &stmt.value {
        NodeValue::ClassDefinition(c) => lower_def(content_start, &c.block, source, comments),
        NodeValue::ClassExtension(c) => lower_def(content_start, &c.block, source, comments),
        NodeValue::MethodDefinition(m) => lower_def(content_start, &m.block, source, comments),
        NodeValue::MethodExtension(m) => lower_def(content_start, &m.block, source, comments),
        NodeValue::Block(b) => lower_block(b, source, comments),
        NodeValue::MethodCall(_) => lower_send(stmt, content_start, content_end, source, comments),
        // `<lvalues> = <rvalue>` and `^^`/`^`/`^>` returns: the prefix through the assignment/return
        // operator is sliced verbatim; only the right-hand expression is lowered. This is what lets a
        // multi-line RHS (`x = Timer.time:{ … }`, `^^ foo.bar:{ … }`) format instead of forcing the
        // whole enclosing block to fall back to verbatim.
        NodeValue::Assignment(a) => {
            lower_prefixed(content_start, &a.rvalue, content_end, source, comments)
        }
        NodeValue::MethodReturn(r) => {
            lower_prefixed(content_start, &r.value, content_end, source, comments)
        }
        NodeValue::BlockReturn(r) => {
            lower_prefixed(content_start, &r.value, content_end, source, comments)
        }
        NodeValue::YieldReturn(r) => {
            lower_prefixed(content_start, &r.value, content_end, source, comments)
        }
        _ => None,
    }
}

/// Lower a statement of the form `<prefix> <expr>` — an assignment (`lvalues =`) or a return
/// (`^^` / `^` / `^>`). The prefix (everything up to the right-hand expression) is emitted verbatim;
/// the expression is lowered, so its own multi-line layout works. `None` (→ caller emits verbatim)
/// if the prefix spans lines or the expression is an un-lowerable multi-line construct.
fn lower_prefixed(
    content_start: usize,
    expr: &Node,
    content_end: usize,
    source: &str,
    comments: &[Comment],
) -> Option<Doc> {
    let estart = statement_content_start(source, expr.source_info.as_ref()?.start, content_start);
    let prefix = &source[content_start..estart];
    if prefix.contains('\n') {
        return None;
    }
    let expr_doc = match lower_stmt(expr, estart, content_end, source, comments) {
        Some(d) => d,
        None => {
            let slice = &source[estart..content_end];
            if slice.contains('\n') {
                return None;
            }
            Doc::verbatim(slice.to_string())
        }
    };
    Some(Doc::concat(vec![
        Doc::verbatim(prefix.to_string()),
        expr_doc,
    ]))
}

/// `<header> <block>` — a class/method definition or reopening. The header (`Point <- `,
/// `dist: -> `, `Point <-- `) is sliced verbatim from the statement start to the block's `{`.
fn lower_def(
    header_start: usize,
    block: &BlockNode,
    source: &str,
    comments: &[Comment],
) -> Option<Doc> {
    let bstart = block.source_info.as_ref()?.start;
    let header = &source[header_start..bstart];
    if header.contains('\n') {
        return None;
    }
    let block_doc = lower_block(block, source, comments)?;
    Some(Doc::concat(vec![
        Doc::verbatim(header.trim_end().to_string()),
        Doc::text(" "),
        block_doc,
    ]))
}

/// Lower a `{ … }` block.
///
/// A single, comment-free *value* statement (anything but a member declaration) is laid out
/// width-driven — `{ stmt }` when it fits, broken otherwise — so a short method body like
/// `name -> { @name }` stays on one line while an over-long one wraps. A block that *declares*
/// members (a class or meta body, whose statement is a method/class definition) always breaks,
/// one member per line, as do multi-statement blocks and any block whose body carries a comment.
/// An empty block, and a single-line block we don't lay out ourselves, are kept verbatim.
fn lower_block(block: &BlockNode, source: &str, comments: &[Comment]) -> Option<Doc> {
    let si = block.source_info.as_ref()?;
    let (bstart, bend) = (si.start, si.end);
    if block.statements.is_empty() {
        return Some(Doc::verbatim(source[bstart..bend].to_string()));
    }

    let region_start = bstart + 1;
    let region_end = bend - 1; // before the closing `}`
    let (header, body_start) = block_header(block, source, region_start, region_end);
    if header.contains('\n') {
        return None;
    }
    let has_comment = comments
        .iter()
        .any(|c| c.start >= region_start && c.end <= region_end);
    // A block that declares members (methods, nested classes) always breaks, one per line, so class
    // and meta bodies stay expanded however they were written.
    let declares_member = block.statements.iter().any(|s| {
        matches!(
            &s.value,
            NodeValue::ClassDefinition(_)
                | NodeValue::ClassExtension(_)
                | NodeValue::MethodDefinition(_)
                | NodeValue::MethodExtension(_)
        )
    });

    // A single comment-free value statement is laid out width-driven: `{ stmt }` if it fits.
    if !declares_member && block.statements.len() == 1 && !has_comment {
        let stmt = &block.statements[0];
        let sstart = stmt.source_info.as_ref()?.start;
        let cstart = statement_content_start(source, sstart, body_start);
        let cend = statement_content_end(source, comments, cstart, region_end);
        let lowered = lower_stmt(stmt, cstart, cend, source, comments);
        // Inline only when the statement can render on one line: a proper `Doc` (whose own hard
        // breaks the group still honors), or a single-line verbatim slice. A multi-line verbatim
        // slice can't sit in a flat group, so fall through to the always-break path.
        let inlineable = match &lowered {
            Some(_) => true,
            None => !source[cstart..cend].contains('\n'),
        };
        if inlineable {
            let stmt_doc =
                lowered.unwrap_or_else(|| Doc::verbatim(source[cstart..cend].to_string()));
            let mut open = vec![Doc::text("{")];
            if !header.is_empty() {
                open.push(Doc::text(" "));
                open.push(Doc::verbatim(header.to_string()));
            }
            return Some(Doc::group(Doc::concat(vec![
                Doc::concat(open),
                Doc::nest(INDENT, Doc::concat(vec![Doc::Line, stmt_doc])),
                Doc::Line,
                Doc::text("}"),
            ])));
        }
    }

    // Otherwise break — a declaration block, or a multi-statement / commented / un-inlineable body —
    // except a plain (non-declaration) single-line block, which we keep verbatim.
    let slice = &source[bstart..bend];
    if !declares_member && !slice.contains('\n') {
        return Some(Doc::verbatim(slice.to_string()));
    }
    let body = emit_sequence(
        source,
        comments,
        body_start,
        region_end,
        &block.statements,
        false,
    )?;
    let mut head = vec![Doc::text("{")];
    if !header.is_empty() {
        head.push(Doc::text(" "));
        head.push(Doc::verbatim(header.to_string()));
    }
    let mut inner = vec![Doc::HardLine];
    inner.extend(body);
    Some(Doc::concat(vec![
        Doc::concat(head),
        Doc::nest(INDENT, Doc::concat(inner)),
        Doc::HardLine,
        Doc::text("}"),
    ]))
}

/// The block's leading declarations — a name (`#foo`) and/or an argument pipe (`|a b - decls|`) —
/// and the offset where its statements begin. Empty header when the block has neither.
fn block_header<'a>(
    block: &BlockNode,
    source: &'a str,
    region_start: usize,
    region_end: usize,
) -> (&'a str, usize) {
    let has_pipe =
        !block.arguments.is_empty() || !block.decls.is_empty() || block.decl_block.is_some();
    let has_name = block.name.is_some();
    if !has_pipe && !has_name {
        return ("", region_start);
    }
    // The name and the pipe both sit between `{` and the first statement, name first. Capture from
    // `region_start` (not the `|`) so a leading `#name` is kept, and end after the closing pipe.
    if let Some(open) = source[region_start..region_end]
        .find('|')
        .map(|i| region_start + i)
        && let Some(close) = source[open + 1..region_end].find('|').map(|i| open + 1 + i)
    {
        return (source[region_start..close + 1].trim(), close + 1);
    }
    // A named block with no pipe: the header is just the `#name` symbol.
    let bytes = source.as_bytes();
    let mut end = region_start;
    while end < region_end && bytes[end].is_ascii_whitespace() {
        end += 1;
    }
    if bytes.get(end) == Some(&b'#') {
        end += 1;
        while end < region_end && !bytes[end].is_ascii_whitespace() && bytes[end] != b'|' {
            end += 1;
        }
        return (source[region_start..end].trim(), end);
    }
    ("", region_start)
}

/// Lower a message send. The subject and non-block arguments are sliced verbatim from raw
/// source (preserving parentheses); block arguments recurse. A keyword send that spans lines
/// breaks with continuation keywords aligned under the first (via `Align`).
fn lower_send(
    node: &Node,
    content_start: usize,
    content_end: usize,
    source: &str,
    comments: &[Comment],
) -> Option<Doc> {
    let NodeValue::MethodCall(mc) = &node.value else {
        return None;
    };
    let sels = &mc.arguments.signature.identifiers;
    let args = &mc.arguments.expressions;
    if sels.is_empty() {
        return None;
    }

    // Selector name spans (name only, the `:` sits at `end`).
    let mut sel_spans = Vec::with_capacity(sels.len());
    for id in sels.iter() {
        let s = id.source_info.as_ref()?;
        sel_spans.push((s.start, s.end, id.name.as_str()));
    }
    let dot_pos = sel_spans[0].0.checked_sub(1)?;
    let subject_text = &source[content_start..dot_pos];
    if subject_text.contains('\n') {
        return None;
    }

    // Bail if a comment lives in a structural position (anywhere in the send that isn't inside a
    // block argument) — reconstructing the send would drop it. Comments inside block args and
    // inside verbatim arg slices are preserved.
    let block_spans: Vec<(usize, usize)> = args
        .iter()
        .filter_map(|a| match &a.value {
            NodeValue::Block(b) => b.source_info.as_ref().map(|s| (s.start, s.end)),
            _ => None,
        })
        .collect();
    let bad_comment = comments.iter().any(|c| {
        c.start >= content_start
            && c.end <= content_end
            && !block_spans
                .iter()
                .any(|(s, e)| c.start >= *s && c.end <= *e)
    });
    if bad_comment {
        return None;
    }

    let subject = if subject_text.is_empty() {
        Doc::Nil
    } else {
        Doc::verbatim(subject_text.to_string())
    };

    // Trailing content between the send's last element and `content_end` — closing parens that wrap
    // the whole send (the leading `(` was captured in `content_start`). A paren-wrapped keyword arg
    // keeps its `)` inside the last verbatim arg slice, but a unary send or a block-terminated send
    // would otherwise drop it, so re-attach it here.
    let last_end = if args.is_empty() {
        // A postfix marker (`!` / `?`) is part of the selector *name* but not its span, so measure
        // from the name's rendered length to avoid re-appending the marker as a "trailing" char.
        let last = &sel_spans[sel_spans.len() - 1];
        last.0 + last.2.len()
    } else {
        match &args[args.len() - 1].value {
            NodeValue::Block(b) => b.source_info.as_ref().map(|s| s.end).unwrap_or(content_end),
            _ => content_end,
        }
    };
    let tail = source[last_end..content_end].trim();
    let tail_doc = if tail.is_empty() {
        Doc::Nil
    } else {
        Doc::verbatim(tail.to_string())
    };

    // Unary send: `subject.name`.
    if args.is_empty() {
        return Some(Doc::concat(vec![
            subject,
            Doc::text("."),
            Doc::text(sel_spans[0].2.to_string()),
            tail_doc,
        ]));
    }
    if sels.len() != args.len() {
        return None;
    }

    // Build each `name:arg` pair. Block args recurse; others are verbatim source between the
    // colon and the next selector (or the send end), preserving their parentheses.
    let mut pairs = Vec::with_capacity(sels.len());
    for i in 0..sels.len() {
        let name = sel_spans[i].2;
        let arg_doc = match &args[i].value {
            NodeValue::Block(b) => lower_block(b, source, comments)?,
            _ => {
                let colon = sel_spans[i].1;
                let boundary = if i + 1 < sels.len() {
                    sel_spans[i + 1].0
                } else {
                    content_end
                };
                let raw = source[colon + 1..boundary].trim();
                if raw.contains('\n') {
                    return None;
                }
                Doc::verbatim(raw.to_string())
            }
        };
        pairs.push(Doc::concat(vec![Doc::text(format!("{name}:")), arg_doc]));
    }

    // A single keyword has no continuation to align: emit `subject.k0:a0` and let the block arg
    // (if any) break on its own.
    if pairs.len() == 1 {
        return Some(Doc::concat(vec![
            subject,
            Doc::text("."),
            pairs.into_iter().next().unwrap(),
            tail_doc,
        ]));
    }

    // Multiple keywords: width-driven. Flat joins the pairs with a space; when that doesn't fit
    // the line budget (or an argument block spans lines, forcing the group to break), continuation
    // keywords drop onto their own lines aligned under the first — via a `Group` of `Line`s wrapped
    // in `Align` (which pins the break indent to the first keyword's column).
    {
        let mut inner = Vec::new();
        for (i, p) in pairs.into_iter().enumerate() {
            if i > 0 {
                inner.push(Doc::Line);
            }
            inner.push(p);
        }
        Some(Doc::concat(vec![
            subject,
            Doc::text("."),
            Doc::align(Doc::group(Doc::concat(inner))),
            tail_doc,
        ]))
    }
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
        // Trim a trailing comment before a `;` — a line comment may itself end in `;`
        // (e.g. `"* … cases;`), which must not be mistaken for a statement separator.
        if let Some(c) = comments.iter().find(|c| c.end == end && c.start >= start) {
            end = c.start;
            continue;
        }
        if bytes[end - 1] == b';' {
            end -= 1;
            continue;
        }
        break;
    }
    end
}

/// Split the comments in a gap into those that trail the previous statement (on its line,
/// before any newline) and those that lead the next one (on their own line). Once a newline is
/// seen, everything after is "leading".
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
