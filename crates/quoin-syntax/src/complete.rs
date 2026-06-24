//! Heuristic completion of syntactically *incomplete* source: append a minimal suffix that
//! makes a partial input parse, so tooling (the REPL, the language server) can highlight or
//! analyze input that is still being typed. The suffix is **append-only** (the original
//! tokens keep their byte offsets) and every candidate is **verified by re-parsing**, so a
//! wrong guess degrades to `None` rather than producing incorrect output.
//!
//! Because the completion exists only to make the input parse — callers crop it away (see
//! [`crate::highlight::highlight_resilient`]) — *which* candidate succeeds does not affect
//! the result for the original span, only that one does. So we try a small fixed set rather
//! than classifying the trailing token precisely.

use crate::ast::Node;
use crate::try_parse_quoin_string_named;

/// Parse `source`, returning the AST or `None` — **never panicking**. `try_parse` only
/// guards the pest step; the AST builder still has `unreachable!`s on some pest-valid shapes
/// (e.g. `Foo <-- 0` — the tracked `Runtime.eval:` parse-panic bug), so tooling that feeds
/// arbitrary/partial input (completion, live highlighting) catches the unwind here rather
/// than aborting. A caught panic prints its message but is otherwise treated as "no parse".
pub(crate) fn parse_or_none(source: &str) -> Option<Node> {
    let src = source.to_string();
    std::panic::catch_unwind(move || try_parse_quoin_string_named(&src, "<resilient>"))
        .ok()
        .and_then(Result::ok)
}

/// Given `source` that fails to parse, return a minimal **suffix** such that `source +
/// suffix` parses, or `None` if no candidate works. The suffix closes any open
/// string/regex/brackets and, for input ending mid-expression (a trailing binary operator,
/// keyword selector, `=`, or a definition operator), supplies a placeholder operand.
pub fn complete_source(source: &str) -> Option<String> {
    let close = closing_delimiters(source);
    // In order: close open delimiters only (a complete expression with open brackets), then a
    // block placeholder (`{}` — valid as the right side of *any* operator incl. definition
    // ops `<-`/`<--`/`->`, and AST-safe), then a primitive placeholder (`0`) as a last resort.
    // The first that parses wins; the placeholder is cropped away by callers, so the choice
    // only needs to make *some* completion parse — `{}` before `0` also avoids the
    // `Foo <-- 0` AST-build panic.
    for operand in ["", " {}", " 0"] {
        let suffix = format!("{operand}{close}");
        if suffix.is_empty() {
            continue; // `source` already failed to parse; an empty suffix can't help.
        }
        if parse_or_none(&format!("{source}{suffix}")).is_some() {
            return Some(suffix);
        }
    }
    None
}

/// Scan `source` and return the closing string that balances its open delimiters and any
/// open string / regex / block comment — innermost context first, then brackets in LIFO
/// order. `""` when nothing is open. A heuristic mini-lexer (the grammar owns the real one);
/// `complete_source`'s re-parse verification keeps a miscount from ever mis-highlighting.
fn closing_delimiters(source: &str) -> String {
    enum St {
        Normal,
        Str,          // '…'  (also covers #ident'…' user strings, %'…', #'…' symbols)
        Regex,        // #/…/
        LineComment,  // "* … to end of line
        BlockComment, // " … "
    }
    let mut st = St::Normal;
    let mut stack: Vec<char> = Vec::new();
    let chars: Vec<char> = source.chars().collect();
    let mut prev = '\0';
    let mut k = 0;
    while k < chars.len() {
        let c = chars[k];
        match st {
            St::Normal => match c {
                '\'' => st = St::Str,
                '"' => match chars.get(k + 1) {
                    Some('*') => {
                        st = St::LineComment;
                        k += 1;
                    }
                    Some('"') => k += 1, // empty `""` comment
                    _ => st = St::BlockComment,
                },
                '(' => stack.push(')'),
                '{' => stack.push('}'),
                '[' => stack.push(']'),
                // `<` and `/` are operators on their own; only `#<` (set) / `#/` (regex)
                // open a context, so they require the `#` immediately before.
                '<' if prev == '#' => stack.push('>'),
                '/' if prev == '#' => st = St::Regex,
                ')' | '}' | ']' if stack.last() == Some(&c) => {
                    stack.pop();
                }
                '>' if stack.last() == Some(&'>') => {
                    stack.pop();
                }
                _ => {}
            },
            St::Str => match c {
                '\\' => k += 1, // skip the escaped char
                '\'' => st = St::Normal,
                _ => {}
            },
            St::Regex => match c {
                '\\' => k += 1,
                '/' => st = St::Normal,
                _ => {}
            },
            St::LineComment => {
                if c == '\n' {
                    st = St::Normal;
                }
            }
            St::BlockComment => match c {
                '\\' => k += 1,
                '"' => st = St::Normal,
                _ => {}
            },
        }
        prev = c;
        k += 1;
    }

    let mut out = String::new();
    match st {
        St::Str => out.push('\''),
        St::Regex => out.push('/'),
        St::BlockComment => out.push('"'),
        _ => {}
    }
    out.extend(stack.iter().rev());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `source` completes iff some suffix makes `source + suffix` parse.
    fn completes(source: &str) -> bool {
        match complete_source(source) {
            Some(suffix) => parse_or_none(&format!("{source}{suffix}")).is_some(),
            None => false,
        }
    }

    #[test]
    fn trailing_binary_operators() {
        for src in [
            "1 +", "a * ", "x <=", "p && ", "r ..", "m ~", "n -", "a / ", "b > ", "c == ",
        ] {
            assert!(completes(src), "should complete: {src:?}");
        }
    }

    #[test]
    fn keyword_selector_and_assignment() {
        for src in ["a.foo:", "list.at: 0 put:", "x ="] {
            assert!(completes(src), "should complete: {src:?}");
        }
    }

    #[test]
    fn definition_operators_take_a_block() {
        for src in ["Foo <-", "Box <--", "bar ->", "baz -->"] {
            assert!(completes(src), "should complete: {src:?}");
        }
    }

    #[test]
    fn open_delimiters_and_strings() {
        for src in [
            "#(1 2", "Foo <- {", "( a + b", "#{ 'a':", "#< 1 2", "'hello", "#/ab",
        ] {
            assert!(completes(src), "should complete: {src:?}");
        }
    }

    #[test]
    fn nested_incomplete() {
        for src in ["#{ 'a': #(1 2", "Foo <- { bar -> { 1 +", "#( 'x'"] {
            assert!(completes(src), "should complete: {src:?}");
        }
    }

    #[test]
    fn brackets_inside_strings_do_not_count() {
        // The `{`/`(` are inside a (closed) string, so nothing is open — no closers added.
        assert_eq!(closing_delimiters("'{ ('"), "");
        // Open bracket *outside* a string is counted.
        assert_eq!(closing_delimiters("#('a' 'b'"), ")");
    }

    #[test]
    fn hopeless_input_returns_none() {
        for src in [")", "} ]", "* *"] {
            assert_eq!(complete_source(src), None, "should not complete: {src:?}");
        }
    }
}
