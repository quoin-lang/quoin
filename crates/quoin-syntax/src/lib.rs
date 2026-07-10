//! The Quoin **syntax** layer: parser, AST, source-location info, and the
//! AST→span syntax highlighter. This crate is deliberately free of any VM /
//! runtime dependencies so it can be reused by tooling (the language server)
//! as well as by the `quoin` interpreter itself.
//!
//! The `gc` feature derives `gc_arena::Collect` on [`SourceInfo`]; the `quoin`
//! crate enables it, tooling does not.

pub mod ast;
pub mod complete;
pub mod highlight;
pub mod pest;
pub mod pragmas;
pub mod source_info;

pub use ast::*;
pub use complete::complete_source;
pub use highlight::highlight_resilient;
pub use pragmas::scan_allow_pragmas;
pub use source_info::{ParseError, SourceInfo};

// Parsing entry points. `try_parse_quoin_string_named` returns a structured
// error (for diagnostics); the other three preserve the historical panic-on-error
// behavior the VM/CLI rely on.
pub use pest::parser::{
    parse_quoin_file, parse_quoin_string, parse_quoin_string_named, try_parse_quoin_string_named,
};
