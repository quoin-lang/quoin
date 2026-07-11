//! A small Wadler/Leijen document algebra and a width-aware renderer — the layout
//! engine the formatter lowers the AST into. `Group` renders its contents flat (on
//! one line) when they fit the width budget and broken (with newlines) when they
//! don't; `Align` pins subsequent line breaks to the current column — a general
//! primitive the engine provides for column-anchored layouts (the formatter's
//! keyword-continuation style now breaks to the statement base instead of using it).
//!
//! Phase 0 only uses `Verbatim`/`Text`/`HardLine`/`Concat`, but the full algebra is
//! implemented and unit-tested here so the later phases (which add width-driven
//! wrapping) build on a proven engine.

/// A layout document.
#[derive(Clone, Debug)]
pub enum Doc {
    /// Empty.
    Nil,
    /// Inline text with NO embedded newlines; its width counts against the budget.
    Text(String),
    /// Literal text emitted exactly as-is (may contain newlines), with no
    /// re-indentation. Used for source slices preserved verbatim.
    Verbatim(String),
    /// A space when the enclosing group is flat; a newline + indent when broken.
    Line,
    /// The text when the enclosing group is flat; NOTHING when broken. The
    /// statement separator: `;` is load-bearing on a shared line and dead
    /// weight before a line break (minimal-semicolon emission stays
    /// idempotent through a width-forced break exactly because the broken
    /// rendering never contains the `;` a re-format of it would omit).
    FlatText(String),
    /// Nothing when flat; a newline + indent when broken.
    SoftLine,
    /// Always a newline + indent; also forces every enclosing group to break.
    HardLine,
    /// Concatenation of sub-documents.
    Concat(Vec<Doc>),
    /// Increase the indent of the contained document's line breaks by `n` columns.
    Nest(isize, Box<Doc>),
    /// Set the indent of the contained document's line breaks to the current column.
    Align(Box<Doc>),
    /// Render flat if it fits the remaining width, otherwise render broken.
    Group(Box<Doc>),
    /// Render `then` if `need` columns fit at `indent + extra` (i.e. `indent + extra + need <=
    /// width`), else `els`. A render-time either/or keyed on the *indent*, not the current column —
    /// used to keep the keyword receiver break only when its pairs fit at their broken columns, and
    /// fall back to base-column otherwise. `then` and `els` must flatten identically. `need == None`
    /// (a part that can't be flat) always picks `els`.
    PreferIfFits {
        need: Option<usize>,
        extra: isize,
        then: Box<Doc>,
        els: Box<Doc>,
    },
}

impl Doc {
    pub fn text(s: impl Into<String>) -> Doc {
        Doc::Text(s.into())
    }
    pub fn verbatim(s: impl Into<String>) -> Doc {
        Doc::Verbatim(s.into())
    }
    pub fn concat(docs: Vec<Doc>) -> Doc {
        Doc::Concat(docs)
    }
    pub fn group(d: Doc) -> Doc {
        Doc::Group(Box::new(d))
    }
    pub fn nest(n: isize, d: Doc) -> Doc {
        Doc::Nest(n, Box::new(d))
    }
    pub fn align(d: Doc) -> Doc {
        Doc::Align(Box::new(d))
    }
    pub fn prefer_if_fits(need: Option<usize>, extra: isize, then: Doc, els: Doc) -> Doc {
        Doc::PreferIfFits {
            need,
            extra,
            then: Box::new(then),
            els: Box::new(els),
        }
    }
}

/// The width of `doc` rendered entirely flat, or `None` if it can't be flat (it contains a
/// `HardLine`, or a `Verbatim` with an embedded newline). Layout-invariant, so a decision keyed on
/// it is idempotent.
pub fn flat_width(doc: &Doc) -> Option<usize> {
    match doc {
        Doc::Nil | Doc::SoftLine => Some(0),
        Doc::Line => Some(1),
        Doc::FlatText(s) => Some(s.chars().count()),
        Doc::Text(s) => Some(s.chars().count()),
        Doc::Verbatim(s) => (!s.contains('\n')).then(|| s.chars().count()),
        Doc::HardLine => None,
        Doc::Concat(ds) => ds.iter().try_fold(0, |acc, d| Some(acc + flat_width(d)?)),
        Doc::Nest(_, d) | Doc::Align(d) | Doc::Group(d) => flat_width(d),
        Doc::PreferIfFits { then, .. } => flat_width(then),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Flat,
    Break,
}

/// Render `doc` targeting a maximum line width of `width` columns.
pub fn render(doc: &Doc, width: usize) -> String {
    let mut out = String::new();
    let mut col: usize = 0;
    // Work stack of (indent, mode, doc), processed LIFO.
    let mut stack: Vec<(usize, Mode, &Doc)> = vec![(0, Mode::Break, doc)];

    while let Some((indent, mode, d)) = stack.pop() {
        match d {
            Doc::Nil => {}
            Doc::Text(s) => {
                out.push_str(s);
                col += s.chars().count();
            }
            Doc::Verbatim(s) => {
                out.push_str(s);
                col = match s.rfind('\n') {
                    Some(i) => s[i + 1..].chars().count(),
                    None => col + s.chars().count(),
                };
            }
            Doc::Concat(ds) => {
                for sub in ds.iter().rev() {
                    stack.push((indent, mode, sub));
                }
            }
            Doc::Nest(n, sub) => {
                let ind = (indent as isize + n).max(0) as usize;
                stack.push((ind, mode, sub));
            }
            Doc::Align(sub) => {
                stack.push((col, mode, sub));
            }
            Doc::PreferIfFits {
                need,
                extra,
                then,
                els,
            } => {
                let fits =
                    need.is_some_and(|w| indent as isize + extra + w as isize <= width as isize);
                stack.push((indent, mode, if fits { then } else { els }));
            }
            Doc::Line => match mode {
                Mode::Flat => {
                    out.push(' ');
                    col += 1;
                }
                Mode::Break => newline(&mut out, indent, &mut col),
            },
            Doc::FlatText(s) => {
                if mode == Mode::Flat {
                    out.push_str(s);
                    col += s.chars().count();
                }
            }
            Doc::SoftLine => {
                if mode == Mode::Break {
                    newline(&mut out, indent, &mut col);
                }
            }
            Doc::HardLine => newline(&mut out, indent, &mut col),
            Doc::Group(sub) => {
                let remaining = width as isize - col as isize;
                let mode = if fits(remaining, sub, &stack) {
                    Mode::Flat
                } else {
                    Mode::Break
                };
                stack.push((indent, mode, sub));
            }
        }
    }
    out
}

/// Start a fresh line: trim trailing spaces from the line we're leaving (keeps output
/// clean and idempotent), emit the newline, then indent.
fn newline(out: &mut String, indent: usize, col: &mut usize) {
    while out.ends_with(' ') {
        out.pop();
    }
    out.push('\n');
    for _ in 0..indent {
        out.push(' ');
    }
    *col = indent;
}

/// Would `doc`, rendered flat, fit in `remaining` columns before the next line break?
/// Scans `doc` flat, then continues into `rest` (what comes after the group on the same
/// line) until a break ends the line. A `HardLine` anywhere flat makes it not fit, so a
/// group containing one always breaks.
fn fits(mut remaining: isize, doc: &Doc, rest: &[(usize, Mode, &Doc)]) -> bool {
    if remaining < 0 {
        return false;
    }
    let mut local: Vec<(Mode, &Doc)> = vec![(Mode::Flat, doc)];
    let mut ri = rest.len();
    loop {
        let (mode, d) = match local.pop() {
            Some(x) => x,
            None => {
                if ri == 0 {
                    return true;
                }
                ri -= 1;
                let (_indent, m, dd) = rest[ri];
                (m, dd)
            }
        };
        match d {
            Doc::Nil => {}
            Doc::Text(s) => {
                remaining -= s.chars().count() as isize;
                if remaining < 0 {
                    return false;
                }
            }
            Doc::Verbatim(s) => {
                if s.contains('\n') {
                    return true; // the line ends inside the verbatim block
                }
                remaining -= s.chars().count() as isize;
                if remaining < 0 {
                    return false;
                }
            }
            Doc::Concat(ds) => {
                for sub in ds.iter().rev() {
                    local.push((mode, sub));
                }
            }
            Doc::Nest(_, sub) | Doc::Align(sub) => local.push((mode, sub)),
            // Both branches flatten identically, so measure via `then`.
            Doc::PreferIfFits { then, .. } => local.push((mode, then)),
            Doc::Line => match mode {
                Mode::Flat => {
                    remaining -= 1;
                    if remaining < 0 {
                        return false;
                    }
                }
                Mode::Break => return true,
            },
            Doc::FlatText(s) => {
                if mode == Mode::Flat {
                    remaining -= s.chars().count() as isize;
                    if remaining < 0 {
                        return false;
                    }
                }
            }
            Doc::SoftLine => {
                if mode == Mode::Break {
                    return true;
                }
            }
            // A hard break inside the group's own (flat) content means the group can't be flat;
            // one reached in the trailing context just ends the line, so what came before fit.
            Doc::HardLine => return mode != Mode::Flat,
            Doc::Group(sub) => local.push((Mode::Flat, sub)),
        }
    }
}

#[cfg(test)]
#[path = "doc_tests.rs"]
mod doc_tests;
