//! `quoin-fmt` — an opinionated, zero-config formatter for Quoin source code.
//!
//! The pipeline is: parse the source with `quoin-syntax`, scan the raw text for
//! comments (which the pest grammar drops as silent trivia — see [`comments`]), lower
//! the AST into a small Wadler/Leijen document algebra ([`doc`]), and render it at a
//! fixed width. The canonical style is baked in; there is no configuration.
//!
//! Correctness is enforced by the guardrails in [`verify`]: formatting never changes a
//! program's AST and never drops a comment. Those properties are checked over the whole
//! `qnlib/` corpus in the crate's integration tests.
//!
//! This is currently at **Phase 0**: the top level is laid out canonically while each
//! statement body is preserved verbatim. Later phases lower deeper and add width-driven
//! wrapping, reusing the same engine and guardrails.

pub mod comments;
pub mod doc;
pub mod format;
pub mod verify;

pub use format::{DEFAULT_WIDTH, FormatError, format_source};

/// Format a string of Quoin source with the canonical style.
pub fn format(source: &str) -> Result<String, FormatError> {
    format_source(source, "<fmt>")
}
