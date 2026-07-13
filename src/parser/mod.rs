//! The Quoin parser & AST now live in the standalone `quoin-syntax` crate.
//! This module re-exports them so existing `crate::parser::…` paths keep
//! working unchanged across the VM.

pub use quoin_syntax::ast;
pub use quoin_syntax::ast::*;
pub use quoin_syntax::interp;

// Public parsing entry points (same interface as before, plus the fallible
// `try_parse_quoin_string_named` used by tooling such as the language server).
pub use quoin_syntax::{
    ParseError, parse_quoin_file, parse_quoin_string, parse_quoin_string_named,
    try_parse_quoin_string_named,
};

#[macro_export]
macro_rules! cast_node {
    ( $p:pat, $v:ident, $e:expr ) => {{
        match $e.value {
            $p => $v,
            x => panic!("MethodCall.arguments set to incorrect NodeValue {:?}", x),
        }
    }};
}
