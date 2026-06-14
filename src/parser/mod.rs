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
            | $p => $v,
            | x => panic!("MethodCall.arguments set to incorrect NodeValue {:?}", x),
        }
    }};
}

#[macro_export]
macro_rules! cast_nodes {
    ( $p:pat, $v:ident, $it:expr ) => {
        $it.map(|n| cast_node!($p, $v, n).clone()).collect()
    };
}

#[macro_export]
macro_rules! panic_unexpected_sub_node {
    ( $e:expr ) => {
        panic!("Found unexpected lone {:?} node", $e)
    };
}
