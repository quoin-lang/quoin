pub mod ast;
pub use ast::*;

pub mod pest;

// Public entry points for parsing, maintaining same interface
pub use pest::parser::parse_quoin_file;
pub use pest::parser::parse_quoin_string;
pub use pest::parser::parse_quoin_string_named;

#[macro_export]
macro_rules! cast_node {
    ( $p:pat, $v:ident, $e:expr ) => {{
        match $e.value {
            $p => $v,
            x => panic!("MethodCall.arguments set to incorrect NodeValue {:?}", x),
        }
    }};
}
