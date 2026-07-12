//! REPL tab completion. A [`CompletionIndex`] is a plain owned snapshot of the surface
//! metadata completion needs — built once per input line from the live VM (which is frozen
//! while a line is edited, so the snapshot is never stale), via the read-only [`introspect`]
//! API. [`complete_input`] is then a *pure* function of `(line, pos, &index)`, so the actual
//! completion logic is unit-testable without a VM; the rustyline `Completer` is a thin
//! adapter over it. Design notes: `docs/internal/INTROSPECTION.md` and the REPL section of QUOIN_TODO.
//!
//! v1 scope: bare-word completion (globals, session locals, a few keywords), namespace names
//! inside `[ … ]`, and selectors after `recv.` when the receiver's class is statically known
//! — a class name (→ its class-side selectors), a session local (→ its value's class instance
//! selectors, inherited included), or a syntactically-typed literal (string / integer /
//! `true`/`false` / `nil` → that class's instance selectors). Receivers we'd have to evaluate
//! (`@ivars`, `(expr)`, chained sends, and richer literals like lists/maps/sets/regex) and
//! namespaced class names after `]` are out of scope and yield nothing.

use crate::introspect::{self, GlobalKind};
use crate::vm::VmState;

use std::collections::HashMap;

/// An owned, `'static` snapshot of everything [`complete_input`] needs — detached from the
/// GC arena (the whole point of the [`introspect`] structs being `'static`).
#[derive(Debug, Default, Clone)]
pub struct CompletionIndex {
    /// Bare-word candidates: plain-identifier global names + session-local names + keywords.
    pub words: Vec<String>,
    /// Namespace path names (`IO`, `HTTP`, …) — completed inside `[ … ]`.
    pub namespaces: Vec<String>,
    /// Fully-qualified namespaced global names (`[IO]File`, `[HTTP]Parser`, …) — completed
    /// after a closed `[ns]` (`[IO]Fi` → `[IO]File`).
    pub namespaced: Vec<String>,
    /// Class name → its class-side selectors (for a `ClassName.` receiver).
    pub class_side: HashMap<String, Vec<String>>,
    /// Class name → its instance selectors, inherited included (for a value receiver).
    pub instance_side: HashMap<String, Vec<String>>,
    /// Session-local name → the class name of its current value (to resolve `local.`).
    pub local_class: HashMap<String, String>,
}

/// Snapshot the completion-relevant surface metadata from the live VM. One read-only pass;
/// returns owned data with no `'gc` lifetime, so the caller keeps it past the arena borrow.
pub fn build_completion_index<'gc>(vm: &VmState<'gc>) -> CompletionIndex {
    let mut words: Vec<String> = Vec::new();
    let mut namespaced: Vec<String> = Vec::new();
    let mut class_side: HashMap<String, Vec<String>> = HashMap::new();
    let mut instance_side: HashMap<String, Vec<String>> = HashMap::new();

    for g in introspect::globals(vm) {
        // A namespaced name (`[IO]File`) is completed via the `[ns]…` path, not as a bare
        // word; a plain identifier is a bare-word candidate.
        if g.name.starts_with('[') {
            namespaced.push(g.name.clone());
        } else if is_plain_ident(&g.name) {
            words.push(g.name.clone());
        }
        if let GlobalKind::Class = g.kind {
            if let Some(info) = introspect::describe_class(vm, &g.name) {
                let mut cs: Vec<String> = info
                    .class_methods
                    .iter()
                    .map(|m| m.selector.clone())
                    .collect();
                cs.sort();
                cs.dedup();
                class_side.insert(g.name.clone(), cs);
            }
            let is = introspect::find_selectors(vm, &g.name, "", true); // sorted (BTreeSet)
            instance_side.insert(g.name.clone(), is);
        }
    }

    let mut local_class: HashMap<String, String> = HashMap::new();
    for b in introspect::session_locals(vm) {
        words.push(b.name.clone());
        local_class.insert(b.name, b.class);
    }

    // Literals/pseudo-vars that aren't globals but are valid bare words.
    for kw in ["self", "super", "true", "false", "nil"] {
        words.push(kw.to_string());
    }
    words.sort();
    words.dedup();

    namespaced.sort();
    namespaced.dedup();

    CompletionIndex {
        words,
        namespaces: introspect::find_namespaces(vm, ""),
        namespaced,
        class_side,
        instance_side,
        local_class,
    }
}

/// Complete the token ending at `pos`. Returns the byte offset where the replacement starts
/// (the start of the current fragment) and the candidate list (full tokens, sorted). An
/// unresolvable position returns an empty candidate list (with a sensible start offset).
pub fn complete_input(line: &str, pos: usize, index: &CompletionIndex) -> (usize, Vec<String>) {
    let bytes = line.as_bytes();
    let pos = pos.min(line.len());
    let fstart = ident_start(bytes, pos);
    let frag = &line[fstart..pos];

    // Namespace position: an unclosed `[` before the fragment. Complete the whole path from
    // just after the `[` (paths may contain `/`, which isn't an identifier byte).
    if let Some(open) = open_namespace_bracket(&bytes[..fstart]) {
        let nsfrag = &line[open + 1..pos];
        return (open + 1, filter_prefix(&index.namespaces, nsfrag));
    }

    // Namespaced-name position: the fragment sits right after a closed `[ns]` (`[IO]Fi`).
    // Complete the fully-qualified name, replacing from the `[`. (`[` is namespace-only in the
    // grammar, so a `]` here always closes one.)
    if fstart > 0 && bytes[fstart - 1] == b']' {
        if let Some(open) = bytes[..fstart - 1].iter().rposition(|&c| c == b'[') {
            let prefix = &line[open..pos];
            return (open, filter_prefix(&index.namespaced, prefix));
        }
    }

    // Method-send position: the fragment is directly preceded by a single `.` (a `..` is a
    // range operator, not a send, so its RHS completes as a bare word instead).
    if fstart > 0 && bytes[fstart - 1] == b'.' && !(fstart >= 2 && bytes[fstart - 2] == b'.') {
        let dot = fstart - 1;
        // A send: offer the receiver's selectors, or — when its class can't be known without
        // evaluating it — nothing (never bare words after a `.`).
        let cands = receiver_selectors(line, bytes, dot, index)
            .map(|sels| filter_prefix(sels, frag))
            .unwrap_or_default();
        return (fstart, cands);
    }

    // Bare word.
    (fstart, filter_prefix(&index.words, frag))
}

/// Selectors to offer for the receiver expression ending at byte `dot` (the send `.`), or
/// `None` when the receiver's class can't be known without evaluating it. Resolves: a class
/// name used as a value → its class-side selectors; a syntactically-typed literal (string /
/// integer / `true`/`false` / `nil`) → that class's instance selectors; and a session local
/// → its value's instance selectors. Complex receivers (`@ivar`, chained sends, `(expr)`,
/// and other literals like lists/maps/sets/regex) return `None`.
fn receiver_selectors<'i>(
    line: &str,
    bytes: &[u8],
    dot: usize,
    index: &'i CompletionIndex,
) -> Option<&'i Vec<String>> {
    // Identifier / keyword / integer-literal / `#symbol` receiver: the run of identifier bytes
    // before `.`.
    let rstart = ident_start(bytes, dot);
    let recv = &line[rstart..dot];
    if !recv.is_empty() {
        // `#foo` is a bareword symbol literal — a Symbol value.
        if rstart > 0 && bytes[rstart - 1] == b'#' {
            return index.instance_side.get("Symbol");
        }
        // Preceded by `@`/`.`/`)`/`]`/`}` → a sub-expression we can't type without evaluating.
        if rstart > 0 && matches!(bytes[rstart - 1], b'@' | b'.' | b')' | b']' | b'}') {
            return None;
        }
        // A class name used as a value → class-side; a typed literal → instance-side; a
        // session local → its value's instance-side.
        if let Some(sels) = index.class_side.get(recv) {
            return Some(sels);
        }
        if let Some(cls) = literal_class(recv) {
            return index.instance_side.get(cls);
        }
        return index
            .local_class
            .get(recv)
            .and_then(|c| index.instance_side.get(c));
    }
    // Non-identifier receiver: a literal whose closing delimiter sits just before the `.` — a
    // string/symbol, a `#`-sigil collection/regex, or a block. Its class is fully determined
    // by syntax, so no evaluation is needed.
    closing_literal_class(bytes, dot).and_then(|cls| index.instance_side.get(cls))
}

/// The class of a receiver written as a bare keyword/integer literal: all-digits → `Integer`,
/// `true`/`false` → `Boolean`, `nil` → `Nil`. (`self`/`super` have no statically-known class.)
fn literal_class(recv: &str) -> Option<&'static str> {
    if recv.bytes().all(|b| b.is_ascii_digit()) {
        return Some("Integer");
    }
    match recv {
        "true" | "false" => Some("Boolean"),
        "nil" => Some("Nil"),
        _ => None,
    }
}

/// The class of a literal whose closing delimiter sits at byte `dot - 1`, determined purely
/// from syntax by a string/regex/nesting-aware forward scan of `bytes[..dot]`: `'…'`→`String`,
/// `#'…'`→`Symbol`, `#(…)`→`List`, `#{…}`→`Map`, `#<…>`→`Set`, `#/…/`→`Regex`, and a bare
/// `{…}` block→`Block`. `None` if the closer is a plain `(…)` grouping (its value's type isn't
/// syntactic) or doesn't close such a literal. Brackets/quotes inside strings and regexes are
/// ignored, and `<`/`>` count as brackets only with a `#` sigil (else they're comparisons).
fn closing_literal_class(bytes: &[u8], dot: usize) -> Option<&'static str> {
    if dot == 0 {
        return None;
    }
    enum St {
        Normal,
        Str { hash: bool },
        Regex,
    }
    let mut st = St::Normal;
    let mut stack: Vec<(u8, bool)> = Vec::new(); // (opening bracket, had a `#` sigil)
    let target = dot - 1;
    let mut i = 0;
    while i < dot {
        let c = bytes[i];
        let hash = i > 0 && bytes[i - 1] == b'#';
        match st {
            St::Normal => match c {
                b'\'' => st = St::Str { hash },
                b'/' if hash => st = St::Regex,
                b'(' => stack.push((b'(', hash)),
                b'{' => stack.push((b'{', hash)),
                b'<' if hash => stack.push((b'<', true)),
                b')' => {
                    if let Some((b'(', had)) = stack.last().copied() {
                        stack.pop();
                        if i == target {
                            // `#(…)` is a List; a plain `(…)` grouping has no syntactic type.
                            return had.then_some("List");
                        }
                    }
                }
                b'}' => {
                    if let Some((b'{', had)) = stack.last().copied() {
                        stack.pop();
                        if i == target {
                            return Some(if had { "Map" } else { "Block" });
                        }
                    }
                }
                b'>' => {
                    if let Some((b'<', _)) = stack.last().copied() {
                        stack.pop();
                        if i == target {
                            return Some("Set");
                        }
                    }
                }
                _ => {}
            },
            St::Str { hash } => match c {
                b'\\' => i += 1, // skip the escaped char
                b'\'' => {
                    st = St::Normal;
                    if i == target {
                        return Some(if hash { "Symbol" } else { "String" });
                    }
                }
                _ => {}
            },
            St::Regex => match c {
                b'\\' => i += 1,
                b'/' => {
                    st = St::Normal;
                    if i == target {
                        return Some("Regex");
                    }
                }
                _ => {}
            },
        }
        i += 1;
    }
    None
}

/// Sorted, de-duplicated members of `items` that start with `prefix`.
fn filter_prefix(items: &[String], prefix: &str) -> Vec<String> {
    let mut out: Vec<String> = items
        .iter()
        .filter(|s| s.starts_with(prefix))
        .cloned()
        .collect();
    out.sort();
    out.dedup();
    out
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// All identifier bytes are ASCII, so a non-ident byte is always a char boundary — scanning
/// back byte-wise is UTF-8-safe.
fn is_plain_ident(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(is_ident_byte)
}

/// The start of the run of identifier bytes ending at `end`.
fn ident_start(bytes: &[u8], end: usize) -> usize {
    let mut s = end;
    while s > 0 && is_ident_byte(bytes[s - 1]) {
        s -= 1;
    }
    s
}

/// The byte offset of the last `[` in `bytes` if more `[` than `]` are open (namespaces are
/// flat, so a simple open-count suffices), else `None`.
fn open_namespace_bracket(bytes: &[u8]) -> Option<usize> {
    let opens = bytes.iter().filter(|&&c| c == b'[').count();
    let closes = bytes.iter().filter(|&&c| c == b']').count();
    if opens > closes {
        bytes.iter().rposition(|&c| c == b'[')
    } else {
        None
    }
}

#[cfg(test)]
#[path = "repl_complete_tests.rs"]
mod tests;
