//! Syntax highlighter
//!
//! It walks the parsed AST and produces a list of [`HighlightSpan`]s — byte
//! ranges into the original source tagged with a [`HighlightType`]. This is the
//! VM/runtime-free *span-producing core*; the ANSI rendering (`format_ansi`,
//! `highlight_to_ansi`) that depends on the VM's `ansi_colorizer` lives in the
//! `quoin` crate's `highlighter` module, which re-exports the items here. The
//! language server consumes these spans directly to emit LSP semantic tokens,
//! and reads [`colors_for`] to generate a matching theme.

use crate::ast::{
    BlockNode, IdentifierNode, IdentifierType, MethodCallNode, Node, NodeValue, UseNode,
};
use crate::source_info::SourceInfo;

use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum HighlightType {
    None = 0,
    ErrorStatement,
    MethodReturnStatement,
    BlockReturnStatement,
    NumberLiteral,
    StringLiteral,
    SymbolLiteral,
    RegexLiteral,
    Identifier,
    InstanceIdentifier,
    BlockBrace,
    CollectionBrace,
    Operator,
    Comment,
    MethodSignature,
    Global,
    Namespace,
    /// The `use` keyword (and a slot for future keywords).
    Keyword,
    /// A `use` target path (and its trailing `/*` glob).
    Path,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HighlightSpan {
    pub start: usize,
    pub end: usize,
    pub htype: HighlightType,
    pub counter: usize,
}

impl HighlightSpan {
    fn new(start: usize, end: usize, htype: HighlightType, counter: usize) -> Self {
        Self {
            start,
            end,
            htype,
            counter,
        }
    }
}

/// Highlight `source`, tolerating syntactically **incomplete** input. If `source` parses,
/// it is highlighted directly. Otherwise a minimal completion (see
/// [`crate::complete::complete_source`]) is appended so it parses, the completed source is
/// highlighted, and only the spans lying within the original input are returned — the
/// synthetic completion is highlighted but cropped away (it is append-only, so the original
/// tokens keep their offsets). Never panics; an input that cannot be completed yields no
/// spans. This is the entry point for live highlighting of as-you-type input (the REPL, the
/// language server) where the underlying parser would otherwise panic on a partial line.
pub fn highlight_resilient(source: &str) -> Vec<HighlightSpan> {
    if let Some(node) = crate::complete::parse_or_none(source) {
        return HighlightParser::new(source).highlight_program(&node);
    }
    if let Some(suffix) = crate::complete::complete_source(source) {
        let completed = format!("{source}{suffix}");
        if let Some(node) = crate::complete::parse_or_none(&completed) {
            let orig = source.len();
            let mut spans = HighlightParser::new(&completed).highlight_program(&node);
            spans.retain(|s| s.start < orig);
            for s in &mut spans {
                s.end = s.end.min(orig);
            }
            return spans;
        }
    }
    Vec::new()
}

/// The **reserved identifiers** — `true`/`false`/`nil`. These are distinct from
/// *keywords* (e.g. `use`, `HighlightType::Keyword`); reserved identifiers are colored
/// as globals, keywords get their own type.
const RESERVED_IDENTIFIERS: [&str; 3] = ["true", "false", "nil"];

/// Color attribute spec per highlight type. The chosen variant is
/// `colors[counter % colors.len()]`. Each entry is a hex color optionally
/// suffixed with a style flag (`;bw` bold, `;lw` light/dim); the language
/// server parses these to build a semantic-token theme.
pub fn colors_for(htype: HighlightType) -> &'static [&'static str] {
    match htype {
        HighlightType::None => &["#ffffff"],
        HighlightType::ErrorStatement => &["#d9534f;bw"],
        HighlightType::MethodReturnStatement => &["#ffffff"],
        HighlightType::BlockReturnStatement => &["#ffffff"],
        HighlightType::NumberLiteral => &["#00bfff"],
        HighlightType::StringLiteral => &["#4682b4"],
        HighlightType::SymbolLiteral => &["#4682b4"],
        HighlightType::RegexLiteral => &["#4682b4"],
        HighlightType::Identifier => &["#5fd7af", "#aeb1ab", "#c79ca9", "#85b9a5"],
        HighlightType::InstanceIdentifier => &["#6ab1c2"],
        HighlightType::BlockBrace => &["#f79c88", "#80f0ff", "#fa859d", "#eabe95", "#a4dbbe"],
        HighlightType::CollectionBrace => &["#93c6a5"],
        HighlightType::Operator => &["#ffffff"],
        HighlightType::Comment => &["#b9bdba;lw"],
        HighlightType::MethodSignature => &["#ab82ff"],
        HighlightType::Global => &["#ef65a5"],
        HighlightType::Namespace => &["#d53b82"],
        HighlightType::Keyword => &["#e0a45a;bw"], // amber, bold
        HighlightType::Path => &["#6aa9e0"],       // steel-blue
    }
}

// Comments are skipped at the lexing phase (they can appear anywhere and don't
// fit the AST), so we scan plain-text gaps for them. A comment is either a
// `"*`-to-end-of-line line comment or a `"..."` quote-delimited block comment.
static COMMENT_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r#""\*[^\n]*|"[^"]*""#).unwrap());

fn si_range(si: &Option<SourceInfo>) -> Option<(usize, usize)> {
    si.as_ref().map(|s| (s.start, s.end))
}

fn slice(source: &str, start: usize, end: usize) -> &str {
    source.get(start..end).unwrap_or("")
}

pub struct HighlightParser<'a> {
    source: &'a str,
    /// Stack of scopes; each maps identifier name -> the depth at which it was
    /// first seen, used to pick a stable color variant.
    depth_stack: Vec<HashMap<String, usize>>,
}

impl<'a> HighlightParser<'a> {
    pub fn new(source: &'a str) -> Self {
        Self {
            source,
            depth_stack: Vec::new(),
        }
    }

    pub fn highlight_program(&mut self, prog: &Node) -> Vec<HighlightSpan> {
        let statements = match &prog.value {
            NodeValue::Program(p) => &p.expressions,
            _ => return Vec::new(),
        };

        self.depth_stack.push(HashMap::new());

        let mut spans: Vec<HighlightSpan> = Vec::new();
        for stmt in statements {
            spans.extend(self.highlight_statement(stmt, 0));
        }

        if let Some(first) = spans.first() {
            if first.start > 0 {
                let before = text_span(self.source, 0, first.start.saturating_sub(1), 0);
                spans.splice(0..0, before);
            }
        }

        if let Some(last) = spans.last() {
            if last.end < self.source.len() {
                let after = text_span(self.source, last.end, self.source.len(), 0);
                spans.extend(after);
            }
        }

        let (ps, pe) = si_range(&prog.source_info).unwrap_or((0, self.source.len()));
        fill_in_gaps(self.source, ps, pe, spans, 0)
    }

    fn highlight_statement(&mut self, stmt: &Node, depth: usize) -> Vec<HighlightSpan> {
        let spans = match &stmt.value {
            NodeValue::Bang3 | NodeValue::Dot3 | NodeValue::Huh3 => {
                single_span_node(stmt, HighlightType::ErrorStatement, depth)
            }
            NodeValue::Assignment(a) => {
                let mut s = Vec::new();
                for lv in &a.lvalues {
                    s.extend(self.highlight_lvalue(lv, depth));
                }
                s.extend(self.highlight_expression(&a.rvalue, depth));
                s
            }
            NodeValue::BlockReturn(b) => self.highlight_expression(&b.value, depth),
            NodeValue::MethodReturn(m) => self.highlight_expression(&m.value, depth),
            NodeValue::YieldReturn(y) => self.highlight_expression(&y.value, depth),
            NodeValue::ConstDefinition(c) => {
                let mut s = self.highlight_identifier(&c.identifier, depth);
                s.extend(self.highlight_expression(&c.rvalue, depth));
                s
            }
            NodeValue::Use(u) => match si_range(&stmt.source_info) {
                Some((start, end)) => self.highlight_use(u, start, end, depth),
                None => Vec::new(),
            },
            _ => self.highlight_expression(stmt, depth),
        };

        match si_range(&stmt.source_info) {
            Some((start, end)) => fill_in_gaps(self.source, start, end, spans, depth),
            None => spans,
        }
    }

    /// `use (pkg:)? path (/*)?` — three spans: the `use` keyword, the optional `pkg:`
    /// qualifier (namespace hue), and the path. Offsets are derived from the statement
    /// span: the keyword is always the first three bytes, and the target is contiguous
    /// (no internal whitespace), so the package length locates the path. `[start, end)`
    /// excludes the trailing `;`. The caller's `fill_in_gaps` covers the whitespace.
    fn highlight_use(
        &self,
        u: &UseNode,
        start: usize,
        end: usize,
        depth: usize,
    ) -> Vec<HighlightSpan> {
        let bytes = self.source.as_bytes();
        let mut spans = Vec::new();

        // `use` keyword.
        let kw_end = (start + 3).min(end);
        spans.push(HighlightSpan::new(
            start,
            kw_end,
            HighlightType::Keyword,
            depth,
        ));

        // The target follows the whitespace after `use`.
        let mut target_start = kw_end;
        while target_start < end && bytes[target_start].is_ascii_whitespace() {
            target_start += 1;
        }

        // Optional `pkg:` qualifier (name + colon), colored as a namespace.
        let mut path_start = target_start;
        if let Some(pkg) = &u.package {
            let pkg_end = (target_start + pkg.len() + 1).min(end);
            spans.push(HighlightSpan::new(
                target_start,
                pkg_end,
                HighlightType::Namespace,
                depth,
            ));
            path_start = pkg_end;
        }

        // The path itself (including any trailing `/*`).
        if path_start < end {
            spans.push(HighlightSpan::new(
                path_start,
                end,
                HighlightType::Path,
                depth,
            ));
        }
        spans
    }

    fn highlight_expression(&mut self, expr: &Node, depth: usize) -> Vec<HighlightSpan> {
        let spans = match &expr.value {
            NodeValue::ClassDefinition(cd) => {
                let mut s = Vec::new();
                if let Some(parent) = &cd.parent_identifier {
                    s.extend(self.highlight_identifier(parent, depth));
                }
                s.extend(self.highlight_identifier(&cd.identifier, depth));
                s.extend(self.highlight_block(&cd.block, depth));
                s
            }
            NodeValue::ClassExtension(ce) => {
                let mut s = self.highlight_expression(&ce.expression, depth);
                s.extend(self.highlight_block(&ce.block, depth));
                s
            }
            NodeValue::MethodDefinition(md) => {
                let mut s = Vec::new();
                for id in &md.signature.identifiers {
                    s.extend(single_span_ident(id, HighlightType::MethodSignature, depth));
                }
                s.extend(self.highlight_block(&md.block, depth));
                s
            }
            NodeValue::MethodExtension(me) => {
                let mut s = Vec::new();
                for id in &me.signature.identifiers {
                    s.extend(single_span_ident(id, HighlightType::MethodSignature, depth));
                }
                s.extend(self.highlight_block(&me.block, depth));
                s
            }
            NodeValue::Double(_) | NodeValue::Integer(_) => {
                single_span_node(expr, HighlightType::NumberLiteral, depth)
            }
            NodeValue::Str(_) => single_span_node(expr, HighlightType::StringLiteral, depth),
            NodeValue::Symbol(_) => single_span_node(expr, HighlightType::SymbolLiteral, depth),
            NodeValue::Regex(_) => single_span_node(expr, HighlightType::RegexLiteral, depth),
            NodeValue::MethodCall(call) => self.highlight_method_call(expr, call, depth),
            NodeValue::Block(block) => self.highlight_block_value(block, depth),
            NodeValue::Identifier(ident) => self.highlight_identifier_node(ident, depth),
            NodeValue::Map(dict) => {
                // interleave key, value, key, value, ...
                let mut elements: Vec<Arc<Node>> = Vec::new();
                for (k, v) in dict.keys.iter().zip(dict.values.iter()) {
                    elements.push(k.clone());
                    elements.push(v.clone());
                }
                self.highlight_braced(expr, &elements, 2, depth)
            }
            NodeValue::List(list) => self.highlight_braced(expr, &list.values, 2, depth),
            NodeValue::Set(set) => self.highlight_braced(expr, &set.values, 2, depth),
            NodeValue::BinaryOperator(bin) => {
                let mut s = self.highlight_expression(&bin.left, depth);
                s.extend(self.highlight_expression(&bin.right, depth));
                s
            }
            NodeValue::UnaryOperator(un) => self.highlight_expression(&un.right, depth),
            NodeValue::UserList(ul) => {
                self.highlight_user_list(expr, &ul.identifier, &ul.values, depth)
            }
            NodeValue::UserString(us) => self.highlight_user_string(expr, &us.identifier, depth),
            _ => Vec::new(),
        };

        match si_range(&expr.source_info) {
            Some((start, end)) => fill_in_gaps(self.source, start, end, spans, depth),
            None => spans,
        }
    }

    fn highlight_lvalue(&mut self, lv: &Node, depth: usize) -> Vec<HighlightSpan> {
        match &lv.value {
            NodeValue::IdentLValue(ilv) => self.highlight_identifier(&ilv.identifier, depth),
            NodeValue::SplatLValue(slv) => self.highlight_identifier(&slv.identifier, depth),
            NodeValue::IgnoredLValue | NodeValue::IgnoredSplatLValue => {
                single_span_node(lv, HighlightType::Identifier, depth)
            }
            NodeValue::SubLValue(sub) => {
                let mut s = Vec::new();
                for inner in &sub.lvalues {
                    s.extend(self.highlight_lvalue(inner, depth));
                }
                s
            }
            _ => Vec::new(),
        }
    }

    fn highlight_method_call(
        &mut self,
        expr: &Node,
        call: &MethodCallNode,
        depth: usize,
    ) -> Vec<HighlightSpan> {
        let mut spans = Vec::new();

        if let Some(subject) = &call.subject {
            spans.extend(self.highlight_expression(subject, depth));
        }

        let sig = &call.arguments.signature.identifiers;
        let exprs = &call.arguments.expressions;

        if !exprs.is_empty() {
            for (s, arg) in sig.iter().zip(exprs.iter()) {
                spans.extend(single_span_ident(s, HighlightType::MethodSignature, depth));
                spans.extend(self.highlight_expression(arg, depth));
            }
        } else if let Some(first) = sig.first() {
            spans.extend(single_span_ident(
                first,
                HighlightType::MethodSignature,
                depth,
            ));
        }

        match si_range(&expr.source_info) {
            Some((start, end)) => fill_in_gaps(self.source, start, end, spans, depth),
            None => spans,
        }
    }

    /// Dictionary / List / Set — `start_brace_len` is 2 for `#{`, `#(`, `#<`.
    fn highlight_braced(
        &mut self,
        expr: &Node,
        elements: &[Arc<Node>],
        start_brace_len: usize,
        depth: usize,
    ) -> Vec<HighlightSpan> {
        let (start, end) = match si_range(&expr.source_info) {
            Some(r) => r,
            None => return Vec::new(),
        };
        let mut spans = Vec::new();
        spans.push(HighlightSpan::new(
            start,
            start + start_brace_len,
            HighlightType::CollectionBrace,
            depth,
        ));
        for el in elements {
            spans.extend(self.highlight_expression(el, depth));
        }
        spans.push(HighlightSpan::new(
            end - 1,
            end,
            HighlightType::CollectionBrace,
            depth,
        ));
        fill_in_gaps(self.source, start, end, spans, depth)
    }

    fn highlight_user_list(
        &mut self,
        expr: &Node,
        identifier: &IdentifierNode,
        values: &[Arc<Node>],
        depth: usize,
    ) -> Vec<HighlightSpan> {
        let (start, end) = match si_range(&expr.source_info) {
            Some(r) => r,
            None => return Vec::new(),
        };
        let name_len = identifier.name.len();
        let ident_start = start + 1; // after '#'
        let ident_end = ident_start + name_len;

        let mut spans = Vec::new();
        // leading '#'
        spans.push(HighlightSpan::new(
            start,
            ident_start,
            HighlightType::CollectionBrace,
            depth,
        ));
        // identifier
        spans.extend(single_span_range(
            ident_start,
            ident_end,
            HighlightType::Identifier,
            depth,
        ));
        // opening '('
        spans.push(HighlightSpan::new(
            ident_end,
            ident_end + 1,
            HighlightType::CollectionBrace,
            depth,
        ));
        for el in values {
            spans.extend(self.highlight_expression(el, depth));
        }
        // closing ')'
        spans.push(HighlightSpan::new(
            end - 1,
            end,
            HighlightType::CollectionBrace,
            depth,
        ));
        fill_in_gaps(self.source, start, end, spans, depth)
    }

    fn highlight_user_string(
        &mut self,
        expr: &Node,
        identifier: &IdentifierNode,
        depth: usize,
    ) -> Vec<HighlightSpan> {
        let (start, end) = match si_range(&expr.source_info) {
            Some(r) => r,
            None => return Vec::new(),
        };
        let name_len = identifier.name.len();
        let ident_start = start + 1; // after '#'
        let ident_end = ident_start + name_len;

        let mut spans = Vec::new();
        // leading '#'
        spans.push(HighlightSpan::new(
            start,
            ident_start,
            HighlightType::StringLiteral,
            depth,
        ));
        // identifier
        spans.extend(single_span_range(
            ident_start,
            ident_end,
            HighlightType::Identifier,
            depth,
        ));
        // the quoted string body
        spans.push(HighlightSpan::new(
            ident_end,
            end,
            HighlightType::StringLiteral,
            depth,
        ));
        fill_in_gaps(self.source, start, end, spans, depth)
    }

    fn highlight_block_value(&mut self, block: &BlockNode, depth: usize) -> Vec<HighlightSpan> {
        self.highlight_block(block, depth)
    }

    fn highlight_block(&mut self, block: &BlockNode, depth: usize) -> Vec<HighlightSpan> {
        let (start, end) = match si_range(&block.source_info) {
            Some(r) => r,
            None => return Vec::new(),
        };

        let depth = depth + 1;
        self.depth_stack.push(HashMap::new());

        let mut spans = Vec::new();
        // opening brace
        spans.push(HighlightSpan::new(
            start,
            start + 1,
            HighlightType::BlockBrace,
            depth,
        ));

        // (block name is a SymbolNode without source offsets — span omitted)

        for arg in &block.arguments {
            spans.extend(self.highlight_identifier(&arg.identifier, depth));
            if let Some(type_hint) = &arg.type_hint {
                spans.extend(self.highlight_identifier(type_hint, depth));
            }
        }

        if let Some(decl_block) = &block.decl_block {
            spans.extend(self.highlight_block(decl_block, depth));
        }

        for decl in &block.decls {
            spans.extend(self.highlight_identifier(&decl.identifier, depth));
        }

        for stmt in &block.statements {
            spans.extend(self.highlight_statement(stmt, depth));
        }

        // closing brace
        spans.push(HighlightSpan::new(
            end - 1,
            end,
            HighlightType::BlockBrace,
            depth,
        ));

        self.depth_stack.pop();

        fill_in_gaps(self.source, start, end, spans, depth)
    }

    fn highlight_identifier_node(
        &mut self,
        ident: &IdentifierNode,
        depth: usize,
    ) -> Vec<HighlightSpan> {
        self.highlight_identifier(ident, depth)
    }

    fn highlight_identifier(&mut self, ident: &IdentifierNode, depth: usize) -> Vec<HighlightSpan> {
        let ident_depth = self.lookup_or_insert_depth(&ident.name, depth);

        let (start, end) = match si_range(&ident.source_info) {
            Some(r) => r,
            None => return Vec::new(),
        };

        let is_reserved = RESERVED_IDENTIFIERS.contains(&ident.name.as_str());
        let starts_upper = ident
            .name
            .chars()
            .next()
            .map(|c| c.is_uppercase())
            .unwrap_or(false);

        if is_reserved || starts_upper {
            if ident.identifier_type == IdentifierType::Namespaced {
                return self.namespaced_spans(
                    ident,
                    start,
                    end,
                    HighlightType::Global,
                    ident_depth,
                );
            }
            return vec![HighlightSpan::new(
                start,
                end,
                HighlightType::Global,
                ident_depth,
            )];
        }

        if ident.identifier_type == IdentifierType::Instance {
            return vec![HighlightSpan::new(
                start,
                end,
                HighlightType::InstanceIdentifier,
                ident_depth,
            )];
        }

        if ident.identifier_type == IdentifierType::Namespaced {
            return self.namespaced_spans(
                ident,
                start,
                end,
                HighlightType::Identifier,
                ident_depth,
            );
        }

        vec![HighlightSpan::new(
            start,
            end,
            HighlightType::Identifier,
            ident_depth,
        )]
    }

    /// Build spans for a namespaced identifier: a `Namespace` span over the
    /// `[..]` prefix, then a `name_type` span over the remaining name only
    /// (so the namespace text isn't emitted twice).
    fn namespaced_spans(
        &self,
        ident: &IdentifierNode,
        start: usize,
        end: usize,
        name_type: HighlightType,
        ident_depth: usize,
    ) -> Vec<HighlightSpan> {
        let mut spans = Vec::new();
        let mut name_start = start;
        if let Some(ns) = &ident.namespace {
            if let Some((ns_start, ns_end)) = si_range(&ns.source_info) {
                spans.push(HighlightSpan::new(
                    ns_start,
                    ns_end,
                    HighlightType::Namespace,
                    ident_depth,
                ));
                name_start = ns_end;
            }
        }
        spans.push(HighlightSpan::new(name_start, end, name_type, ident_depth));
        spans
    }

    fn lookup_or_insert_depth(&mut self, name: &str, depth: usize) -> usize {
        for scope in self.depth_stack.iter().rev() {
            if let Some(&found) = scope.get(name) {
                return found;
            }
        }
        if let Some(top) = self.depth_stack.last_mut() {
            top.insert(name.to_string(), depth);
        }
        depth
    }
}

fn single_span_node(node: &Node, htype: HighlightType, counter: usize) -> Vec<HighlightSpan> {
    match si_range(&node.source_info) {
        Some((start, end)) => vec![HighlightSpan::new(start, end, htype, counter)],
        None => Vec::new(),
    }
}

fn single_span_ident(
    ident: &IdentifierNode,
    htype: HighlightType,
    counter: usize,
) -> Vec<HighlightSpan> {
    match si_range(&ident.source_info) {
        Some((start, end)) => vec![HighlightSpan::new(start, end, htype, counter)],
        None => Vec::new(),
    }
}

fn single_span_range(
    start: usize,
    end: usize,
    htype: HighlightType,
    counter: usize,
) -> Vec<HighlightSpan> {
    vec![HighlightSpan::new(start, end, htype, counter)]
}

/// Insert plain-text (and comment) spans into the gaps between `spans`,
/// covering `[full_start, ..)`.
fn fill_in_gaps(
    source: &str,
    full_start: usize,
    _full_end: usize,
    spans: Vec<HighlightSpan>,
    depth: usize,
) -> Vec<HighlightSpan> {
    if spans.is_empty() {
        return spans;
    }

    let mut new_spans: Vec<HighlightSpan> = Vec::new();
    let mut pos = full_start;
    for span in spans {
        if pos < span.start {
            new_spans.extend(text_span(source, pos, span.start, depth));
        }
        pos = span.end;
        new_spans.push(span);
    }
    new_spans
}

/// Scan a plain-text gap `[start, end)` for comments, emitting Comment spans
/// for them and None spans (via recursion) for the rest.
fn text_span(source: &str, start: usize, end: usize, depth: usize) -> Vec<HighlightSpan> {
    if start >= end {
        return Vec::new();
    }

    let text = slice(source, start, end);
    let mut spans = Vec::new();
    let mut pos = start;

    for m in COMMENT_RE.find_iter(text) {
        let m_start = start + m.start();
        let m_end = start + m.end();
        spans.push(HighlightSpan::new(
            m_start,
            m_end,
            HighlightType::Comment,
            depth,
        ));
        pos = m_end;
    }

    // The trailing remainder after the last comment is plain text, not a
    // comment. Non-comment gaps before/between comments are filled with
    // None spans by fill_in_gaps below.
    if pos < end {
        spans.push(HighlightSpan::new(pos, end, HighlightType::None, depth));
    }

    if pos == start {
        return vec![HighlightSpan::new(start, end, HighlightType::None, depth)];
    }

    fill_in_gaps(source, start, end, spans, depth)
}

#[cfg(test)]
mod resilient_tests {
    use super::*;

    #[test]
    fn crops_completion_to_original() {
        for src in ["1 +", "Foo <- {", "#(1 2", "'hello", "a.foo:", "Box <--"] {
            let spans = highlight_resilient(src);
            assert!(
                spans
                    .iter()
                    .all(|s| s.start < src.len() && s.end <= src.len()),
                "spans escape original input for {src:?}: {spans:?}"
            );
        }
    }

    #[test]
    fn matches_plain_highlight_when_valid() {
        let src = "x = 1 + 2";
        let node = crate::try_parse_quoin_string_named(src, "<t>").unwrap();
        let plain = HighlightParser::new(src).highlight_program(&node);
        assert_eq!(highlight_resilient(src), plain);
    }

    #[test]
    fn never_panics_on_arbitrary_input() {
        // Includes `Box <-- 0`, which pest-parses but panics in the AST builder — the
        // resilient path must catch it (it prints a panic message but does not abort).
        for src in [
            "",
            ")",
            "@",
            "1 +",
            "'",
            "#/ab",
            "{{{{",
            "})(",
            "Foo <-",
            "use ",
            "Box <-- 0",
        ] {
            let _ = highlight_resilient(src);
        }
    }
}
