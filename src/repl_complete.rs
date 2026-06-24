//! REPL tab completion. A [`CompletionIndex`] is a plain owned snapshot of the surface
//! metadata completion needs — built once per input line from the live VM (which is frozen
//! while a line is edited, so the snapshot is never stale), via the read-only [`introspect`]
//! API. [`complete_input`] is then a *pure* function of `(line, pos, &index)`, so the actual
//! completion logic is unit-testable without a VM; the rustyline `Completer` is a thin
//! adapter over it. Design notes: `docs/INTROSPECTION.md` and the REPL section of QUOIN_TODO.
//!
//! v1 scope: bare-word completion (globals, session locals, a few keywords), namespace names
//! inside `[ … ]`, and selectors after `recv.` when `recv` is a simple identifier — a class
//! name (→ its class-side selectors) or a session local (→ its value's class instance
//! selectors, inherited included). Complex receivers (literals, `@ivars`, `(expr)`, chained
//! sends) and namespaced class names after `]` are out of scope and yield nothing.

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
    let mut class_side: HashMap<String, Vec<String>> = HashMap::new();
    let mut instance_side: HashMap<String, Vec<String>> = HashMap::new();

    for g in introspect::globals(vm) {
        // Only plain identifiers are completable as bare words; namespaced names like
        // `[IO]File` are reachable via the `[ … ]` path, not a bare fragment.
        if is_plain_ident(&g.name) {
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

    CompletionIndex {
        words,
        namespaces: introspect::find_namespaces(vm, ""),
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

    // Method-send position: the fragment is directly preceded by a single `.` (a `..` is a
    // range operator, not a send, so its RHS completes as a bare word instead).
    if fstart > 0 && bytes[fstart - 1] == b'.' && !(fstart >= 2 && bytes[fstart - 2] == b'.') {
        let dot = fstart - 1;
        let rstart = ident_start(bytes, dot);
        let recv = &line[rstart..dot];
        // Only a simple identifier receiver resolves — not an `@ivar`, a chained send, or a
        // bracketed/parenthesized expression (we don't evaluate the receiver in v1).
        let complex = rstart > 0 && matches!(bytes[rstart - 1], b'@' | b'.' | b')' | b']' | b'}');
        if !recv.is_empty() && !complex {
            if let Some(sels) = index.class_side.get(recv) {
                return (fstart, filter_prefix(sels, frag));
            }
            if let Some(sels) = index
                .local_class
                .get(recv)
                .and_then(|c| index.instance_side.get(c))
            {
                return (fstart, filter_prefix(sels, frag));
            }
        }
        // A send we can't resolve: offer nothing rather than bare words after a `.`.
        return (fstart, Vec::new());
    }

    // Bare word.
    (fstart, filter_prefix(&index.words, frag))
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
