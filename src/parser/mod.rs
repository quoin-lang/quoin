pub mod ast_visitor;
pub mod parser;

pub mod generated {
    pub mod buildingblockslexer;
    mod buildingblockslistener;
    pub mod buildingblocksparser;
    pub mod buildingblocksvisitor;
}

#[macro_export]
macro_rules! cast_node {
    ( $p:pat, $v:ident, $e:expr ) => {{
        match $e.value {
            $p => $v,
            x => panic!("MethodCall.arguments set to incorrect NodeValue {:?}", x),
        }
    }};
}
