use crate::ast::NodeValue::*;
use crate::ast::*;
use crate::source_info::{ParseError, SourceInfo};

use once_cell::sync::Lazy;
use pest::Parser;
use pest::iterators::Pair;
use pest::pratt_parser::PrattParser;
use pest_derive::Parser;
use regex::Captures;
use std::cell::RefCell;
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;
use substring::Substring;

#[derive(Parser)]
#[grammar = "pest/Quoin.pest"]
pub struct QuoinParser;

static PRATT_PARSER: Lazy<PrattParser<Rule>> = Lazy::new(|| {
    use Rule::*;
    use pest::pratt_parser::{Assoc, Op};

    PrattParser::new()
        .op(Op::infix(op_or, Assoc::Left))
        .op(Op::infix(op_and, Assoc::Left))
        .op(Op::infix(op_eq, Assoc::Left) | Op::infix(op_ne, Assoc::Left))
        .op(Op::infix(op_lt, Assoc::Left)
            | Op::infix(op_le, Assoc::Left)
            | Op::infix(op_gt, Assoc::Left)
            | Op::infix(op_ge, Assoc::Left))
        .op(Op::infix(op_match, Assoc::Left))
        // Precedence increases downward (pest: first `.op` binds loosest). Range
        // is looser than arithmetic so `2..n+1` means `2..(n+1)`; multiplicative
        // binds tighter than additive; postfix `.method` binds tighter than any
        // infix operator so `a.x * b.y` is `(a.x) * (b.y)`.
        .op(Op::infix(op_range, Assoc::Left))
        .op(Op::infix(op_add, Assoc::Left) | Op::infix(op_sub, Assoc::Left))
        .op(Op::infix(op_mul, Assoc::Left)
            | Op::infix(op_div, Assoc::Left)
            | Op::infix(op_mod, Assoc::Left))
        .op(Op::postfix(postfix_op))
        .op(Op::infix(op_class_ext, Assoc::Left))
        .op(Op::prefix(prefix_op))
});

struct LineOffsetTable {
    line_starts: Vec<usize>,
}

impl LineOffsetTable {
    fn new(text: &str) -> Self {
        let mut line_starts = vec![0];
        for (i, c) in text.char_indices() {
            if c == '\n' {
                line_starts.push(i + 1);
            }
        }
        Self { line_starts }
    }

    fn find_line_col(&self, byte_offset: usize, text: &str) -> (usize, usize) {
        let idx = self.line_starts.partition_point(|&pos| pos <= byte_offset);
        let line_idx = idx.saturating_sub(1);
        let line_start_pos = self.line_starts[line_idx];
        let line = line_idx + 1;
        let col = text
            .get(line_start_pos..byte_offset)
            .map(|s| s.chars().count())
            .unwrap_or(0)
            + 1;
        (line, col)
    }
}

thread_local! {
    static LINE_OFFSET_TABLE: RefCell<Option<LineOffsetTable>> = const { RefCell::new(None) };
}

pub fn parse_quoin_string(code: &str) -> Node {
    parse_quoin_string_named(code, "<string>")
}

/// Like `parse_quoin_string`, but with an explicit display name for source-info /
/// error messages (e.g. a `use`d unit's `pkg:path.qn`).
pub fn parse_quoin_string_named(code: &str, filename: &str) -> Node {
    match try_parse_quoin_string_named(code, filename) {
        Ok(node) => node,
        Err(e) => panic!("Pest parsing error in {}: {}", filename, e),
    }
}

/// Fallible variant of [`parse_quoin_string_named`]: returns a structured
/// [`ParseError`] instead of panicking, for callers (e.g. the language server)
/// that need to report diagnostics rather than abort on a syntax error.
pub fn try_parse_quoin_string_named(code: &str, filename: &str) -> Result<Node, ParseError> {
    let code = code.strip_prefix('\u{FEFF}').unwrap_or(code);

    let table = LineOffsetTable::new(code);
    LINE_OFFSET_TABLE.with(|cell| {
        *cell.borrow_mut() = Some(table);
    });

    let result = QuoinParser::parse(Rule::program, code)
        .map_err(parse_error_from_pest)
        .and_then(|mut pairs| {
            let program_pair = pairs.next().unwrap();
            // AST-building steps that run AFTER a successful pest parse
            // (int -> i64, `\u` unescape, …) used to panic! and abort the
            // process even through `Runtime.eval:` (BUGS.md Findings 4/7).
            // Convert ANY builder panic into a structured, catchable
            // ParseError here — the targeted sites carry good messages.
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                parse_program(program_pair, filename, code)
            }))
            .map_err(|payload| {
                let msg = payload
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_else(|| "internal parse failure".to_string());
                ParseError {
                    message: msg,
                    line: 0,
                    column: 0,
                    start: 0,
                    end: 0,
                }
            })
        });

    LINE_OFFSET_TABLE.with(|cell| *cell.borrow_mut() = None);
    result
}

/// Translate a `pest` parse error into a [`ParseError`] with byte-offset and
/// line/column position for diagnostics.
fn parse_error_from_pest(e: pest::error::Error<Rule>) -> ParseError {
    use pest::error::{InputLocation, LineColLocation};

    let (start, end) = match e.location {
        InputLocation::Pos(p) => (p, p),
        InputLocation::Span((s, end)) => (s, end),
    };
    let (line, col) = match e.line_col {
        LineColLocation::Pos((l, c)) => (l, c),
        LineColLocation::Span((l, c), _) => (l, c),
    };

    ParseError {
        message: e.variant.message().to_string(),
        line,
        column: col.saturating_sub(1), // 0-indexed to match `SourceInfo`
        start,
        end,
    }
}

pub fn parse_quoin_file(path: &PathBuf) -> Node {
    let filename = path.display().to_string();

    let mut file = match File::open(path) {
        Err(why) => {
            // CLI hygiene (BUGS.md secondary): a missing file is a clean
            // one-line report + exit 1, not a Rust panic with a backtrace.
            eprintln!("qn: cannot open {}: {}", filename, why);
            std::process::exit(1);
        }
        Ok(file) => file,
    };

    let mut contents = String::new();
    match file.read_to_string(&mut contents) {
        Ok(_) => {}
        Err(why) => {
            eprintln!("qn: cannot read {}: {}", filename, why);
            std::process::exit(1);
        }
    };
    let contents = contents
        .strip_prefix('\u{FEFF}')
        .unwrap_or(&contents)
        .to_string();

    match try_parse_quoin_string_named(&contents, &filename) {
        Ok(node) => node,
        Err(e) => {
            eprintln!("qn: parse error in {}: {}", filename, e);
            std::process::exit(1);
        }
    }
}

fn extract_source_info(span: pest::Span, filename: &str, source_text: &str) -> Option<SourceInfo> {
    let (line, col) = LINE_OFFSET_TABLE.with(|cell| {
        if let Some(ref table) = *cell.borrow() {
            table.find_line_col(span.start(), source_text)
        } else {
            span.start_pos().line_col()
        }
    });
    let text = source_text
        .get(span.start()..span.end())
        .map(|x| x.to_string());
    Some(SourceInfo {
        filename: filename.to_string(),
        line,
        column: col - 1, // 0-indexed to match ANTLR
        start: span.start(),
        end: span.end(),
        source_text: text,
    })
}

fn combine_source_info(
    first: &Option<SourceInfo>,
    second: &Option<SourceInfo>,
    source_text: &str,
) -> Option<SourceInfo> {
    match (first, second) {
        (Some(f), Some(s)) => {
            let combined_text = source_text.get(f.start..s.end).map(|x| x.to_string());
            Some(SourceInfo {
                filename: f.filename.clone(),
                line: f.line,
                column: f.column,
                start: f.start,
                end: s.end,
                source_text: combined_text,
            })
        }
        (Some(f), None) => Some(f.clone()),
        (None, Some(s)) => Some(s.clone()),
        _ => None,
    }
}

fn parse_program(pair: Pair<Rule>, filename: &str, source_text: &str) -> Node {
    let source_info = extract_source_info(pair.as_span(), filename, source_text);
    let mut stmts = Vec::new();
    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::stmt => {
                stmts.push(Arc::new(parse_stmt(inner, filename, source_text)));
            }
            // The shebang line is the OS's, not ours — no AST node, and because it
            // parses (rather than being stripped pre-parse) every later span keeps
            // its true line/column.
            Rule::shebang => {}
            Rule::EOI => {}
            _ => unreachable!("Unexpected rule in program: {:?}", inner.as_rule()),
        }
    }
    Node {
        source_info: source_info.clone(),
        value: Program(ProgramNode {
            expressions: stmts,
            source_info,
            // Comments are pest trivia; the checker's `"* allow:` suppression
            // pragmas are recovered from raw source here, the one place every
            // parse (file, string, REPL, eval) funnels through.
            allow_pragmas: crate::pragmas::scan_allow_pragmas(source_text, filename),
        }),
    }
}

fn parse_stmt(pair: Pair<Rule>, filename: &str, source_text: &str) -> Node {
    let inner = pair.into_inner().next().unwrap();
    let source_info = extract_source_info(inner.as_span(), filename, source_text);
    match inner.as_rule() {
        Rule::method_return => {
            let expr = inner.into_inner().next().unwrap();
            Node {
                source_info,
                value: MethodReturn(MethodReturnNode {
                    value: Arc::new(parse_expr(expr, filename, source_text)),
                }),
            }
        }
        Rule::block_return => {
            let expr = inner.into_inner().next().unwrap();
            Node {
                source_info,
                value: BlockReturn(BlockReturnNode {
                    value: Arc::new(parse_expr(expr, filename, source_text)),
                }),
            }
        }
        Rule::assignment => parse_assignment(inner, filename, source_text),
        Rule::decl => parse_decl(inner, filename, source_text),
        Rule::use_stmt => {
            // use_stmt → use_kw use_target; the atomic `use_kw` token is skipped.
            // `.qn` and the glob suffix are stripped into structured fields here. The glob is
            // either a trailing `/*` on a directory path or a bare `*` (the whole package root,
            // which is the empty path).
            let target = inner
                .into_inner()
                .find(|p| p.as_rule() == Rule::use_target)
                .unwrap();
            let mut package = None;
            let mut path_str = "";
            for p in target.into_inner() {
                match p.as_rule() {
                    Rule::use_pkg => package = Some(p.as_str().to_string()),
                    Rule::use_path => path_str = p.as_str(),
                    _ => {}
                }
            }
            let glob = path_str == "*" || path_str.ends_with("/*");
            let path = if path_str == "*" {
                String::new()
            } else {
                path_str.strip_suffix("/*").unwrap_or(path_str).to_string()
            };
            Node {
                source_info,
                value: Use(UseNode {
                    package,
                    path,
                    glob,
                }),
            }
        }
        Rule::bang3 => Node {
            source_info,
            value: Bang3,
        },
        Rule::dot3 => Node {
            source_info,
            value: Dot3,
        },
        Rule::huh3 => Node {
            source_info,
            value: Huh3,
        },
        Rule::expr => parse_expr(inner, filename, source_text),
        _ => unreachable!(),
    }
}

fn parse_assignment(pair: Pair<Rule>, filename: &str, source_text: &str) -> Node {
    let source_info = extract_source_info(pair.as_span(), filename, source_text);
    let mut inner_pairs: Vec<_> = pair.into_inner().collect();
    let rvalue = inner_pairs.pop().unwrap();
    let lvalues: Vec<_> = inner_pairs
        .into_iter()
        .map(|lv| Arc::new(parse_lvalue(lv, filename, source_text)))
        .collect();
    Node {
        source_info,
        value: Assignment(AssignmentNode {
            lvalues,
            rvalue: Arc::new(parse_expr(rvalue, filename, source_text)),
        }),
    }
}

fn parse_decl(pair: Pair<Rule>, filename: &str, source_text: &str) -> Node {
    let source_info = extract_source_info(pair.as_span(), filename, source_text);
    let mut inner = pair.into_inner();
    let kw = inner.next().unwrap();
    let kind = match kw.as_rule() {
        Rule::var_kw => DeclKind::Var,
        Rule::let_kw => DeclKind::Let,
        _ => unreachable!("decl must start with var_kw or let_kw"),
    };
    let body = inner.next().unwrap(); // Rule::decl_body
    let rvalue = inner.next().unwrap(); // expr

    let mut type_hint = None;
    let mut body_inner = body.into_inner();
    let first = body_inner.next().unwrap();
    let lvalues: Vec<Arc<Node>> = match first.as_rule() {
        Rule::typed_decl_target => {
            // ident_lvalue ~ ":" ~ type_ref  (single typed target)
            let mut ti = first.into_inner();
            let ident_lv = ti.next().unwrap(); // Rule::ident_lvalue
            let ty = ti.next().unwrap(); // Rule::type_ref
            type_hint = Some(Arc::new(parse_type_ref(ty, filename, source_text)));
            let lv_si = extract_source_info(ident_lv.as_span(), filename, source_text);
            let nsvar = ident_lv.into_inner().next().unwrap();
            vec![Arc::new(Node {
                source_info: lv_si,
                value: IdentLValue(IdentLValueNode {
                    identifier: Arc::new(parse_nsvarident(nsvar, filename, source_text)),
                }),
            })]
        }
        _ => {
            // lvalue+  — `first` is the first Rule::lvalue
            let mut lvs = vec![Arc::new(parse_lvalue(first, filename, source_text))];
            for lv in body_inner {
                lvs.push(Arc::new(parse_lvalue(lv, filename, source_text)));
            }
            lvs
        }
    };

    Node {
        source_info,
        value: Declaration(DeclarationNode {
            kind,
            lvalues,
            type_hint,
            rvalue: Arc::new(parse_expr(rvalue, filename, source_text)),
        }),
    }
}

fn parse_lvalue(pair: Pair<Rule>, filename: &str, source_text: &str) -> Node {
    let inner = pair.into_inner().next().unwrap();
    let source_info = extract_source_info(inner.as_span(), filename, source_text);
    match inner.as_rule() {
        Rule::splat_ignored_lvalue => Node {
            source_info,
            value: IgnoredSplatLValue,
        },
        Rule::splat_lvalue => {
            let nsvar = inner.into_inner().next().unwrap();
            Node {
                source_info,
                value: SplatLValue(SplatLValueNode {
                    identifier: Arc::new(parse_nsvarident(nsvar, filename, source_text)),
                }),
            }
        }
        Rule::ignored_lvalue => Node {
            source_info,
            value: IgnoredLValue,
        },
        Rule::ident_lvalue => {
            let nsvar = inner.into_inner().next().unwrap();
            Node {
                source_info,
                value: IdentLValue(IdentLValueNode {
                    identifier: Arc::new(parse_nsvarident(nsvar, filename, source_text)),
                }),
            }
        }
        Rule::sub_lvalue => {
            let lvs: Vec<_> = inner
                .into_inner()
                .map(|lv| Arc::new(parse_lvalue(lv, filename, source_text)))
                .collect();
            Node {
                source_info,
                value: SubLValue(SubLValueNode { lvalues: lvs }),
            }
        }
        _ => unreachable!(),
    }
}

/// Infix operators whose lexeme is also a [`Rule::prefix_op`] (`"+" | "-" | "!" | "%"`).
/// `!` is absent on purpose: the infix form is the two-character `!=`, so it can never be
/// confused with a prefix `!`.
fn is_prefix_capable_infix(rule: Rule) -> bool {
    matches!(rule, Rule::op_add | Rule::op_sub | Rule::op_mod)
}

/// Does this infix operator actually begin a NEW collection-literal element?
///
/// Literal elements are juxtaposed expressions, so `#(1 -2)` and `#('a' %'b')` are
/// ambiguous: the operator could be infix, or a prefix on the next element. Greedy parsing
/// always chose infix, silently turning `#(-1 -2)` into `#(-3)`. Spacing is the only signal
/// that separates the two readings — as in Ruby — so an operator **detached from its left
/// operand and glued to its right one** is a prefix that starts a new element. `#(5-3)` and
/// `#(5 - 3)` both stay subtraction.
fn starts_new_element(op: &Pair<Rule>, source_text: &str) -> bool {
    if !is_prefix_capable_infix(op.as_rule()) {
        return false;
    }
    let span = op.as_span();
    let before = source_text[..span.start()].chars().next_back();
    let after = source_text[span.end()..].chars().next();
    before.is_some_and(char::is_whitespace) && after.is_some_and(|c| !c.is_whitespace())
}

/// Parse one collection-literal element (`expr` / `set_elem`), splitting it into several
/// elements at each prefix-operator boundary — see [`starts_new_element`].
///
/// The split is on the *token* sequence, not the parsed tree: `#(1 -2 % 3)` parses greedily
/// as `1 - (2 % 3)` because `%` binds tighter than `-`, so re-associating the tree would
/// yield `-(2 % 3)` where the element is really `(-2) % 3`. Re-parsing each chunk lets pest
/// derive the precedence instead of us reconstructing it.
fn parse_literal_elements(pair: Pair<Rule>, filename: &str, source_text: &str) -> Vec<Node> {
    let pairs: Vec<Pair<Rule>> = pair.clone().into_inner().collect();
    // An operator never appears first (that position is `prefix_op` or `primary`), so
    // `skip(1)` only guards against a malformed sequence.
    let starts: Vec<usize> = std::iter::once(0)
        .chain(
            pairs
                .iter()
                .enumerate()
                .skip(1)
                .filter(|(_, p)| starts_new_element(p, source_text))
                .map(|(i, _)| i),
        )
        .collect();
    if starts.len() == 1 {
        return vec![parse_expr(pair, filename, source_text)];
    }

    let mut elements = Vec::with_capacity(starts.len());
    for (n, &begin) in starts.iter().enumerate() {
        let end_pair = starts.get(n + 1).copied().unwrap_or(pairs.len()) - 1;
        let (start, end) = (
            pairs[begin].as_span().start(),
            pairs[end_pair].as_span().end(),
        );
        match reparse_element(start, end, filename, source_text) {
            Some(node) => elements.push(node),
            // Unreachable for any chunk pest just parsed. Degrade to the old greedy
            // reading rather than fail the whole program on a parser invariant.
            None => return vec![parse_expr(pair, filename, source_text)],
        }
    }
    elements
}

/// Re-parse `source_text[start..end]` as one expression.
///
/// The chunk is parsed inside a buffer left-padded with spaces to `start`, so every span
/// keeps its original byte offset and `extract_source_info` (which reads the *original*
/// source) still reports the right line, column and snippet. Padding also bounds the parse
/// at `end`, so a chunk can't run past its element. Returns `None` if the chunk does not
/// re-parse in full.
fn reparse_element(start: usize, end: usize, filename: &str, source_text: &str) -> Option<Node> {
    let mut padded = " ".repeat(start);
    padded.push_str(&source_text[start..end]);
    let expr = QuoinParser::parse(Rule::elem_chunk, &padded)
        .ok()?
        .next()?
        .into_inner()
        .next()?;
    (expr.as_span().end() == end).then(|| parse_expr(expr, filename, source_text))
}

fn parse_expr(pair: Pair<Rule>, filename: &str, source_text: &str) -> Node {
    let pairs = pair.into_inner();
    PRATT_PARSER
        .map_primary(|primary| parse_primary(primary, filename, source_text))
        .map_prefix(|op, rhs| {
            let op_str = op.as_str();
            let op_type = match op_str {
                "-" => UnaryOperatorType::Sub,
                "+" => UnaryOperatorType::Add,
                "!" => UnaryOperatorType::Bang,
                "%" => UnaryOperatorType::Mod,
                _ => UnaryOperatorType::Unknown,
            };
            let source_info = combine_source_info(
                &extract_source_info(op.as_span(), filename, source_text),
                &rhs.source_info,
                source_text,
            );
            Node {
                source_info,
                value: UnaryOperator(UnaryOperatorNode {
                    operator: op_type,
                    right: Arc::new(rhs),
                }),
            }
        })
        .map_postfix(|lhs, op| {
            let call_sig = op.into_inner().next().unwrap();
            let sig_args = parse_call_sig(call_sig, filename, source_text);
            // Span the call through its last argument (e.g. `recv.pick:10`), falling
            // back to the last selector identifier for an argument-less call, so the
            // method-call node — and the `Send` that inherits its span — covers the
            // whole call rather than stopping at the selector.
            let end_span = sig_args
                .expressions
                .last()
                .and_then(|e| e.source_info.as_ref())
                .or_else(|| {
                    sig_args
                        .signature
                        .identifiers
                        .last()
                        .and_then(|id| id.source_info.as_ref())
                })
                .map(|si| si.end)
                .unwrap_or_else(|| lhs.source_info.as_ref().map(|si| si.end).unwrap_or(0));
            let source_info = combine_source_info(
                &lhs.source_info,
                &Some(SourceInfo {
                    filename: filename.to_string(),
                    line: 0,
                    column: 0,
                    start: lhs.source_info.as_ref().map(|si| si.start).unwrap_or(0),
                    end: end_span,
                    source_text: None,
                }),
                source_text,
            );
            Node {
                source_info,
                value: MethodCall(MethodCallNode {
                    subject: Some(Arc::new(lhs)),
                    arguments: Arc::new(sig_args),
                }),
            }
        })
        .map_infix(|lhs, op, rhs| {
            let source_info = combine_source_info(&lhs.source_info, &rhs.source_info, source_text);
            match op.as_rule() {
                Rule::op_class_ext => {
                    let block_node = match rhs.value {
                        Block(b) => b,
                        _ => unreachable!("Class extension right hand side must be a block"),
                    };
                    Node {
                        source_info,
                        value: ClassExtension(ClassExtensionNode {
                            expression: Arc::new(lhs),
                            block: Arc::new(block_node),
                        }),
                    }
                }
                Rule::op_range => Node {
                    source_info,
                    value: BinaryOperator(BinaryOperatorNode {
                        operator: BinaryOperatorType::Range,
                        left: Arc::new(lhs),
                        right: Arc::new(rhs),
                    }),
                },
                _ => {
                    let op_type = match op.as_rule() {
                        Rule::op_or => BinaryOperatorType::Or,
                        Rule::op_and => BinaryOperatorType::And,
                        Rule::op_eq => BinaryOperatorType::Eq,
                        Rule::op_ne => BinaryOperatorType::NotEq,
                        Rule::op_lt => BinaryOperatorType::Lt,
                        Rule::op_le => BinaryOperatorType::LtEq,
                        Rule::op_gt => BinaryOperatorType::Gt,
                        Rule::op_ge => BinaryOperatorType::GtEq,
                        Rule::op_match => BinaryOperatorType::Match,
                        Rule::op_add => BinaryOperatorType::Add,
                        Rule::op_sub => BinaryOperatorType::Sub,
                        Rule::op_mul => BinaryOperatorType::Mul,
                        Rule::op_div => BinaryOperatorType::Div,
                        Rule::op_mod => BinaryOperatorType::Mod,
                        _ => BinaryOperatorType::Unknown,
                    };
                    Node {
                        source_info,
                        value: BinaryOperator(BinaryOperatorNode {
                            operator: op_type,
                            left: Arc::new(lhs),
                            right: Arc::new(rhs),
                        }),
                    }
                }
            }
        })
        .parse(pairs)
}

fn parse_primary(pair: Pair<Rule>, filename: &str, source_text: &str) -> Node {
    let inner = pair.into_inner().next().unwrap();
    let source_info = extract_source_info(inner.as_span(), filename, source_text);
    match inner.as_rule() {
        Rule::nested_expr => {
            let expr = inner.into_inner().next().unwrap();
            parse_expr(expr, filename, source_text)
        }
        Rule::yield_return => {
            let expr = inner.into_inner().next().unwrap();
            Node {
                source_info,
                value: YieldReturn(YieldReturnNode {
                    value: Arc::new(parse_expr(expr, filename, source_text)),
                }),
            }
        }
        Rule::user_list_expr => {
            let mut pairs = inner.into_inner();
            let start_pair = pairs.next().unwrap(); // USER_LIST_START
            let start_str = start_pair.as_str();
            let ident_name = start_str
                .trim_start_matches('#')
                .trim_end_matches('(')
                .to_string();

            let mut values = Vec::new();
            for expr in pairs {
                values.extend(
                    parse_literal_elements(expr, filename, source_text)
                        .into_iter()
                        .map(Arc::new),
                );
            }
            Node {
                source_info,
                value: UserList(UserListNode {
                    identifier: Arc::new(IdentifierNode {
                        source_info: None,
                        namespace: None,
                        name: ident_name,
                        identifier_type: IdentifierType::Local,
                    }),
                    values,
                }),
            }
        }
        Rule::list_expr => {
            let values: Vec<_> = inner
                .into_inner()
                .flat_map(|expr| parse_literal_elements(expr, filename, source_text))
                .map(Arc::new)
                .collect();
            Node {
                source_info,
                value: List(ListNode { values }),
            }
        }
        Rule::set_expr => {
            let values: Vec<_> = inner
                .into_inner()
                .flat_map(|expr| parse_literal_elements(expr, filename, source_text))
                .map(Arc::new)
                .collect();
            Node {
                source_info,
                value: Set(SetNode { values }),
            }
        }
        Rule::dict_expr => {
            let mut keys = Vec::new();
            let mut values = Vec::new();
            let mut pairs = inner.into_inner();
            while let Some(k) = pairs.next() {
                let v = pairs.next().unwrap();
                keys.push(Arc::new(parse_expr(k, filename, source_text)));
                values.push(Arc::new(parse_expr(v, filename, source_text)));
            }
            Node {
                source_info,
                value: Map(MapNode { keys, values }),
            }
        }
        Rule::number_expr => {
            let raw = inner.as_str();
            if raw.contains('.') {
                let val: f64 = raw.parse().unwrap();
                Node {
                    source_info,
                    value: Double(DoubleNode { value: val }),
                }
            } else {
                let val: i64 = raw.parse().unwrap_or_else(|_| {
                    panic!(
                        "integer literal '{raw}' is out of range for Integer \
                         (max 9223372036854775807) — use BigInteger.parse:'…'"
                    )
                });
                Node {
                    source_info,
                    value: Integer(IntegerNode { value: val }),
                }
            }
        }
        Rule::string_expr => {
            let raw = inner.as_str();
            // Byte-slice between the single-byte `'` delimiters (not the char-indexed
            // `substring`, which mixed a char start with a byte-length end and swallowed
            // the closing quote on any multibyte content).
            let string_val = raw[1..raw.len() - 1].to_string();
            let unescaped = unescape(string_val);
            Node {
                source_info,
                value: Str(StringNode { value: unescaped }),
            }
        }
        Rule::symbol_expr => {
            let raw_symbol = inner.as_str();
            let symbol_val = raw_symbol
                .trim_start_matches(['#', '\''])
                .trim_end_matches('\'')
                .to_string();
            Node {
                source_info,
                value: Symbol(SymbolNode { value: symbol_val }),
            }
        }
        Rule::definition_expr => parse_definition_expr(inner, filename, source_text),
        Rule::block_expr => {
            let block_pair = inner.into_inner().next().unwrap();
            parse_block(block_pair, filename, source_text)
        }
        Rule::def_call_expr => {
            let call_sig_pair = inner.into_inner().next().unwrap();
            let sig_args = parse_call_sig(call_sig_pair, filename, source_text);
            Node {
                source_info,
                value: MethodCall(MethodCallNode {
                    subject: None,
                    arguments: Arc::new(sig_args),
                }),
            }
        }
        Rule::nsvarident_expr => {
            let nsvar = inner.into_inner().next().unwrap();
            let id = parse_nsvarident(nsvar, filename, source_text);
            Node {
                source_info,
                value: Identifier(id),
            }
        }
        Rule::regex_expr => {
            let raw_regex = inner.as_str().to_string();
            Node {
                source_info,
                value: Regex(RegexNode { value: raw_regex }),
            }
        }
        Rule::user_string_expr => {
            let raw_string = inner.as_str();
            let string_start = raw_string
                .find('\'')
                .unwrap_or_else(|| panic!("Invalid user string: {}", raw_string));
            // Byte-slice at the single-byte `#`/`'` delimiters: `[1..first_quote]` is the
            // identifier, `[first_quote+1..last_byte]` the body. `string_start` is already a
            // byte index (from `find`); the old char-indexed `substring` mixed the two and
            // leaked the closing quote on multibyte content.
            let ident_string = &raw_string[1..string_start];
            let string_string = raw_string[string_start + 1..raw_string.len() - 1].to_string();
            let unescaped_string = unescape(string_string);
            Node {
                source_info,
                value: UserString(UserStringNode {
                    identifier: Arc::new(IdentifierNode {
                        source_info: None,
                        namespace: None,
                        name: ident_string.to_string(),
                        identifier_type: IdentifierType::Local,
                    }),
                    value: unescaped_string,
                }),
            }
        }
        _ => unreachable!("Unexpected primary expression rule: {:?}", inner.as_rule()),
    }
}

fn parse_block(pair: Pair<Rule>, filename: &str, source_text: &str) -> Node {
    let source_info = extract_source_info(pair.as_span(), filename, source_text);
    let inner = pair.into_inner().next().unwrap();
    match inner.as_rule() {
        Rule::named_block_w_decls => {
            let mut pairs = inner.into_inner();
            let sym = pairs.next().unwrap();
            let raw_symbol = sym.as_str();
            let symbol_val = raw_symbol
                .trim_start_matches(['#', '\''])
                .trim_end_matches('\'')
                .to_string();
            let symbol_node = SymbolNode { value: symbol_val };
            let block_decls = pairs.next().unwrap();
            let (arguments, return_type, decls, decl_block) =
                parse_block_decls(block_decls, filename, source_text);
            let mut statements = Vec::new();
            for stmt in pairs {
                statements.push(Arc::new(parse_stmt(stmt, filename, source_text)));
            }
            Node {
                source_info: source_info.clone(),
                value: Block(BlockNode {
                    arguments,
                    return_type,
                    decls,
                    decl_block,
                    statements,
                    name: Some(Arc::new(symbol_node)),
                    source_info,
                }),
            }
        }
        Rule::block_w_decls => {
            let mut pairs = inner.into_inner();
            let block_decls = pairs.next().unwrap();
            let (arguments, return_type, decls, decl_block) =
                parse_block_decls(block_decls, filename, source_text);
            let mut statements = Vec::new();
            for stmt in pairs {
                statements.push(Arc::new(parse_stmt(stmt, filename, source_text)));
            }
            Node {
                source_info: source_info.clone(),
                value: Block(BlockNode {
                    arguments,
                    return_type,
                    decls,
                    decl_block,
                    statements,
                    name: None,
                    source_info,
                }),
            }
        }
        Rule::block_no_decls => {
            let pairs = inner.into_inner();
            let mut statements = Vec::new();
            for stmt in pairs {
                statements.push(Arc::new(parse_stmt(stmt, filename, source_text)));
            }
            Node {
                source_info: source_info.clone(),
                value: Block(BlockNode {
                    arguments: vec![],
                    decls: vec![],
                    return_type: None,
                    decl_block: None,
                    statements,
                    name: None,
                    source_info,
                }),
            }
        }
        _ => unreachable!(),
    }
}

#[allow(clippy::type_complexity)] // returns (args, param syms, return-type node) tuple
fn parse_block_decls(
    pair: Pair<Rule>,
    filename: &str,
    source_text: &str,
) -> (
    Vec<Arc<BlockArgNode>>,
    Option<Arc<TypeRefNode>>,
    Vec<Arc<BlockDeclNode>>,
    Option<Arc<BlockNode>>,
) {
    let mut arguments = Vec::new();
    let mut return_type = None;
    let mut decls = Vec::new();
    let mut decl_block = None;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::op_dash => {}
            Rule::block_arg => {
                let arg = parse_block_arg(inner, filename, source_text);
                arguments.push(Arc::new(arg));
            }
            Rule::block_ret => {
                let ty = inner.into_inner().next().unwrap();
                return_type = Some(Arc::new(parse_type_ref(ty, filename, source_text)));
            }
            Rule::block => {
                let blk = parse_block(inner, filename, source_text);
                if let Block(b) = blk.value {
                    decl_block = Some(Arc::new(b));
                } else {
                    unreachable!();
                }
            }
            Rule::block_decl => {
                let decl = parse_block_decl(inner, filename, source_text);
                decls.push(Arc::new(decl));
            }
            _ => unreachable!(),
        }
    }
    (arguments, return_type, decls, decl_block)
}

fn parse_block_arg(pair: Pair<Rule>, filename: &str, source_text: &str) -> BlockArgNode {
    let inner = pair.into_inner().next().unwrap();
    match inner.as_rule() {
        Rule::block_arg_ignored => BlockArgNode {
            identifier: Arc::new(IdentifierNode {
                source_info: None,
                namespace: None,
                name: "_".to_string(),
                identifier_type: IdentifierType::Local,
            }),
            type_hint: None,
        },
        Rule::block_arg_typed => {
            let mut inner_pairs = inner.into_inner();
            let arg_id = inner_pairs.next().unwrap();
            let type_hint_id = inner_pairs.next().unwrap();
            BlockArgNode {
                identifier: Arc::new(parse_arg_ident(arg_id, filename, source_text)),
                type_hint: Some(Arc::new(parse_type_ref(
                    type_hint_id,
                    filename,
                    source_text,
                ))),
            }
        }
        Rule::block_arg_untyped => {
            let arg_id = inner.into_inner().next().unwrap();
            BlockArgNode {
                identifier: Arc::new(parse_arg_ident(arg_id, filename, source_text)),
                type_hint: None,
            }
        }
        _ => unreachable!(),
    }
}

fn parse_block_decl(pair: Pair<Rule>, filename: &str, source_text: &str) -> BlockDeclNode {
    let inner = pair.into_inner().next().unwrap();
    match inner.as_rule() {
        Rule::block_decl_typed => {
            let mut inner_pairs = inner.into_inner();
            let arg_id = inner_pairs.next().unwrap();
            let type_hint_id = inner_pairs.next().unwrap();
            BlockDeclNode {
                identifier: Arc::new(parse_arg_ident(arg_id, filename, source_text)),
                type_hint: Some(Arc::new(parse_type_ref(
                    type_hint_id,
                    filename,
                    source_text,
                ))),
            }
        }
        Rule::block_decl_untyped => {
            let arg_id = inner.into_inner().next().unwrap();
            BlockDeclNode {
                identifier: Arc::new(parse_arg_ident(arg_id, filename, source_text)),
                type_hint: None,
            }
        }
        _ => unreachable!(),
    }
}

fn parse_arg_ident(pair: Pair<Rule>, filename: &str, source_text: &str) -> IdentifierNode {
    let inner = pair.into_inner().next().unwrap();
    let source_info = extract_source_info(inner.as_span(), filename, source_text);
    match inner.as_rule() {
        Rule::arg_ident_inst => {
            let ident_pair = inner.into_inner().next().unwrap();
            IdentifierNode {
                source_info,
                namespace: None,
                name: ident_pair.as_str().to_string(),
                identifier_type: IdentifierType::Instance,
            }
        }
        Rule::arg_ident_normal => {
            let ident_pair = inner.into_inner().next().unwrap();
            IdentifierNode {
                source_info,
                namespace: None,
                name: ident_pair.as_str().to_string(),
                identifier_type: IdentifierType::Local,
            }
        }
        _ => unreachable!(),
    }
}

fn parse_ident(pair: Pair<Rule>, filename: &str, source_text: &str) -> IdentifierNode {
    let source_info = extract_source_info(pair.as_span(), filename, source_text);
    let inner = pair.into_inner().next().unwrap();
    let name = inner.as_str().to_string();
    let identifier_type = match inner.as_rule() {
        Rule::reserved_ident => IdentifierType::ReservedIdentifier,
        _ => IdentifierType::Local,
    };
    IdentifierNode {
        source_info,
        namespace: None,
        name,
        identifier_type,
    }
}

/// A `type_ref` — an optionally namespaced type-annotation name (`Integer`, `[Web]Halt`).
/// Mirrors `parse_nsvarident`'s namespaced arm so a namespaced type carries the same
/// `namespace` + `IdentifierType::Namespaced` shape as an expression-position reference.
fn parse_type_ref(pair: Pair<Rule>, filename: &str, source_text: &str) -> TypeRefNode {
    let source_info = extract_source_info(pair.as_span(), filename, source_text);
    let mut inner = pair.into_inner();
    let first = inner.next().unwrap();
    let (ident, rest) = match first.as_rule() {
        Rule::namespace => {
            let ns_node = parse_namespace(first, filename, source_text);
            let id = parse_ident(inner.next().unwrap(), filename, source_text);
            (
                IdentifierNode {
                    source_info,
                    namespace: Some(Arc::new(ns_node)),
                    name: id.name,
                    identifier_type: IdentifierType::Namespaced,
                },
                inner.next(),
            )
        }
        Rule::ident => (parse_ident(first, filename, source_text), inner.next()),
        _ => unreachable!(),
    };
    let (args, ret, parenthesized) = match rest {
        Some(ta) if ta.as_rule() == Rule::type_args => {
            let mut args = Vec::new();
            let mut ret = None;
            for p in ta.into_inner() {
                match p.as_rule() {
                    Rule::type_ret => {
                        let inner = p.into_inner().next().unwrap();
                        ret = Some(Arc::new(parse_type_ref(inner, filename, source_text)));
                    }
                    _ => args.push(Arc::new(parse_type_ref(p, filename, source_text))),
                }
            }
            (args, ret, true)
        }
        _ => (Vec::new(), None, false),
    };
    TypeRefNode {
        ident: Arc::new(ident),
        args,
        ret,
        parenthesized,
    }
}

/// Class-header type parameters (`Iterate(T U)`): bare names.
fn parse_type_params(pair: Pair<Rule>, filename: &str, source_text: &str) -> Vec<String> {
    pair.into_inner()
        .map(|id| parse_ident(id, filename, source_text).name)
        .collect()
}

fn parse_definition_expr(pair: Pair<Rule>, filename: &str, source_text: &str) -> Node {
    let source_info = extract_source_info(pair.as_span(), filename, source_text);
    let inner = pair.into_inner().next().unwrap();
    match inner.as_rule() {
        Rule::class_def_2 => {
            let mut pairs = inner.into_inner();
            let parent_id = parse_nsvarident(pairs.next().unwrap(), filename, source_text);
            let child_id = parse_nsvarident(pairs.next().unwrap(), filename, source_text);
            let mut next = pairs.next().unwrap();
            let type_params = if next.as_rule() == Rule::type_params {
                let tp = parse_type_params(next, filename, source_text);
                next = pairs.next().unwrap();
                tp
            } else {
                Vec::new()
            };
            let block_node = match parse_block(next, filename, source_text).value {
                Block(b) => b,
                _ => unreachable!(),
            };
            Node {
                source_info,
                value: ClassDefinition(ClassDefinitionNode {
                    identifier: Arc::new(child_id),
                    parent_identifier: Some(Arc::new(parent_id)),
                    type_params,
                    block: Arc::new(block_node),
                }),
            }
        }
        Rule::class_def => {
            let mut pairs = inner.into_inner();
            let child_id = parse_nsvarident(pairs.next().unwrap(), filename, source_text);
            let mut next = pairs.next().unwrap();
            let type_params = if next.as_rule() == Rule::type_params {
                let tp = parse_type_params(next, filename, source_text);
                next = pairs.next().unwrap();
                tp
            } else {
                Vec::new()
            };
            let block_node = match parse_block(next, filename, source_text).value {
                Block(b) => b,
                _ => unreachable!(),
            };
            Node {
                source_info,
                value: ClassDefinition(ClassDefinitionNode {
                    identifier: Arc::new(child_id),
                    parent_identifier: None,
                    type_params,
                    block: Arc::new(block_node),
                }),
            }
        }
        Rule::const_def => {
            let mut pairs = inner.into_inner();
            let id = parse_nsvarident(pairs.next().unwrap(), filename, source_text);
            let expr_pair = pairs.next().unwrap();
            Node {
                source_info,
                value: ConstDefinition(ConstDefinitionNode {
                    identifier: Arc::new(id),
                    rvalue: Arc::new(parse_expr(expr_pair, filename, source_text)),
                }),
            }
        }
        Rule::method_def => {
            let mut pairs = inner.into_inner();
            let selector = parse_selector(pairs.next().unwrap(), filename, source_text);
            let block_node = match parse_block(pairs.next().unwrap(), filename, source_text).value {
                Block(b) => b,
                _ => unreachable!(),
            };
            Node {
                source_info,
                value: MethodDefinition(MethodDefinitionNode {
                    signature: Arc::new(selector),
                    block: Arc::new(block_node),
                }),
            }
        }
        Rule::method_ext => {
            let mut pairs = inner.into_inner();
            let selector = parse_selector(pairs.next().unwrap(), filename, source_text);
            let block_node = match parse_block(pairs.next().unwrap(), filename, source_text).value {
                Block(b) => b,
                _ => unreachable!(),
            };
            Node {
                source_info,
                value: MethodExtension(MethodExtensionNode {
                    signature: Arc::new(selector),
                    block: Arc::new(block_node),
                }),
            }
        }
        _ => unreachable!(),
    }
}

fn parse_selector(pair: Pair<Rule>, filename: &str, source_text: &str) -> MethodSelectorNode {
    let inner = pair.into_inner().next().unwrap();
    match inner.as_rule() {
        Rule::selector_w_args => {
            // A keyword component may be marked variadic with `+` (`catch+:`). Bake the `+` into
            // the canonical name so a `catch+:finally:` definition interns the same selector that a
            // folded `catch:…catch:…finally:` call produces (see `compile_method_call`).
            let mut identifiers = Vec::new();
            let mut pairs = inner.into_inner().peekable();
            while let Some(id_pair) = pairs.next() {
                let id = parse_ident(id_pair, filename, source_text);
                let variadic = matches!(pairs.peek().map(|p| p.as_rule()), Some(Rule::kw_var));
                if variadic {
                    pairs.next(); // consume the `+`
                }
                let name = if variadic {
                    format!("{}+:", id.name)
                } else {
                    format!("{}:", id.name)
                };
                identifiers.push(Arc::new(IdentifierNode {
                    source_info: id.source_info.clone(),
                    namespace: id.namespace.clone(),
                    name,
                    identifier_type: id.identifier_type,
                }));
            }
            MethodSelectorNode { identifiers }
        }
        Rule::selector_no_args_bang => {
            let id_pair = inner.into_inner().next().unwrap();
            let id = parse_ident(id_pair, filename, source_text);
            MethodSelectorNode {
                identifiers: vec![Arc::new(IdentifierNode {
                    source_info: id.source_info.clone(),
                    namespace: id.namespace.clone(),
                    name: format!("{}!", id.name),
                    identifier_type: id.identifier_type,
                })],
            }
        }
        Rule::selector_no_args => {
            let id_pair = inner.into_inner().next().unwrap();
            let id = parse_ident(id_pair, filename, source_text);
            MethodSelectorNode {
                identifiers: vec![Arc::new(id)],
            }
        }
        Rule::selector_symbol => {
            let sym_pair = inner.into_inner().next().unwrap();
            let raw_sym = sym_pair.as_str();
            let name = raw_sym
                .trim_start_matches('#')
                .trim_matches('\'')
                .to_string();
            let source_info = extract_source_info(sym_pair.as_span(), filename, source_text);
            MethodSelectorNode {
                identifiers: vec![Arc::new(IdentifierNode {
                    source_info,
                    namespace: None,
                    name,
                    identifier_type: IdentifierType::Local,
                })],
            }
        }
        _ => unreachable!(),
    }
}

fn parse_call_sig(pair: Pair<Rule>, filename: &str, source_text: &str) -> MethodCallArgumentsNode {
    let inner = pair.into_inner().next().unwrap();
    match inner.as_rule() {
        Rule::call_sig_w_arg => {
            let mut idents = Vec::new();
            let mut exprs = Vec::new();
            let mut pairs = inner.into_inner();
            while let Some(id_pair) = pairs.next() {
                let expr_pair = pairs.next().unwrap();
                let id = parse_ident(id_pair, filename, source_text);
                idents.push(Arc::new(id));
                exprs.push(Arc::new(parse_expr(expr_pair, filename, source_text)));
            }
            MethodCallArgumentsNode {
                signature: Arc::new(MethodSelectorNode {
                    identifiers: idents,
                }),
                expressions: exprs,
            }
        }
        Rule::call_sig_no_arg_bang => {
            let id_pair = inner.into_inner().next().unwrap();
            let id = parse_ident(id_pair, filename, source_text);
            MethodCallArgumentsNode {
                signature: Arc::new(MethodSelectorNode {
                    identifiers: vec![Arc::new(IdentifierNode {
                        source_info: id.source_info.clone(),
                        namespace: id.namespace.clone(),
                        name: format!("{}!", id.name),
                        identifier_type: id.identifier_type,
                    })],
                }),
                expressions: vec![],
            }
        }
        Rule::call_sig_no_arg => {
            let id_pair = inner.into_inner().next().unwrap();
            let id = parse_ident(id_pair, filename, source_text);
            MethodCallArgumentsNode {
                signature: Arc::new(MethodSelectorNode {
                    identifiers: vec![Arc::new(id)],
                }),
                expressions: vec![],
            }
        }
        _ => unreachable!(),
    }
}

fn parse_nsvarident(pair: Pair<Rule>, filename: &str, source_text: &str) -> IdentifierNode {
    let inner = pair.into_inner().next().unwrap();
    let source_info = extract_source_info(inner.as_span(), filename, source_text);
    match inner.as_rule() {
        Rule::namespaced_ident => {
            let mut inner_pairs = inner.into_inner();
            let ns_pair = inner_pairs.next().unwrap();
            let id_pair = inner_pairs.next().unwrap();
            let ns_node = parse_namespace(ns_pair, filename, source_text);
            let id = parse_ident(id_pair, filename, source_text);
            IdentifierNode {
                source_info,
                namespace: Some(Arc::new(ns_node)),
                name: id.name,
                identifier_type: IdentifierType::Namespaced,
            }
        }
        Rule::instance_ident => {
            let id_pair = inner.into_inner().next().unwrap();
            let id = parse_ident(id_pair, filename, source_text);
            IdentifierNode {
                source_info,
                namespace: None,
                name: id.name,
                identifier_type: IdentifierType::Instance,
            }
        }
        Rule::local_ident => {
            let id_pair = inner.into_inner().next().unwrap();
            let id = parse_ident(id_pair, filename, source_text);
            IdentifierNode {
                source_info,
                namespace: None,
                name: id.name,
                identifier_type: IdentifierType::Local,
            }
        }
        _ => unreachable!(),
    }
}

fn parse_namespace(pair: Pair<Rule>, filename: &str, source_text: &str) -> NamespaceNode {
    let source_info = extract_source_info(pair.as_span(), filename, source_text);
    let inner = pair.into_inner().next().unwrap();
    match inner.as_rule() {
        Rule::root_ns => NamespaceNode {
            source_info,
            identifiers: vec![],
        },
        Rule::full_ns => {
            let idents: Vec<_> = inner
                .into_inner()
                .map(|id_pair| Arc::new(parse_ident(id_pair, filename, source_text)))
                .collect();
            NamespaceNode {
                source_info,
                identifiers: idents,
            }
        }
        _ => unreachable!(),
    }
}

fn unescape(s: String) -> String {
    static ESCAPED_CHAR: Lazy<regex::Regex> = Lazy::new(|| {
        regex::Regex::new("\\\\([ux][0-9a-fA-F][0-9a-fA-F][0-9a-fA-F][0-9a-fA-F]|[\\\\tnr\"'])")
            .unwrap()
    });

    ESCAPED_CHAR
        .replace_all(s.as_str(), |caps: &Captures| {
            let s = caps[1].to_string();
            match s.as_str().substring(0, 1) {
                "n" => "\n".to_string(),
                "r" => "\r".to_string(),
                "t" => "\t".to_string(),
                "u" | "x" => {
                    let hex = s.substring(1, s.len()).to_string();
                    let maybe_char = unicode_from_hex(hex.clone());
                    match maybe_char {
                        Some(x) => x.to_string(),
                        None => panic!(
                            "invalid unicode escape \\u{hex} (not a valid \
                             scalar value — unpaired surrogates are not chars)"
                        ),
                    }
                }
                _ => s,
            }
        })
        .to_string()
}

fn unicode_from_hex(s: String) -> Option<char> {
    let char_num: u32 = match u32::from_str_radix(s.as_str(), 16) {
        Ok(n) => n,
        Err(e) => panic!("Invalid unicode hex value \\x{s}: {}", e),
    };

    char::from_u32(char_num)
}

#[cfg(test)]
mod tests {
    use super::parse_quoin_string;
    use crate::ast::*;
    use std::sync::Arc;

    fn parse(code: &str) -> Node {
        let mut node = parse_quoin_string(code);
        node.clear_source_info();
        node
    }

    fn val_node(val: NodeValue) -> Node {
        Node {
            source_info: None,
            value: val,
        }
    }

    fn arc_node(val: NodeValue) -> Arc<Node> {
        Arc::new(val_node(val))
    }

    fn ident(name: &str, identifier_type: IdentifierType) -> Arc<Node> {
        arc_node(NodeValue::Identifier(IdentifierNode {
            source_info: None,
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
        let ast = parse("123;");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![integer(123)],
        }));
        assert_eq!(ast, expected);

        let ast = parse("12.34;");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![double(12.34)],
        }));
        assert_eq!(ast, expected);

        let ast = parse("'hello';");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![string_node("hello")],
        }));
        assert_eq!(ast, expected);

        let ast = parse("#foo;");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![symbol("foo")],
        }));
        assert_eq!(ast, expected);

        let ast = parse("#/^[a-z]+$/;");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![arc_node(NodeValue::Regex(RegexNode {
                value: "#/^[a-z]+$/".to_string(),
            }))],
        }));
        assert_eq!(ast, expected);
    }

    #[test]
    fn test_parse_identifiers() {
        let ast = parse("x;");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![ident("x", IdentifierType::Local)],
        }));
        assert_eq!(ast, expected);

        let ast = parse("@x;");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![ident("x", IdentifierType::Instance)],
        }));
        assert_eq!(ast, expected);
    }

    #[test]
    fn test_parse_assignment() {
        let ast = parse("x = 42;");
        let lval = arc_node(NodeValue::IdentLValue(IdentLValueNode {
            identifier: Arc::new(IdentifierNode {
                source_info: None,
                namespace: None,
                name: "x".to_string(),
                identifier_type: IdentifierType::Local,
            }),
        }));
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![arc_node(NodeValue::Assignment(AssignmentNode {
                lvalues: vec![lval],
                rvalue: integer(42),
            }))],
        }));
        assert_eq!(ast, expected);
    }

    #[test]
    fn test_parse_operators() {
        let ast = parse("1 + 2;");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![binary(BinaryOperatorType::Add, integer(1), integer(2))],
        }));
        assert_eq!(ast, expected);

        let ast = parse("!x;");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![unary(
                UnaryOperatorType::Bang,
                ident("x", IdentifierType::Local),
            )],
        }));
        assert_eq!(ast, expected);
    }

    #[test]
    fn test_parse_list_and_map() {
        let ast = parse("#(1 2);");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![arc_node(NodeValue::List(ListNode {
                values: vec![integer(1), integer(2)],
            }))],
        }));
        assert_eq!(ast, expected);

        let ast = parse("#{'a': 1};");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![arc_node(NodeValue::Map(MapNode {
                keys: vec![string_node("a")],
                values: vec![integer(1)],
            }))],
        }));
        assert_eq!(ast, expected);
    }

    #[test]
    fn test_parse_block() {
        let ast = parse("{ 1 + 2 };");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![arc_node(NodeValue::Block(BlockNode {
                source_info: None,
                name: None,
                arguments: vec![],
                decls: vec![],
                return_type: None,
                decl_block: None,
                statements: vec![binary(BinaryOperatorType::Add, integer(1), integer(2))],
            }))],
        }));
        assert_eq!(ast, expected);
    }

    fn block_arg(
        name: &str,
        identifier_type: IdentifierType,
        type_hint: Option<Arc<TypeRefNode>>,
    ) -> Arc<BlockArgNode> {
        Arc::new(BlockArgNode {
            identifier: Arc::new(IdentifierNode {
                source_info: None,
                namespace: None,
                name: name.to_string(),
                identifier_type,
            }),
            type_hint,
        })
    }

    fn block_decl(
        name: &str,
        identifier_type: IdentifierType,
        type_hint: Option<Arc<TypeRefNode>>,
    ) -> Arc<BlockDeclNode> {
        Arc::new(BlockDeclNode {
            identifier: Arc::new(IdentifierNode {
                source_info: None,
                namespace: None,
                name: name.to_string(),
                identifier_type,
            }),
            type_hint,
        })
    }

    fn ident_node(name: &str, identifier_type: IdentifierType) -> Arc<IdentifierNode> {
        Arc::new(IdentifierNode {
            source_info: None,
            namespace: None,
            name: name.to_string(),
            identifier_type,
        })
    }

    #[test]
    fn test_parse_method_call() {
        let ast = parse("x.negated;");
        let selector = Arc::new(MethodSelectorNode {
            identifiers: vec![Arc::new(IdentifierNode {
                source_info: None,
                namespace: None,
                name: "negated".to_string(),
                identifier_type: IdentifierType::Local,
            })],
        });
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
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
        let ast = parse("!!!;");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![arc_node(NodeValue::Bang3)],
        }));
        assert_eq!(ast, expected);

        let ast = parse("...;");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![arc_node(NodeValue::Dot3)],
        }));
        assert_eq!(ast, expected);

        let ast = parse("???;");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![arc_node(NodeValue::Huh3)],
        }));
        assert_eq!(ast, expected);

        let ast = parse("^x;");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![arc_node(NodeValue::BlockReturn(BlockReturnNode {
                value: ident("x", IdentifierType::Local),
            }))],
        }));
        assert_eq!(ast, expected);

        let ast = parse("^>x;");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![arc_node(NodeValue::YieldReturn(YieldReturnNode {
                value: ident("x", IdentifierType::Local),
            }))],
        }));
        assert_eq!(ast, expected);

        let ast = parse("^^x;");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![arc_node(NodeValue::MethodReturn(MethodReturnNode {
                value: ident("x", IdentifierType::Local),
            }))],
        }));
        assert_eq!(ast, expected);
    }

    #[test]
    fn test_parse_assignment_lvalues() {
        // Splat: *rest = x;
        let ast = parse("*rest = x;");
        let lval = arc_node(NodeValue::SplatLValue(SplatLValueNode {
            identifier: Arc::new(IdentifierNode {
                source_info: None,
                namespace: None,
                name: "rest".to_string(),
                identifier_type: IdentifierType::Local,
            }),
        }));
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![arc_node(NodeValue::Assignment(AssignmentNode {
                lvalues: vec![lval],
                rvalue: ident("x", IdentifierType::Local),
            }))],
        }));
        assert_eq!(ast, expected);

        // Ignored: _ = x;
        let ast = parse("_ = x;");
        let lval = arc_node(NodeValue::IgnoredLValue);
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![arc_node(NodeValue::Assignment(AssignmentNode {
                lvalues: vec![lval],
                rvalue: ident("x", IdentifierType::Local),
            }))],
        }));
        assert_eq!(ast, expected);

        // Ignored Splat: *_ = x;
        let ast = parse("*_ = x;");
        let lval = arc_node(NodeValue::IgnoredSplatLValue);
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![arc_node(NodeValue::Assignment(AssignmentNode {
                lvalues: vec![lval],
                rvalue: ident("x", IdentifierType::Local),
            }))],
        }));
        assert_eq!(ast, expected);

        // SubLValue: (a *b) = x;
        let ast = parse("(a *b) = x;");
        let lval_a = arc_node(NodeValue::IdentLValue(IdentLValueNode {
            identifier: Arc::new(IdentifierNode {
                source_info: None,
                namespace: None,
                name: "a".to_string(),
                identifier_type: IdentifierType::Local,
            }),
        }));
        let lval_b = arc_node(NodeValue::SplatLValue(SplatLValueNode {
            identifier: Arc::new(IdentifierNode {
                source_info: None,
                namespace: None,
                name: "b".to_string(),
                identifier_type: IdentifierType::Local,
            }),
        }));
        let sub_lval = arc_node(NodeValue::SubLValue(SubLValueNode {
            lvalues: vec![lval_a, lval_b],
        }));
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
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
            let ast = parse(&code);
            let expected = val_node(NodeValue::Program(ProgramNode {
                source_info: None,
                allow_pragmas: Vec::new(),
                expressions: vec![binary(op_type, integer(1), integer(2))],
            }));
            assert_eq!(ast, expected);
        }
    }

    #[test]
    fn test_parse_unary_operators_all() {
        let ast = parse("+x;");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![unary(
                UnaryOperatorType::Add,
                ident("x", IdentifierType::Local),
            )],
        }));
        assert_eq!(ast, expected);

        let ast = parse("-x;");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![unary(
                UnaryOperatorType::Sub,
                ident("x", IdentifierType::Local),
            )],
        }));
        assert_eq!(ast, expected);

        let ast = parse("%x;");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![unary(
                UnaryOperatorType::Mod,
                ident("x", IdentifierType::Local),
            )],
        }));
        assert_eq!(ast, expected);
    }

    #[test]
    fn test_parse_classes_and_consts() {
        // Const Definition: MY_CONST <- 42;
        let ast = parse("MY_CONST <- 42;");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![arc_node(NodeValue::ConstDefinition(ConstDefinitionNode {
                identifier: Arc::new(IdentifierNode {
                    source_info: None,
                    namespace: None,
                    name: "MY_CONST".to_string(),
                    identifier_type: IdentifierType::Local,
                }),
                rvalue: integer(42),
            }))],
        }));
        assert_eq!(ast, expected);

        // Class Definition: MyClass <- { 1 };
        let ast = parse("MyClass <- { 1 };");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![arc_node(NodeValue::ClassDefinition(ClassDefinitionNode {
                identifier: Arc::new(IdentifierNode {
                    source_info: None,
                    namespace: None,
                    name: "MyClass".to_string(),
                    identifier_type: IdentifierType::Local,
                }),
                parent_identifier: None,
                type_params: vec![],
                block: Arc::new(BlockNode {
                    source_info: None,
                    name: None,
                    arguments: vec![],
                    decls: vec![],
                    return_type: None,
                    decl_block: None,
                    statements: vec![integer(1)],
                }),
            }))],
        }));
        assert_eq!(ast, expected);

        // Class Definition 2: ParentClass <- ChildClass <- { 1 };
        let ast = parse("ParentClass <- ChildClass <- { 1 };");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![arc_node(NodeValue::ClassDefinition(ClassDefinitionNode {
                identifier: Arc::new(IdentifierNode {
                    source_info: None,
                    namespace: None,
                    name: "ChildClass".to_string(),
                    identifier_type: IdentifierType::Local,
                }),
                parent_identifier: Some(Arc::new(IdentifierNode {
                    source_info: None,
                    namespace: None,
                    name: "ParentClass".to_string(),
                    identifier_type: IdentifierType::Local,
                })),
                type_params: vec![],
                block: Arc::new(BlockNode {
                    source_info: None,
                    name: None,
                    arguments: vec![],
                    decls: vec![],
                    return_type: None,
                    decl_block: None,
                    statements: vec![integer(1)],
                }),
            }))],
        }));
        assert_eq!(ast, expected);

        // Class Extension: MyClass <-- { 1 };
        let ast = parse("MyClass <-- { 1 };");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![arc_node(NodeValue::ClassExtension(ClassExtensionNode {
                expression: ident("MyClass", IdentifierType::Local),
                block: Arc::new(BlockNode {
                    source_info: None,
                    name: None,
                    arguments: vec![],
                    decls: vec![],
                    return_type: None,
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
        let ast = parse("foo -> { 1 };");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![arc_node(NodeValue::MethodDefinition(
                MethodDefinitionNode {
                    signature: Arc::new(MethodSelectorNode {
                        identifiers: vec![Arc::new(IdentifierNode {
                            source_info: None,
                            namespace: None,
                            name: "foo".to_string(),
                            identifier_type: IdentifierType::Local,
                        })],
                    }),
                    block: Arc::new(BlockNode {
                        source_info: None,
                        name: None,
                        arguments: vec![],
                        decls: vec![],
                        return_type: None,
                        decl_block: None,
                        statements: vec![integer(1)],
                    }),
                },
            ))],
        }));
        assert_eq!(ast, expected);

        // SelectorNoArgsBang
        let ast = parse("foo! -> { 1 };");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![arc_node(NodeValue::MethodDefinition(
                MethodDefinitionNode {
                    signature: Arc::new(MethodSelectorNode {
                        identifiers: vec![Arc::new(IdentifierNode {
                            source_info: None,
                            namespace: None,
                            name: "foo!".to_string(),
                            identifier_type: IdentifierType::Local,
                        })],
                    }),
                    block: Arc::new(BlockNode {
                        source_info: None,
                        name: None,
                        arguments: vec![],
                        decls: vec![],
                        return_type: None,
                        decl_block: None,
                        statements: vec![integer(1)],
                    }),
                },
            ))],
        }));
        assert_eq!(ast, expected);

        // SelectorWArgs
        let ast = parse("foo: bar: -> { 1 };");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![arc_node(NodeValue::MethodDefinition(
                MethodDefinitionNode {
                    signature: Arc::new(MethodSelectorNode {
                        identifiers: vec![
                            Arc::new(IdentifierNode {
                                source_info: None,
                                namespace: None,
                                name: "foo:".to_string(),
                                identifier_type: IdentifierType::Local,
                            }),
                            Arc::new(IdentifierNode {
                                source_info: None,
                                namespace: None,
                                name: "bar:".to_string(),
                                identifier_type: IdentifierType::Local,
                            }),
                        ],
                    }),
                    block: Arc::new(BlockNode {
                        source_info: None,
                        name: None,
                        arguments: vec![],
                        decls: vec![],
                        return_type: None,
                        decl_block: None,
                        statements: vec![integer(1)],
                    }),
                },
            ))],
        }));
        assert_eq!(ast, expected);

        // SelectorSymbol
        let ast = parse("#foo -> { 1 };");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![arc_node(NodeValue::MethodDefinition(
                MethodDefinitionNode {
                    signature: Arc::new(MethodSelectorNode {
                        identifiers: vec![Arc::new(IdentifierNode {
                            source_info: None,
                            namespace: None,
                            name: "foo".to_string(),
                            identifier_type: IdentifierType::Local,
                        })],
                    }),
                    block: Arc::new(BlockNode {
                        source_info: None,
                        name: None,
                        arguments: vec![],
                        decls: vec![],
                        return_type: None,
                        decl_block: None,
                        statements: vec![integer(1)],
                    }),
                },
            ))],
        }));
        assert_eq!(ast, expected);

        // MethodExtension
        let ast = parse("foo --> { 1 };");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![arc_node(NodeValue::MethodExtension(MethodExtensionNode {
                signature: Arc::new(MethodSelectorNode {
                    identifiers: vec![Arc::new(IdentifierNode {
                        source_info: None,
                        namespace: None,
                        name: "foo".to_string(),
                        identifier_type: IdentifierType::Local,
                    })],
                }),
                block: Arc::new(BlockNode {
                    source_info: None,
                    name: None,
                    arguments: vec![],
                    decls: vec![],
                    return_type: None,
                    decl_block: None,
                    statements: vec![integer(1)],
                }),
            }))],
        }));
        assert_eq!(ast, expected);

        // SelectorNoArgs with a reserved identifier
        let ast = parse("nil -> { 1 };");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![arc_node(NodeValue::MethodDefinition(
                MethodDefinitionNode {
                    signature: Arc::new(MethodSelectorNode {
                        identifiers: vec![Arc::new(IdentifierNode {
                            source_info: None,
                            namespace: None,
                            name: "nil".to_string(),
                            identifier_type: IdentifierType::ReservedIdentifier,
                        })],
                    }),
                    block: Arc::new(BlockNode {
                        source_info: None,
                        name: None,
                        arguments: vec![],
                        decls: vec![],
                        return_type: None,
                        decl_block: None,
                        statements: vec![integer(1)],
                    }),
                },
            ))],
        }));
        assert_eq!(ast, expected);
    }

    #[test]
    fn test_parse_method_calls() {
        // Implicit subject (DefCall): .foo;
        let ast = parse(".foo;");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![arc_node(NodeValue::MethodCall(MethodCallNode {
                subject: None,
                arguments: Arc::new(MethodCallArgumentsNode {
                    signature: Arc::new(MethodSelectorNode {
                        identifiers: vec![Arc::new(IdentifierNode {
                            source_info: None,
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
        let ast = parse(".foo!;");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![arc_node(NodeValue::MethodCall(MethodCallNode {
                subject: None,
                arguments: Arc::new(MethodCallArgumentsNode {
                    signature: Arc::new(MethodSelectorNode {
                        identifiers: vec![Arc::new(IdentifierNode {
                            source_info: None,
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
        let ast = parse("x.foo!;");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![arc_node(NodeValue::MethodCall(MethodCallNode {
                subject: Some(ident("x", IdentifierType::Local)),
                arguments: Arc::new(MethodCallArgumentsNode {
                    signature: Arc::new(MethodSelectorNode {
                        identifiers: vec![Arc::new(IdentifierNode {
                            source_info: None,
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
        let ast = parse("x.foo: 1 bar: 2;");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![arc_node(NodeValue::MethodCall(MethodCallNode {
                subject: Some(ident("x", IdentifierType::Local)),
                arguments: Arc::new(MethodCallArgumentsNode {
                    signature: Arc::new(MethodSelectorNode {
                        identifiers: vec![
                            Arc::new(IdentifierNode {
                                source_info: None,
                                namespace: None,
                                name: "foo".to_string(),
                                identifier_type: IdentifierType::Local,
                            }),
                            Arc::new(IdentifierNode {
                                source_info: None,
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
    fn test_parse_namespaces_and_reserved_idents() {
        // Namespaced Ident: [foo/bar]baz;
        let ast = parse("[foo/bar]baz;");
        let ns = Arc::new(NamespaceNode {
            source_info: None,
            identifiers: vec![
                Arc::new(IdentifierNode {
                    source_info: None,
                    namespace: None,
                    name: "foo".to_string(),
                    identifier_type: IdentifierType::Local,
                }),
                Arc::new(IdentifierNode {
                    source_info: None,
                    namespace: None,
                    name: "bar".to_string(),
                    identifier_type: IdentifierType::Local,
                }),
            ],
        });
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![arc_node(NodeValue::Identifier(IdentifierNode {
                source_info: None,
                namespace: Some(ns),
                name: "baz".to_string(),
                identifier_type: IdentifierType::Namespaced,
            }))],
        }));
        assert_eq!(ast, expected);

        // Root namespace: [/]baz;
        let ast = parse("[/]baz;");
        let ns = Arc::new(NamespaceNode {
            source_info: None,
            identifiers: vec![],
        });
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![arc_node(NodeValue::Identifier(IdentifierNode {
                source_info: None,
                namespace: Some(ns),
                name: "baz".to_string(),
                identifier_type: IdentifierType::Namespaced,
            }))],
        }));
        assert_eq!(ast, expected);

        // Reserved identifiers used as names: nil; true; false;
        let ast = parse("nil;");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![arc_node(NodeValue::Identifier(IdentifierNode {
                source_info: None,
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
        let ast = parse("#<1 2>;");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![arc_node(NodeValue::Set(SetNode {
                values: vec![integer(1), integer(2)],
            }))],
        }));
        assert_eq!(ast, expected);

        // User string: #MyStr'hello';
        let ast = parse("#MyStr'hello';");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![arc_node(NodeValue::UserString(UserStringNode {
                identifier: Arc::new(IdentifierNode {
                    source_info: None,
                    namespace: None,
                    name: "MyStr".to_string(),
                    identifier_type: IdentifierType::Local,
                }),
                value: "hello".to_string(),
            }))],
        }));
        assert_eq!(ast, expected);

        // User list: #MyList(1 2);
        let ast = parse("#MyList(1 2);");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![arc_node(NodeValue::UserList(UserListNode {
                identifier: Arc::new(IdentifierNode {
                    source_info: None,
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
        let ast = parse("{ #my_block |x| 1; };");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![arc_node(NodeValue::Block(BlockNode {
                source_info: None,
                name: Some(Arc::new(SymbolNode {
                    value: "my_block".to_string(),
                })),
                arguments: vec![block_arg("x", IdentifierType::Local, None)],
                decls: vec![],
                return_type: None,
                decl_block: None,
                statements: vec![integer(1)],
            }))],
        }));
        assert_eq!(ast, expected);

        // Typed block arg: { |x:Int| 1; }
        let ast = parse("{ |x:Int| 1; };");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![arc_node(NodeValue::Block(BlockNode {
                source_info: None,
                name: None,
                arguments: vec![block_arg(
                    "x",
                    IdentifierType::Local,
                    Some(flat_type("Int")),
                )],
                decls: vec![],
                return_type: None,
                decl_block: None,
                statements: vec![integer(1)],
            }))],
        }));
        assert_eq!(ast, expected);

        // Ignored block arg: { |_| 1; }
        // Visitor maps Ignored to name "_"
        let ast = parse("{ |_| 1; };");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![arc_node(NodeValue::Block(BlockNode {
                source_info: None,
                name: None,
                arguments: vec![block_arg("_", IdentifierType::Local, None)],
                decls: vec![],
                return_type: None,
                decl_block: None,
                statements: vec![integer(1)],
            }))],
        }));
        assert_eq!(ast, expected);

        // Instance variable arg: { |@x| 1; }
        let ast = parse("{ |@x| 1; };");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![arc_node(NodeValue::Block(BlockNode {
                source_info: None,
                name: None,
                arguments: vec![block_arg("x", IdentifierType::Instance, None)],
                decls: vec![],
                return_type: None,
                decl_block: None,
                statements: vec![integer(1)],
            }))],
        }));
        assert_eq!(ast, expected);

        // Untyped block decl: { | - x| 1; }
        let ast = parse("{ | - x| 1; };");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![arc_node(NodeValue::Block(BlockNode {
                source_info: None,
                name: None,
                arguments: vec![],
                decls: vec![block_decl("x", IdentifierType::Local, None)],
                return_type: None,
                decl_block: None,
                statements: vec![integer(1)],
            }))],
        }));
        assert_eq!(ast, expected);

        // Typed block decl: { | - x:Int| 1; }
        let ast = parse("{ | - x:Int| 1; };");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![arc_node(NodeValue::Block(BlockNode {
                source_info: None,
                name: None,
                arguments: vec![],
                decls: vec![block_decl(
                    "x",
                    IdentifierType::Local,
                    Some(flat_type("Int")),
                )],
                return_type: None,
                decl_block: None,
                statements: vec![integer(1)],
            }))],
        }));
        assert_eq!(ast, expected);

        // Decl block: { |x { 2 } - y| 1; }
        let ast = parse("{ |x { 2 } - y| 1; };");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![arc_node(NodeValue::Block(BlockNode {
                source_info: None,
                name: None,
                arguments: vec![block_arg("x", IdentifierType::Local, None)],
                decls: vec![block_decl("y", IdentifierType::Local, None)],
                return_type: None,
                decl_block: Some(Arc::new(BlockNode {
                    source_info: None,
                    name: None,
                    arguments: vec![],
                    decls: vec![],
                    return_type: None,
                    decl_block: None,
                    statements: vec![integer(2)],
                })),
                statements: vec![integer(1)],
            }))],
        }));
        assert_eq!(ast, expected);
    }

    /// A namespaced type node as `parse_type_ref` builds it: `[Web]Halt`, `[A/B]Gadget`,
    /// or the explicit root `[/]Thing` (empty path).
    fn ns_type(path: &[&str], name: &str) -> Arc<TypeRefNode> {
        Arc::new(TypeRefNode {
            args: Vec::new(),
            ident: ns_ident(path, name),
            ret: None,
            parenthesized: false,
        })
    }

    /// A generic type annotation node: `base(args…)`.
    fn generic_type(name: &str, args: Vec<Arc<TypeRefNode>>) -> Arc<TypeRefNode> {
        Arc::new(TypeRefNode {
            ident: ident_node(name, IdentifierType::Local),
            args,
            ret: None,
            parenthesized: true,
        })
    }

    /// A block type annotation node: `Block(args… ^ret)` (`ret` = None for no
    /// `^`-tail; `args` empty + no ret = `Block()`).
    fn block_type(args: Vec<Arc<TypeRefNode>>, ret: Option<Arc<TypeRefNode>>) -> Arc<TypeRefNode> {
        Arc::new(TypeRefNode {
            ident: ident_node("Block", IdentifierType::Local),
            args,
            ret,
            parenthesized: true,
        })
    }

    /// A flat (argument-less) type annotation node, as tests spell `x: Int`.
    fn flat_type(name: &str) -> Arc<TypeRefNode> {
        Arc::new(TypeRefNode {
            ident: ident_node(name, IdentifierType::Local),
            args: Vec::new(),
            ret: None,
            parenthesized: false,
        })
    }

    fn ns_ident(path: &[&str], name: &str) -> Arc<IdentifierNode> {
        Arc::new(IdentifierNode {
            source_info: None,
            namespace: Some(Arc::new(NamespaceNode {
                source_info: None,
                identifiers: path
                    .iter()
                    .map(|p| {
                        Arc::new(IdentifierNode {
                            source_info: None,
                            namespace: None,
                            name: p.to_string(),
                            identifier_type: IdentifierType::Local,
                        })
                    })
                    .collect(),
            })),
            name: name.to_string(),
            identifier_type: IdentifierType::Namespaced,
        })
    }

    fn block_with(
        arguments: Vec<Arc<BlockArgNode>>,
        return_type: Option<Arc<TypeRefNode>>,
        decls: Vec<Arc<BlockDeclNode>>,
    ) -> Arc<Node> {
        arc_node(NodeValue::Block(BlockNode {
            source_info: None,
            name: None,
            arguments,
            decls,
            return_type,
            decl_block: None,
            statements: vec![integer(1)],
        }))
    }

    #[test]
    fn test_parse_generic_type_annotations() {
        // `|l: List(Integer)|` — one generic argument.
        let ast = parse("{ |l: List(Integer)| 1; };");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![block_with(
                vec![block_arg(
                    "l",
                    IdentifierType::Local,
                    Some(generic_type("List", vec![flat_type("Integer")])),
                )],
                None,
                vec![],
            )],
        }));
        assert_eq!(ast, expected);

        // Nested, space-separated: `^Map(String List(Integer))`.
        let ast = parse("{ |^Map(String List(Integer))| 1; };");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![block_with(
                vec![],
                Some(generic_type(
                    "Map",
                    vec![
                        flat_type("String"),
                        generic_type("List", vec![flat_type("Integer")]),
                    ],
                )),
                vec![],
            )],
        }));
        assert_eq!(ast, expected);
    }

    #[test]
    fn test_parse_block_type_annotations() {
        // `|b: Block(Integer Integer ^Integer)|` — args plus the `^`-marked return.
        let ast = parse("{ |b: Block(Integer Integer ^Integer)| 1; };");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![block_with(
                vec![block_arg(
                    "b",
                    IdentifierType::Local,
                    Some(block_type(
                        vec![flat_type("Integer"), flat_type("Integer")],
                        Some(flat_type("Integer")),
                    )),
                )],
                None,
                vec![],
            )],
        }));
        assert_eq!(ast, expected);

        // `Block()` (zero args, parens present) ≠ bare `Block`; `Block(^Boolean)`
        // is return-only; block types nest as generic arguments and returns.
        let ast = parse(
            "{ |a: Block() b: Block(^Boolean) c: Block d: Block(List(Integer) ^Block(Integer ^Boolean))| 1; };",
        );
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![block_with(
                vec![
                    block_arg("a", IdentifierType::Local, Some(block_type(vec![], None))),
                    block_arg(
                        "b",
                        IdentifierType::Local,
                        Some(block_type(vec![], Some(flat_type("Boolean")))),
                    ),
                    block_arg("c", IdentifierType::Local, Some(flat_type("Block"))),
                    block_arg(
                        "d",
                        IdentifierType::Local,
                        Some(block_type(
                            vec![generic_type("List", vec![flat_type("Integer")])],
                            Some(block_type(
                                vec![flat_type("Integer")],
                                Some(flat_type("Boolean")),
                            )),
                        )),
                    ),
                ],
                None,
                vec![],
            )],
        }));
        assert_eq!(ast, expected);
    }

    #[test]
    fn test_parse_class_header_type_params() {
        // `Iterate(T U) <- { 1 };` — declared type variables on the header.
        let ast = parse("Iterate(T U) <- { 1 };");
        let NodeValue::Program(p) = &ast.value else {
            panic!("not a program");
        };
        let NodeValue::ClassDefinition(cd) = &p.expressions[0].value else {
            panic!("not a class definition");
        };
        assert_eq!(cd.identifier.name, "Iterate");
        assert_eq!(cd.type_params, vec!["T".to_string(), "U".to_string()]);

        // With a parent: `Mixin <- Iterate(T) <- { 1 };`
        let ast = parse("Mixin <- Iterate(T) <- { 1 };");
        let NodeValue::Program(p) = &ast.value else {
            panic!("not a program");
        };
        let NodeValue::ClassDefinition(cd) = &p.expressions[0].value else {
            panic!("not a class definition");
        };
        assert_eq!(cd.identifier.name, "Iterate");
        assert_eq!(cd.parent_identifier.as_ref().unwrap().name, "Mixin");
        assert_eq!(cd.type_params, vec!["T".to_string()]);

        // Plain definitions carry no params.
        let ast = parse("Plain <- { 1 };");
        let NodeValue::Program(p) = &ast.value else {
            panic!("not a program");
        };
        let NodeValue::ClassDefinition(cd) = &p.expressions[0].value else {
            panic!("not a class definition");
        };
        assert!(cd.type_params.is_empty());
    }

    #[test]
    fn test_parse_namespaced_type_annotations() {
        // Typed block arg: { |e:[Web]Halt| 1; }
        let ast = parse("{ |e:[Web]Halt| 1; };");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![block_with(
                vec![Arc::new(BlockArgNode {
                    identifier: ident_node("e", IdentifierType::Local),
                    type_hint: Some(ns_type(&["Web"], "Halt")),
                })],
                None,
                vec![],
            )],
        }));
        assert_eq!(ast, expected);

        // Block return type: { |x ^[Web]Halt| 1; } — with a nullable multi-segment
        // variant ([A/B]Gadget?) on the block-local side below.
        let ast = parse("{ |x ^[Web]Halt| 1; };");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![block_with(
                vec![block_arg("x", IdentifierType::Local, None)],
                Some(ns_type(&["Web"], "Halt")),
                vec![],
            )],
        }));
        assert_eq!(ast, expected);

        // Typed block-local, multi-segment namespace + nullable: { | - g:[A/B]Gadget?| 1; }
        let ast = parse("{ | - g:[A/B]Gadget?| 1; };");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![block_with(
                vec![],
                None,
                vec![Arc::new(BlockDeclNode {
                    identifier: ident_node("g", IdentifierType::Local),
                    type_hint: Some(ns_type(&["A", "B"], "Gadget?")),
                })],
            )],
        }));
        assert_eq!(ast, expected);

        // Explicit-root namespace in type position: { |x:[/]Thing| 1; } (empty path).
        let ast = parse("{ |x:[/]Thing| 1; };");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![block_with(
                vec![Arc::new(BlockArgNode {
                    identifier: ident_node("x", IdentifierType::Local),
                    type_hint: Some(ns_type(&[], "Thing")),
                })],
                None,
                vec![],
            )],
        }));
        assert_eq!(ast, expected);

        // Typed declaration: var x: [IO]File = 1;
        let ast = parse("var x: [IO]File = 1;");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![arc_node(NodeValue::Declaration(DeclarationNode {
                kind: DeclKind::Var,
                lvalues: vec![arc_node(NodeValue::IdentLValue(IdentLValueNode {
                    identifier: ident_node("x", IdentifierType::Local),
                }))],
                type_hint: Some(ns_type(&["IO"], "File")),
                rvalue: integer(1),
            }))],
        }));
        assert_eq!(ast, expected);

        // A bare type still parses exactly as before (namespace: None).
        let ast = parse("{ |x:Integer| 1; };");
        let expected = val_node(NodeValue::Program(ProgramNode {
            source_info: None,
            allow_pragmas: Vec::new(),
            expressions: vec![block_with(
                vec![block_arg(
                    "x",
                    IdentifierType::Local,
                    Some(flat_type("Integer")),
                )],
                None,
                vec![],
            )],
        }));
        assert_eq!(ast, expected);
    }

    #[test]
    fn test_line_offset_table() {
        let text = "line 1\nline 2\n💖 line 3\n\nline 5";
        let table = super::LineOffsetTable::new(text);

        // "line 1" is on line 1.
        assert_eq!(table.find_line_col(0, text), (1, 1)); // 'l'
        assert_eq!(table.find_line_col(5, text), (1, 6)); // '1'
        assert_eq!(table.find_line_col(6, text), (1, 7)); // '\n'

        // "line 2" is on line 2.
        assert_eq!(table.find_line_col(7, text), (2, 1)); // 'l'
        assert_eq!(table.find_line_col(13, text), (2, 7)); // '\n'

        // "💖 line 3" is on line 3.
        assert_eq!(table.find_line_col(14, text), (3, 1)); // '💖' (UTF-8 index starts at 14)
        assert_eq!(table.find_line_col(18, text), (3, 2)); // ' ' (UTF-8 💖 is 4 bytes, so space is at 18)
        assert_eq!(table.find_line_col(19, text), (3, 3)); // 'l'
        assert_eq!(table.find_line_col(25, text), (3, 9)); // '\n'

        // Empty line 4.
        assert_eq!(table.find_line_col(26, text), (4, 1)); // '\n'

        // "line 5" is on line 5.
        assert_eq!(table.find_line_col(27, text), (5, 1)); // 'l'
    }
}

#[cfg(test)]
#[path = "parser_tests.rs"]
mod parser_tests;
