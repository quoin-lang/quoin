//! Doc extraction: the `"*` block above a definition is its reference doc
//! (docs/internal/DOCS_ARCH.md §4).
//!
//! The parser drops comments (pest trivia), so docs are recovered from *source text* at the
//! location introspection reports (`MethodVariant::source`, `ClassInfo::source`). Extraction
//! is line-based rather than reusing `quoin-fmt`'s byte-ranged `scan_comments`: a doc line is
//! a line that contains *nothing but* a `"* …` comment, and since Quoin strings cannot span
//! lines, a line whose first non-blank characters are `"*` cannot be string interior — so no
//! string/regex-context tracking is needed for this narrower question.
//!
//! The adjacency rules (§4): a contiguous run of `"*` lines **immediately** above the
//! definition — no blank line between — is its doc; a blank line detaches the block into file
//! commentary. The extracted text strips the leading `"*` and at most one following space per
//! line. First line is the summary, the rest the body.

use crate::packages::read_stdlib_unit;

/// The doc block for a definition at 1-based `line` of `source`, under the §4 adjacency
/// rules. `None` when the line above is blank, code, or the top of the file.
pub fn doc_above(source: &str, line: usize) -> Option<String> {
    if line < 2 {
        return None;
    }
    let lines: Vec<&str> = source.lines().collect();
    // 0-based index of the definition line; scan upward from the line before it.
    let def = line.checked_sub(1)?;
    if def > lines.len() {
        return None;
    }
    let mut block: Vec<&str> = Vec::new();
    for i in (0..def).rev() {
        let trimmed = lines[i].trim_start();
        if let Some(rest) = trimmed.strip_prefix("\"*") {
            // Strip at most one space after the marker; keep deeper indentation (code in
            // fenced examples relies on it).
            block.push(rest.strip_prefix(' ').unwrap_or(rest));
        } else {
            break; // blank or code: the block (if any) ended
        }
    }
    if block.is_empty() {
        return None;
    }
    block.reverse();
    Some(block.join("\n"))
}

/// The doc block for the *method* whose block literal starts at 1-based `line`. Like
/// [`doc_above`], but resilient to a wrapped header: `qn fmt` breaks a long definition after
/// `->`, so the block literal — whose location is what introspection reports — starts a line
/// or more below the selector, and the doc block sits above the *selector*. A method header
/// line is unmistakable (the selector followed by `->`/`-->`), so when the reported line is
/// not a header, scan a short window upward for it and anchor there.
pub fn method_doc_above(source: &str, line: usize, selector: &str) -> Option<String> {
    /// How far a wrapped header can put the block below its selector. Two covers today's
    /// formatter (selector line + param line); the slack is for a future param-list wrap.
    const WRAP_WINDOW: usize = 8;

    let lines: Vec<&str> = source.lines().collect();
    let reported = line.checked_sub(1)?; // 0-based
    let is_header = |i: usize| {
        lines.get(i).is_some_and(|l| {
            l.trim_start().strip_prefix(selector).is_some_and(|rest| {
                // `-->` also passes the `->` prefix test, covering both arrow forms.
                rest.trim_start().starts_with("->")
            })
        })
    };
    if is_header(reported) {
        return doc_above(source, line);
    }
    for back in 1..=WRAP_WINDOW {
        let Some(i) = reported.checked_sub(back) else {
            break;
        };
        if is_header(i) {
            return doc_above(source, i + 1);
        }
    }
    // No header found (a runtime-built or unconventional definition): the old behavior.
    doc_above(source, line)
}

/// The first line of a doc — the summary shown in selector lists.
pub fn summary(doc: &str) -> &str {
    doc.lines().next().unwrap_or("")
}

/// Source text for a unit filename as introspection reports it (`SourceLoc::file`):
/// `[pkg:]path.qn` for `use`-loaded units, `prelude.qn`/`test.qn` for the bootstrap ones, or
/// a plain filesystem path for an entry script. Embedded stdlib units resolve from the binary
/// (`read_stdlib_unit` honours `QUOIN_STDLIB`), so `$doc`/`qn doc` work outside a checkout.
pub fn unit_source(file: &str) -> Option<String> {
    let unit = file.strip_suffix(".qn").unwrap_or(file);
    // `std:core/06-io` / bare `core/06-io` / `prelude` — try the stdlib first.
    let stdlib_key = unit.strip_prefix("std:").unwrap_or(unit);
    if let Some(text) = read_stdlib_unit(stdlib_key) {
        return Some(text);
    }
    // `self:app/util` — self-rooted package paths; in the doc/eval modes self_root is the
    // CWD, so a relative read matches how the unit was loaded.
    if let Some(rest) = unit.strip_prefix("self:") {
        return std::fs::read_to_string(format!("{rest}.qn")).ok();
    }
    // An entry script or other on-disk file, named as given.
    std::fs::read_to_string(file)
        .ok()
        .or_else(|| std::fs::read_to_string(format!("{unit}.qn")).ok())
}

#[cfg(test)]
#[path = "docs_tests.rs"]
mod docs_tests;
