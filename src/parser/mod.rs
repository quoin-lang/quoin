pub mod ast;
pub use ast::*;

pub mod antlr;

// Public entry points for parsing, maintaining same interface
pub use antlr::parser::parse_building_blocks_file;
pub use antlr::parser::parse_building_blocks_string;

#[macro_export]
macro_rules! cast_node {
    ( $p:pat, $v:ident, $e:expr ) => {{
        match $e.value {
            $p => $v,
            x => panic!("MethodCall.arguments set to incorrect NodeValue {:?}", x),
        }
    }};
}
