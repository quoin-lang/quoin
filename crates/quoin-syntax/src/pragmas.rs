//! `"* allow: <kind>` pragma recovery. Pest treats `COMMENT` as a silent rule, so
//! pragmas — like doc blocks — must be re-scanned from raw source. Unlike doc
//! extraction's whole-line scan, a pragma *trails* code, so the scan tracks
//! string/regex/block-comment context (the state machine mirrors `complete.rs` and
//! `quoin-fmt`'s comment scanner): a `"* allow:` inside a `'…'` string, `#/…/`
//! regex, or `"…"` block comment is content, not a pragma.
//!
//! The scanner is syntax-only: it records what was written and where. Whether the
//! kind names exist, and whether the pragma actually trails code (`trailing`), are
//! the checker's judgements — it owns the warning taxonomy and the diagnostics.

use crate::ast::AllowPragma;
use crate::source_info::SourceInfo;

/// Scan `source` and return every `"* allow: …` pragma in source order.
///
/// All delimiters are ASCII, so this walks bytes; recorded ranges land on UTF-8
/// boundaries because multibyte content sits strictly between delimiters.
pub fn scan_allow_pragmas(source: &str, filename: &str) -> Vec<AllowPragma> {
    enum St {
        Normal,
        Str,
        Regex,
        /// A `"* …` line comment; `start` is the byte offset of its `"`.
        Line {
            start: usize,
        },
        Block,
    }

    let bytes = source.as_bytes();
    let n = bytes.len();
    let mut pragmas = Vec::new();
    let mut st = St::Normal;
    let mut line = 1usize;
    let mut line_start = 0usize;
    // Any non-whitespace byte seen on this line outside comments — i.e. the comment
    // trails code. String/regex content counts; block-comment content does not.
    let mut code_on_line = false;

    let close_line_comment = |start: usize,
                              end: usize,
                              line: usize,
                              line_start: usize,
                              trailing: bool,
                              pragmas: &mut Vec<AllowPragma>| {
        // Text after the `"*` marker, e.g. ` allow: nil-receiver (why…)`.
        let text = source[start + 2..end].trim();
        let Some(rest) = text.strip_prefix("allow:") else {
            return;
        };
        // Kinds run to end of line or to a `(rationale…)` — prose belongs in parens so a
        // typo'd kind name can't masquerade as commentary.
        let kinds: Vec<String> = rest
            .split('(')
            .next()
            .unwrap_or("")
            .split([',', ' '])
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        pragmas.push(AllowPragma {
            line,
            kinds,
            trailing,
            span: SourceInfo {
                filename: filename.to_string(),
                line,
                column: start - line_start,
                start,
                end,
                source_text: Some(source[start..end].to_string()),
            },
        });
    };

    let mut i = 0usize;
    while i < n {
        let c = bytes[i];
        if c == b'\n' {
            if let St::Line { start } = st {
                close_line_comment(start, i, line, line_start, code_on_line, &mut pragmas);
                st = St::Normal;
            }
            line += 1;
            line_start = i + 1;
            code_on_line = false;
            i += 1;
            continue;
        }
        match st {
            St::Normal => match c {
                b'\'' => {
                    code_on_line = true;
                    st = St::Str;
                }
                b'"' => match bytes.get(i + 1) {
                    Some(b'*') => {
                        st = St::Line { start: i };
                        i += 1; // consume the '*'
                    }
                    Some(b'"') => i += 1, // empty `""` comment
                    _ => st = St::Block,
                },
                b'/' if i > 0 && bytes[i - 1] == b'#' => {
                    code_on_line = true;
                    st = St::Regex;
                }
                b' ' | b'\t' | b'\r' => {}
                _ => code_on_line = true,
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
            St::Line { .. } => {}
            St::Block => match c {
                b'\\' => i += 1,
                b'"' => st = St::Normal,
                _ => {}
            },
        }
        i += 1;
    }
    if let St::Line { start } = st {
        close_line_comment(start, n, line, line_start, code_on_line, &mut pragmas);
    }
    pragmas
}

#[cfg(test)]
#[path = "pragmas_tests.rs"]
mod tests;
