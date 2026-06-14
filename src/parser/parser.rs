#![allow(non_snake_case)]

use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::thread;

use antlr_rust::common_token_stream::CommonTokenStream;
use antlr_rust::tree::ParseTreeVisitorCompat;
use antlr_rust::InputStream;

use crate::parser::ast_visitor::{AstVisitor, Node, NodeValue};
use crate::parser::generated::buildingblockslexer::BuildingBlocksLexer;
use crate::parser::generated::buildingblocksparser::BuildingBlocksParser;

pub fn parse_building_blocks_string(code: &str) -> Node {
    let lexer = BuildingBlocksLexer::new(InputStream::new(code));
    let mut parser = BuildingBlocksParser::new(CommonTokenStream::new(lexer));

    let root = parser.program().unwrap();

    let mut visitor = AstVisitor { x: Node { value: NodeValue::Unknown } };

    let visitor_result = visitor.visit(&*root);

    // println!("PROGRAM> {:?}", visitor_result);

    visitor_result
}

pub fn parse_building_blocks_file(path: &Path) -> Node {
    let filename = path.display();

    let mut file = match File::open(&path) {
        | Err(why) => panic!("couldn't open {}: {}", filename, why),
        | Ok(file) => file,
    };

    let mut contents = String::new();
    match file.read_to_string(&mut contents) {
        | Ok(_) => {}
        | Err(why) => panic!("couldn't read {}: {}", filename, why),
    };

    let builder = thread::Builder::new()
        .name("parser".into())
        .stack_size(32 * 1024 * 1024); // 32MB of stack space

    let handler = builder
        .spawn(move || {
            let lexer = BuildingBlocksLexer::new(InputStream::new(contents.as_str()));
            let mut parser = BuildingBlocksParser::new(CommonTokenStream::new(lexer));

            let root = parser.program().unwrap();

            let mut visitor = AstVisitor { x: Node { value: NodeValue::Unknown } };

            let visitor_result = visitor.visit(&*root);

            // println!("PROGRAM> {:?}", visitor_result);

            visitor_result
        })
        .unwrap();

    handler.join().unwrap()
}
