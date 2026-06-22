//! Source-location info attached to AST nodes, plus the structured parse-error
//! type. `SourceInfo` was previously defined in the VM's `value.rs`; it lives
//! here now so the parser/AST can stand alone, and the VM re-exports it.

/// Location of a node (or an error) within a source file.
///
/// `line` is 1-indexed and `column` is 0-indexed (matching the ANTLR-era
/// convention the VM was originally ported from). `start`/`end` are byte offsets
/// into the source text, with `end` exclusive.
#[cfg_attr(feature = "gc", derive(gc_arena::Collect))]
#[cfg_attr(feature = "gc", collect(require_static))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceInfo {
    pub filename: String,
    pub line: usize,
    pub column: usize,
    /// Byte offset of the first character of this node in the source text.
    pub start: usize,
    /// Byte offset one past the last character of this node (exclusive).
    pub end: usize,
    pub source_text: Option<String>,
}

/// A structured parse failure carrying enough position info to build a
/// language-server diagnostic. Derived from the underlying `pest` error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    /// Human-readable description (pest's "expected …" message).
    pub message: String,
    /// 1-indexed line of the error start.
    pub line: usize,
    /// 0-indexed column of the error start.
    pub column: usize,
    /// Byte offset of the error start.
    pub start: usize,
    /// Byte offset of the error end (`>= start`; equals `start` for a point error).
    pub end: usize,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} (line {}, column {})",
            self.message,
            self.line,
            self.column + 1
        )
    }
}

impl std::error::Error for ParseError {}
