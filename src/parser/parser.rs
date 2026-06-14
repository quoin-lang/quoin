#![allow(non_snake_case)]

use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::thread;

use antlr_rust::InputStream;
use antlr_rust::common_token_stream::CommonTokenStream;
use antlr_rust::tree::ParseTreeVisitorCompat;

use crate::parser::ast_visitor::{AstVisitor, Node, NodeValue};
use crate::parser::generated::buildingblockslexer::BuildingBlocksLexer;
use crate::parser::generated::buildingblocksparser::BuildingBlocksParser;

pub fn parse_building_blocks_string(code: &str) -> Node {
    let lexer = BuildingBlocksLexer::new(InputStream::new(code));
    let mut parser = BuildingBlocksParser::new(CommonTokenStream::new(lexer));

    let root = parser.program().unwrap();

    let mut visitor = AstVisitor {
        x: Node {
            value: NodeValue::Unknown,
        },
    };

    let visitor_result = visitor.visit(&*root);

    // println!("PROGRAM> {:?}", visitor_result);

    visitor_result
}

pub fn parse_building_blocks_file(path: &Path) -> Node {
    let filename = path.display();

    let mut file = match File::open(&path) {
        Err(why) => panic!("couldn't open {}: {}", filename, why),
        Ok(file) => file,
    };

    let mut contents = String::new();
    match file.read_to_string(&mut contents) {
        Ok(_) => {}
        Err(why) => panic!("couldn't read {}: {}", filename, why),
    };

    let builder = thread::Builder::new()
        .name("parser".into())
        .stack_size(32 * 1024 * 1024); // 32MB of stack space

    let handler = builder
        .spawn(move || {
            let lexer = BuildingBlocksLexer::new(InputStream::new(contents.as_str()));
            let mut parser = BuildingBlocksParser::new(CommonTokenStream::new(lexer));

            let root = parser.program().unwrap();

            let mut visitor = AstVisitor {
                x: Node {
                    value: NodeValue::Unknown,
                },
            };

            let visitor_result = visitor.visit(&*root);

            // println!("PROGRAM> {:?}", visitor_result);

            visitor_result
        })
        .unwrap();

    handler.join().unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::ast_visitor::*;
    use std::sync::Arc;

    fn val_node(val: NodeValue) -> Node {
        Node { value: val }
    }

    fn arc_node(val: NodeValue) -> Arc<Node> {
        Arc::new(val_node(val))
    }

    fn ident(name: &str, identifier_type: IdentifierType) -> Arc<Node> {
        arc_node(NodeValue::Identifier(IdentifierNode {
            namespace: None,
            name: name.to_string(),
            identifier_type,
        }))
    }

    fn integer(value: i64) -> Arc<Node> {
        arc_node(NodeValue::Integer(IntegerNode { value }))
    }

    fn double(value: f64) -> Arc<Node> {
        arc_node(NodeValue::Double(DoubleNode { value }))
    }

    fn string_node(value: &str) -> Arc<Node> {
        arc_node(NodeValue::Str(StringNode {
            value: value.to_string(),
        }))
    }

    fn symbol(value: &str) -> Arc<Node> {
        arc_node(NodeValue::Symbol(SymbolNode {
            value: value.to_string(),
        }))
    }

    fn binary(op: BinaryOperatorType, left: Arc<Node>, right: Arc<Node>) -> Arc<Node> {
        arc_node(NodeValue::BinaryOperator(BinaryOperatorNode {
            operator: op,
            left,
            right,
        }))
    }

    fn unary(op: UnaryOperatorType, right: Arc<Node>) -> Arc<Node> {
        arc_node(NodeValue::UnaryOperator(UnaryOperatorNode {
            operator: op,
            right,
        }))
    }

    #[test]
    fn test_parse_literals() {
        let ast = parse_building_blocks_string("123;");
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![integer(123)],
        }));
        assert_eq!(ast, expected);

        let ast = parse_building_blocks_string("12.34;");
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![double(12.34)],
        }));
        assert_eq!(ast, expected);

        let ast = parse_building_blocks_string("'hello';");
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![string_node("hello")],
        }));
        assert_eq!(ast, expected);

        let ast = parse_building_blocks_string("#foo;");
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![symbol("foo")],
        }));
        assert_eq!(ast, expected);

        let ast = parse_building_blocks_string("#/^[a-z]+$/;");
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![arc_node(NodeValue::Regex(RegexNode {
                value: "#/^[a-z]+$/".to_string(),
            }))],
        }));
        assert_eq!(ast, expected);
    }

    #[test]
    fn test_parse_identifiers() {
        let ast = parse_building_blocks_string("x;");
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![ident("x", IdentifierType::Local)],
        }));
        assert_eq!(ast, expected);

        let ast = parse_building_blocks_string("@x;");
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![ident("x", IdentifierType::Instance)],
        }));
        assert_eq!(ast, expected);
    }

    #[test]
    fn test_parse_assignment() {
        let ast = parse_building_blocks_string("x = 42;");
        let lval = arc_node(NodeValue::IdentLValue(IdentLValueNode {
            identifier: Arc::new(IdentifierNode {
                namespace: None,
                name: "x".to_string(),
                identifier_type: IdentifierType::Local,
            }),
        }));
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![arc_node(NodeValue::Assignment(AssignmentNode {
                lvalues: vec![lval],
                rvalue: integer(42),
            }))],
        }));
        assert_eq!(ast, expected);
    }

    #[test]
    fn test_parse_operators() {
        let ast = parse_building_blocks_string("1 + 2;");
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![binary(BinaryOperatorType::Add, integer(1), integer(2))],
        }));
        assert_eq!(ast, expected);

        let ast = parse_building_blocks_string("!x;");
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![unary(
                UnaryOperatorType::Bang,
                ident("x", IdentifierType::Local),
            )],
        }));
        assert_eq!(ast, expected);
    }

    #[test]
    fn test_parse_list_and_dict() {
        let ast = parse_building_blocks_string("#(1 2);");
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![arc_node(NodeValue::List(ListNode {
                values: vec![integer(1), integer(2)],
            }))],
        }));
        assert_eq!(ast, expected);

        let ast = parse_building_blocks_string("#{'a': 1};");
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![arc_node(NodeValue::Dictionary(DictionaryNode {
                keys: vec![string_node("a")],
                values: vec![integer(1)],
            }))],
        }));
        assert_eq!(ast, expected);
    }

    #[test]
    fn test_parse_block() {
        let ast = parse_building_blocks_string("{ 1 + 2 };");
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![arc_node(NodeValue::Block(BlockNode {
                name: None,
                arguments: vec![],
                decls: vec![],
                decl_block: None,
                statements: vec![binary(BinaryOperatorType::Add, integer(1), integer(2))],
            }))],
        }));
        assert_eq!(ast, expected);
    }

    fn block_arg(name: &str, identifier_type: IdentifierType, type_hint: Option<Arc<IdentifierNode>>) -> Arc<BlockArgNode> {
        Arc::new(BlockArgNode {
            identifier: Arc::new(IdentifierNode {
                namespace: None,
                name: name.to_string(),
                identifier_type,
            }),
            type_hint,
        })
    }

    fn block_decl(name: &str, identifier_type: IdentifierType, type_hint: Option<Arc<IdentifierNode>>) -> Arc<BlockDeclNode> {
        Arc::new(BlockDeclNode {
            identifier: Arc::new(IdentifierNode {
                namespace: None,
                name: name.to_string(),
                identifier_type,
            }),
            type_hint,
        })
    }

    fn ident_node(name: &str, identifier_type: IdentifierType) -> Arc<IdentifierNode> {
        Arc::new(IdentifierNode {
            namespace: None,
            name: name.to_string(),
            identifier_type,
        })
    }

    #[test]
    fn test_parse_method_call() {
        let ast = parse_building_blocks_string("x.negated;");
        let selector = Arc::new(MethodSelectorNode {
            identifiers: vec![Arc::new(IdentifierNode {
                namespace: None,
                name: "negated".to_string(),
                identifier_type: IdentifierType::Local,
            })],
        });
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![arc_node(NodeValue::MethodCall(MethodCallNode {
                subject: Some(ident("x", IdentifierType::Local)),
                arguments: Arc::new(MethodCallArgumentsNode {
                    signature: selector,
                    expressions: vec![],
                }),
            }))],
        }));
        assert_eq!(ast, expected);
    }

    #[test]
    fn test_parse_special_statements() {
        let ast = parse_building_blocks_string("!!!;");
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![arc_node(NodeValue::Bang3)],
        }));
        assert_eq!(ast, expected);

        let ast = parse_building_blocks_string("...;");
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![arc_node(NodeValue::Dot3)],
        }));
        assert_eq!(ast, expected);

        let ast = parse_building_blocks_string("???;");
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![arc_node(NodeValue::Huh3)],
        }));
        assert_eq!(ast, expected);

        let ast = parse_building_blocks_string("^x;");
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![arc_node(NodeValue::BlockReturn(BlockReturnNode {
                value: ident("x", IdentifierType::Local),
            }))],
        }));
        assert_eq!(ast, expected);

        let ast = parse_building_blocks_string("^>x;");
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![arc_node(NodeValue::YieldReturn(YieldReturnNode {
                value: ident("x", IdentifierType::Local),
            }))],
        }));
        assert_eq!(ast, expected);

        let ast = parse_building_blocks_string("^^x;");
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![arc_node(NodeValue::MethodReturn(MethodReturnNode {
                value: ident("x", IdentifierType::Local),
            }))],
        }));
        assert_eq!(ast, expected);
    }

    #[test]
    fn test_parse_assignment_lvalues() {
        // Splat: *rest = x;
        let ast = parse_building_blocks_string("*rest = x;");
        let lval = arc_node(NodeValue::SplatLValue(SplatLValueNode {
            identifier: Arc::new(IdentifierNode {
                namespace: None,
                name: "rest".to_string(),
                identifier_type: IdentifierType::Local,
            }),
        }));
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![arc_node(NodeValue::Assignment(AssignmentNode {
                lvalues: vec![lval],
                rvalue: ident("x", IdentifierType::Local),
            }))],
        }));
        assert_eq!(ast, expected);

        // Ignored: _ = x;
        let ast = parse_building_blocks_string("_ = x;");
        let lval = arc_node(NodeValue::IgnoredLValue);
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![arc_node(NodeValue::Assignment(AssignmentNode {
                lvalues: vec![lval],
                rvalue: ident("x", IdentifierType::Local),
            }))],
        }));
        assert_eq!(ast, expected);

        // Ignored Splat: *_ = x;
        let ast = parse_building_blocks_string("*_ = x;");
        let lval = arc_node(NodeValue::IgnoredSplatLValue);
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![arc_node(NodeValue::Assignment(AssignmentNode {
                lvalues: vec![lval],
                rvalue: ident("x", IdentifierType::Local),
            }))],
        }));
        assert_eq!(ast, expected);

        // SubLValue: (a *b) = x;
        let ast = parse_building_blocks_string("(a *b) = x;");
        let lval_a = arc_node(NodeValue::IdentLValue(IdentLValueNode {
            identifier: Arc::new(IdentifierNode {
                namespace: None,
                name: "a".to_string(),
                identifier_type: IdentifierType::Local,
            }),
        }));
        let lval_b = arc_node(NodeValue::SplatLValue(SplatLValueNode {
            identifier: Arc::new(IdentifierNode {
                namespace: None,
                name: "b".to_string(),
                identifier_type: IdentifierType::Local,
            }),
        }));
        let sub_lval = arc_node(NodeValue::SubLValue(SubLValueNode {
            lvalues: vec![lval_a, lval_b],
        }));
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![arc_node(NodeValue::Assignment(AssignmentNode {
                lvalues: vec![sub_lval],
                rvalue: ident("x", IdentifierType::Local),
            }))],
        }));
        assert_eq!(ast, expected);
    }

    #[test]
    fn test_parse_binary_operators_all() {
        let test_ops = vec![
            ("-", BinaryOperatorType::Sub),
            ("*", BinaryOperatorType::Mul),
            ("/", BinaryOperatorType::Div),
            ("&&", BinaryOperatorType::And),
            ("||", BinaryOperatorType::Or),
            ("==", BinaryOperatorType::Eq),
            ("!=", BinaryOperatorType::NotEq),
            (">", BinaryOperatorType::Gt),
            (">=", BinaryOperatorType::GtEq),
            ("<", BinaryOperatorType::Lt),
            ("<=", BinaryOperatorType::LtEq),
            ("..", BinaryOperatorType::Range),
            ("%", BinaryOperatorType::Mod),
            ("~", BinaryOperatorType::Match),
        ];
        for (op_str, op_type) in test_ops {
            let code = format!("1 {op_str} 2;");
            let ast = parse_building_blocks_string(&code);
            let expected = val_node(NodeValue::Program(ProgramNode {
                expressions: vec![binary(op_type, integer(1), integer(2))],
            }));
            assert_eq!(ast, expected);
        }
    }

    #[test]
    fn test_parse_unary_operators_all() {
        let ast = parse_building_blocks_string("+x;");
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![unary(UnaryOperatorType::Add, ident("x", IdentifierType::Local))],
        }));
        assert_eq!(ast, expected);

        let ast = parse_building_blocks_string("-x;");
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![unary(UnaryOperatorType::Sub, ident("x", IdentifierType::Local))],
        }));
        assert_eq!(ast, expected);

        let ast = parse_building_blocks_string("%x;");
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![unary(UnaryOperatorType::Mod, ident("x", IdentifierType::Local))],
        }));
        assert_eq!(ast, expected);
    }

    #[test]
    fn test_parse_classes_and_consts() {
        // Const Definition: MY_CONST <- 42;
        let ast = parse_building_blocks_string("MY_CONST <- 42;");
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![arc_node(NodeValue::ConstDefinition(ConstDefinitionNode {
                identifier: Arc::new(IdentifierNode {
                    namespace: None,
                    name: "MY_CONST".to_string(),
                    identifier_type: IdentifierType::Local,
                }),
                rvalue: integer(42),
            }))],
        }));
        assert_eq!(ast, expected);

        // Class Definition: MyClass <- { 1 };
        let ast = parse_building_blocks_string("MyClass <- { 1 };");
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![arc_node(NodeValue::ClassDefinition(ClassDefinitionNode {
                identifier: Arc::new(IdentifierNode {
                    namespace: None,
                    name: "MyClass".to_string(),
                    identifier_type: IdentifierType::Local,
                }),
                parent_identifier: None,
                block: Arc::new(BlockNode {
                    name: None,
                    arguments: vec![],
                    decls: vec![],
                    decl_block: None,
                    statements: vec![integer(1)],
                }),
            }))],
        }));
        assert_eq!(ast, expected);

        // Class Definition 2: ParentClass <- ChildClass <- { 1 };
        let ast = parse_building_blocks_string("ParentClass <- ChildClass <- { 1 };");
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![arc_node(NodeValue::ClassDefinition(ClassDefinitionNode {
                identifier: Arc::new(IdentifierNode {
                    namespace: None,
                    name: "ChildClass".to_string(),
                    identifier_type: IdentifierType::Local,
                }),
                parent_identifier: Some(Arc::new(IdentifierNode {
                    namespace: None,
                    name: "ParentClass".to_string(),
                    identifier_type: IdentifierType::Local,
                })),
                block: Arc::new(BlockNode {
                    name: None,
                    arguments: vec![],
                    decls: vec![],
                    decl_block: None,
                    statements: vec![integer(1)],
                }),
            }))],
        }));
        assert_eq!(ast, expected);

        // Class Extension: MyClass <-- { 1 };
        let ast = parse_building_blocks_string("MyClass <-- { 1 };");
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![arc_node(NodeValue::ClassExtension(ClassExtensionNode {
                expression: ident("MyClass", IdentifierType::Local),
                block: Arc::new(BlockNode {
                    name: None,
                    arguments: vec![],
                    decls: vec![],
                    decl_block: None,
                    statements: vec![integer(1)],
                }),
            }))],
        }));
        assert_eq!(ast, expected);
    }

    #[test]
    fn test_parse_method_definitions() {
        // SelectorNoArgs
        let ast = parse_building_blocks_string("foo -> { 1 };");
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![arc_node(NodeValue::MethodDefinition(MethodDefinitionNode {
                signature: Arc::new(MethodSelectorNode {
                    identifiers: vec![Arc::new(IdentifierNode {
                        namespace: None,
                        name: "foo".to_string(),
                        identifier_type: IdentifierType::Local,
                    })],
                }),
                block: Arc::new(BlockNode {
                    name: None,
                    arguments: vec![],
                    decls: vec![],
                    decl_block: None,
                    statements: vec![integer(1)],
                }),
            }))],
        }));
        assert_eq!(ast, expected);

        // SelectorNoArgsBang
        let ast = parse_building_blocks_string("foo! -> { 1 };");
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![arc_node(NodeValue::MethodDefinition(MethodDefinitionNode {
                signature: Arc::new(MethodSelectorNode {
                    identifiers: vec![Arc::new(IdentifierNode {
                        namespace: None,
                        name: "foo!".to_string(),
                        identifier_type: IdentifierType::Local,
                    })],
                }),
                block: Arc::new(BlockNode {
                    name: None,
                    arguments: vec![],
                    decls: vec![],
                    decl_block: None,
                    statements: vec![integer(1)],
                }),
            }))],
        }));
        assert_eq!(ast, expected);

        // SelectorWArgs
        let ast = parse_building_blocks_string("foo: bar: -> { 1 };");
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![arc_node(NodeValue::MethodDefinition(MethodDefinitionNode {
                signature: Arc::new(MethodSelectorNode {
                    identifiers: vec![
                        Arc::new(IdentifierNode {
                            namespace: None,
                            name: "foo:".to_string(),
                            identifier_type: IdentifierType::Local,
                        }),
                        Arc::new(IdentifierNode {
                            namespace: None,
                            name: "bar:".to_string(),
                            identifier_type: IdentifierType::Local,
                        }),
                    ],
                }),
                block: Arc::new(BlockNode {
                    name: None,
                    arguments: vec![],
                    decls: vec![],
                    decl_block: None,
                    statements: vec![integer(1)],
                }),
            }))],
        }));
        assert_eq!(ast, expected);

        // SelectorSymbol
        let ast = parse_building_blocks_string("#foo -> { 1 };");
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![arc_node(NodeValue::MethodDefinition(MethodDefinitionNode {
                signature: Arc::new(MethodSelectorNode {
                    identifiers: vec![Arc::new(IdentifierNode {
                        namespace: None,
                        name: "foo".to_string(),
                        identifier_type: IdentifierType::Local,
                    })],
                }),
                block: Arc::new(BlockNode {
                    name: None,
                    arguments: vec![],
                    decls: vec![],
                    decl_block: None,
                    statements: vec![integer(1)],
                }),
            }))],
        }));
        assert_eq!(ast, expected);

        // MethodExtension
        let ast = parse_building_blocks_string("foo --> { 1 };");
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![arc_node(NodeValue::MethodExtension(MethodExtensionNode {
                signature: Arc::new(MethodSelectorNode {
                    identifiers: vec![Arc::new(IdentifierNode {
                        namespace: None,
                        name: "foo".to_string(),
                        identifier_type: IdentifierType::Local,
                    })],
                }),
                block: Arc::new(BlockNode {
                    name: None,
                    arguments: vec![],
                    decls: vec![],
                    decl_block: None,
                    statements: vec![integer(1)],
                }),
            }))],
        }));
        assert_eq!(ast, expected);

        // SelectorNoArgs with keyword
        let ast = parse_building_blocks_string("nil -> { 1 };");
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![arc_node(NodeValue::MethodDefinition(MethodDefinitionNode {
                signature: Arc::new(MethodSelectorNode {
                    identifiers: vec![Arc::new(IdentifierNode {
                        namespace: None,
                        name: "nil".to_string(),
                        identifier_type: IdentifierType::Keyword,
                    })],
                }),
                block: Arc::new(BlockNode {
                    name: None,
                    arguments: vec![],
                    decls: vec![],
                    decl_block: None,
                    statements: vec![integer(1)],
                }),
            }))],
        }));
        assert_eq!(ast, expected);
    }

    #[test]
    fn test_parse_method_calls() {
        // Implicit subject (DefCall): .foo;
        let ast = parse_building_blocks_string(".foo;");
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![arc_node(NodeValue::MethodCall(MethodCallNode {
                subject: None,
                arguments: Arc::new(MethodCallArgumentsNode {
                    signature: Arc::new(MethodSelectorNode {
                        identifiers: vec![Arc::new(IdentifierNode {
                            namespace: None,
                            name: "foo".to_string(),
                            identifier_type: IdentifierType::Local,
                        })],
                    }),
                    expressions: vec![],
                }),
            }))],
        }));
        assert_eq!(ast, expected);

        // Implicit subject with bang: .foo!;
        let ast = parse_building_blocks_string(".foo!;");
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![arc_node(NodeValue::MethodCall(MethodCallNode {
                subject: None,
                arguments: Arc::new(MethodCallArgumentsNode {
                    signature: Arc::new(MethodSelectorNode {
                        identifiers: vec![Arc::new(IdentifierNode {
                            namespace: None,
                            name: "foo!".to_string(),
                            identifier_type: IdentifierType::Local,
                        })],
                    }),
                    expressions: vec![],
                }),
            }))],
        }));
        assert_eq!(ast, expected);

        // Call with bang: x.foo!;
        let ast = parse_building_blocks_string("x.foo!;");
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![arc_node(NodeValue::MethodCall(MethodCallNode {
                subject: Some(ident("x", IdentifierType::Local)),
                arguments: Arc::new(MethodCallArgumentsNode {
                    signature: Arc::new(MethodSelectorNode {
                        identifiers: vec![Arc::new(IdentifierNode {
                            namespace: None,
                            name: "foo!".to_string(),
                            identifier_type: IdentifierType::Local,
                        })],
                    }),
                    expressions: vec![],
                }),
            }))],
        }));
        assert_eq!(ast, expected);

        // Call with multiple args: x.foo: 1 bar: 2;
        let ast = parse_building_blocks_string("x.foo: 1 bar: 2;");
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![arc_node(NodeValue::MethodCall(MethodCallNode {
                subject: Some(ident("x", IdentifierType::Local)),
                arguments: Arc::new(MethodCallArgumentsNode {
                    signature: Arc::new(MethodSelectorNode {
                        identifiers: vec![
                            Arc::new(IdentifierNode {
                                namespace: None,
                                name: "foo".to_string(),
                                identifier_type: IdentifierType::Local,
                            }),
                            Arc::new(IdentifierNode {
                                namespace: None,
                                name: "bar".to_string(),
                                identifier_type: IdentifierType::Local,
                            }),
                        ],
                    }),
                    expressions: vec![integer(1), integer(2)],
                }),
            }))],
        }));
        assert_eq!(ast, expected);
    }

    #[test]
    fn test_parse_namespaces_and_keywords() {
        // Namespaced Ident: [foo/bar]baz;
        let ast = parse_building_blocks_string("[foo/bar]baz;");
        let ns = Arc::new(NamespaceNode {
            identifiers: vec![
                Arc::new(IdentifierNode {
                    namespace: None,
                    name: "foo".to_string(),
                    identifier_type: IdentifierType::Local,
                }),
                Arc::new(IdentifierNode {
                    namespace: None,
                    name: "bar".to_string(),
                    identifier_type: IdentifierType::Local,
                }),
            ],
        });
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![arc_node(NodeValue::Identifier(IdentifierNode {
                namespace: Some(ns),
                name: "baz".to_string(),
                identifier_type: IdentifierType::Namespaced,
            }))],
        }));
        assert_eq!(ast, expected);

        // Root namespace: [/]baz;
        let ast = parse_building_blocks_string("[/]baz;");
        let ns = Arc::new(NamespaceNode {
            identifiers: vec![],
        });
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![arc_node(NodeValue::Identifier(IdentifierNode {
                namespace: Some(ns),
                name: "baz".to_string(),
                identifier_type: IdentifierType::Namespaced,
            }))],
        }));
        assert_eq!(ast, expected);

        // Keywords as identifiers: nil; true; false;
        let ast = parse_building_blocks_string("nil;");
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![arc_node(NodeValue::Identifier(IdentifierNode {
                namespace: None,
                name: "nil".to_string(),
                identifier_type: IdentifierType::Local,
            }))],
        }));
        assert_eq!(ast, expected);
    }

    #[test]
    fn test_parse_sets_user_strings_and_user_lists() {
        // Set: #<1 2>;
        let ast = parse_building_blocks_string("#<1 2>;");
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![arc_node(NodeValue::Set(SetNode {
                values: vec![integer(1), integer(2)],
            }))],
        }));
        assert_eq!(ast, expected);

        // User string: #MyStr'hello';
        let ast = parse_building_blocks_string("#MyStr'hello';");
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![arc_node(NodeValue::UserString(UserStringNode {
                identifier: Arc::new(IdentifierNode {
                    namespace: None,
                    name: "MyStr".to_string(),
                    identifier_type: IdentifierType::Local,
                }),
                value: "hello".to_string(),
            }))],
        }));
        assert_eq!(ast, expected);

        // User list: #MyList(1 2);
        let ast = parse_building_blocks_string("#MyList(1 2);");
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![arc_node(NodeValue::UserList(UserListNode {
                identifier: Arc::new(IdentifierNode {
                    namespace: None,
                    name: "MyList".to_string(),
                    identifier_type: IdentifierType::Local,
                }),
                values: vec![integer(1), integer(2)],
            }))],
        }));
        assert_eq!(ast, expected);
    }

    #[test]
    fn test_parse_advanced_blocks() {
        // Named block: { #my_block |x| 1; }
        let ast = parse_building_blocks_string("{ #my_block |x| 1; };");
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![arc_node(NodeValue::Block(BlockNode {
                name: Some(Arc::new(SymbolNode {
                    value: "my_block".to_string(),
                })),
                arguments: vec![block_arg("x", IdentifierType::Local, None)],
                decls: vec![],
                decl_block: None,
                statements: vec![integer(1)],
            }))],
        }));
        assert_eq!(ast, expected);

        // Typed block arg: { |x:Int| 1; }
        let ast = parse_building_blocks_string("{ |x:Int| 1; };");
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![arc_node(NodeValue::Block(BlockNode {
                name: None,
                arguments: vec![block_arg("x", IdentifierType::Local, Some(ident_node("Int", IdentifierType::Local)))],
                decls: vec![],
                decl_block: None,
                statements: vec![integer(1)],
            }))],
        }));
        assert_eq!(ast, expected);

        // Ignored block arg: { |_| 1; }
        // Visitor maps Ignored to name "?"
        let ast = parse_building_blocks_string("{ |_| 1; };");
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![arc_node(NodeValue::Block(BlockNode {
                name: None,
                arguments: vec![block_arg("?", IdentifierType::Local, None)],
                decls: vec![],
                decl_block: None,
                statements: vec![integer(1)],
            }))],
        }));
        assert_eq!(ast, expected);

        // Instance variable arg: { |@x| 1; }
        let ast = parse_building_blocks_string("{ |@x| 1; };");
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![arc_node(NodeValue::Block(BlockNode {
                name: None,
                arguments: vec![block_arg("x", IdentifierType::Instance, None)],
                decls: vec![],
                decl_block: None,
                statements: vec![integer(1)],
            }))],
        }));
        assert_eq!(ast, expected);

        // Untyped block decl: { | - x| 1; }
        let ast = parse_building_blocks_string("{ | - x| 1; };");
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![arc_node(NodeValue::Block(BlockNode {
                name: None,
                arguments: vec![],
                decls: vec![block_decl("x", IdentifierType::Local, None)],
                decl_block: None,
                statements: vec![integer(1)],
            }))],
        }));
        assert_eq!(ast, expected);

        // Typed block decl: { | - x:Int| 1; }
        let ast = parse_building_blocks_string("{ | - x:Int| 1; };");
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![arc_node(NodeValue::Block(BlockNode {
                name: None,
                arguments: vec![],
                decls: vec![block_decl("x", IdentifierType::Local, Some(ident_node("Int", IdentifierType::Local)))],
                decl_block: None,
                statements: vec![integer(1)],
            }))],
        }));
        assert_eq!(ast, expected);

        // Decl block: { |x { 2 } - y| 1; }
        let ast = parse_building_blocks_string("{ |x { 2 } - y| 1; };");
        let expected = val_node(NodeValue::Program(ProgramNode {
            expressions: vec![arc_node(NodeValue::Block(BlockNode {
                name: None,
                arguments: vec![block_arg("x", IdentifierType::Local, None)],
                decls: vec![block_decl("y", IdentifierType::Local, None)],
                decl_block: Some(Arc::new(BlockNode {
                    name: None,
                    arguments: vec![],
                    decls: vec![],
                    decl_block: None,
                    statements: vec![integer(2)],
                })),
                statements: vec![integer(1)],
            }))],
        }));
        assert_eq!(ast, expected);
    }
}
