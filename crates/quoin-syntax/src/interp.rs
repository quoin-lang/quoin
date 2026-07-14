//! Scanner for `%{…}` interpolation templates.
//!
//! Shared by the compiler (which lowers a `%'…'` literal to a `+`
//! concatenation chain at compile time) and the runtime `String#%` method
//! (the dynamic path, for interpolating a computed string), so the two can
//! never disagree on how a template splits.

/// One segment of an interpolation template: literal text, or the source of
/// an embedded `%{…}` expression (marker and braces stripped).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InterpPart {
    Lit(String),
    Expr(String),
}

/// Split `s` on `%{…}` markers. Braces nest by plain depth counting (the
/// scanner has no awareness of strings or comments inside the expression),
/// and an unterminated `%{` is literal text, not an error.
pub fn split_interpolation(s: &str) -> Vec<InterpPart> {
    let chars: Vec<char> = s.chars().collect();
    let mut parts = Vec::new();
    let mut lit = String::new();
    let mut i = 0;
    while i < chars.len() {
        if i + 1 < chars.len() && chars[i] == '%' && chars[i + 1] == '{' {
            let mut depth = 1;
            let mut j = i + 2;
            while j < chars.len() && depth > 0 {
                if chars[j] == '{' {
                    depth += 1;
                } else if chars[j] == '}' {
                    depth -= 1;
                }
                j += 1;
            }
            if depth == 0 {
                if !lit.is_empty() {
                    parts.push(InterpPart::Lit(std::mem::take(&mut lit)));
                }
                parts.push(InterpPart::Expr(chars[i + 2..j - 1].iter().collect()));
                i = j;
                continue;
            }
        }
        lit.push(chars[i]);
        i += 1;
    }
    if !lit.is_empty() {
        parts.push(InterpPart::Lit(lit));
    }
    parts
}

/// Does `s` contain at least one complete `%{…}` expression?
pub fn has_interpolation(s: &str) -> bool {
    split_interpolation(s)
        .iter()
        .any(|p| matches!(p, InterpPart::Expr(_)))
}

#[cfg(test)]
#[path = "interp_tests.rs"]
mod tests;
