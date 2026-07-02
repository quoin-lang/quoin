//! Throwaway migration: add `var` at the first binding of each local (per scope)
//! so `.qn` code stops relying on the retired first-assignment-declares rule.
//!
//! Scope model mirrors the compiler: program scope holds `self`; each block
//! introduces a scope pre-populated with its params, decls, and `self`; a name is
//! "declared" if it appears in ANY enclosing scope (closures see outer locals).
//! Only assignment *targets* bind locals (reads of undeclared names are globals),
//! so we track targets + block scopes and ignore reads.
//!
//! An assignment whose targets are all fresh becomes a `var`; a pure reassignment
//! is left alone; a *mixed* one (some fresh, some existing — only possible with
//! destructuring) is reported for manual splitting. The strict compiler is the
//! backstop: anything mis-migrated surfaces as an "undeclared local" error.
//!
//! Usage: `decl-migrate <file.qn>...` — edits each file in place.

use quoin_syntax::ast::*;
use quoin_syntax::parse_quoin_string_named;
use std::collections::HashSet;
use std::sync::Arc;

struct Ctx {
    scopes: Vec<HashSet<String>>,
    inserts: Vec<usize>,
    mixed: Vec<usize>,
}

impl Ctx {
    fn is_declared(&self, name: &str) -> bool {
        self.scopes.iter().any(|s| s.contains(name))
    }
    fn declare(&mut self, name: &str) {
        self.scopes.last_mut().unwrap().insert(name.to_string());
    }
}

fn is_plain_local(id: &IdentifierNode) -> bool {
    id.namespace.is_none()
        && id.identifier_type != IdentifierType::Namespaced
        && id.identifier_type != IdentifierType::Instance
        // Uppercase-initial names are globals/classes/consts (compiled to StoreGlobal),
        // not locals — never declared with `var`/`let`.
        && !id
            .name
            .chars()
            .next()
            .map(|c| c.is_ascii_uppercase())
            .unwrap_or(false)
        // Reserved words parse as lvalues (in negative tests like `true = false`) but are
        // not locals — never prefix them with `var`.
        && id.identifier_type != IdentifierType::ReservedIdentifier
        && !matches!(id.name.as_str(), "true" | "false" | "nil")
}

fn collect_target_names(lvalues: &[Arc<Node>], out: &mut Vec<String>) {
    for lv in lvalues {
        match &lv.value {
            NodeValue::IdentLValue(l) if is_plain_local(&l.identifier) => {
                out.push(l.identifier.name.clone())
            }
            NodeValue::SplatLValue(l) if is_plain_local(&l.identifier) => {
                out.push(l.identifier.name.clone())
            }
            NodeValue::SubLValue(s) => collect_target_names(&s.lvalues, out),
            _ => {}
        }
    }
}

fn block_scope_names(b: &BlockNode) -> HashSet<String> {
    let mut s = HashSet::new();
    s.insert("self".to_string());
    for a in &b.arguments {
        if is_plain_local(&a.identifier) {
            s.insert(a.identifier.name.clone());
        }
    }
    for d in &b.decls {
        if is_plain_local(&d.identifier) {
            s.insert(d.identifier.name.clone());
        }
    }
    s
}

fn walk_block(b: &BlockNode, ctx: &mut Ctx) {
    ctx.scopes.push(block_scope_names(b));
    if let Some(db) = &b.decl_block {
        walk_block(db, ctx);
    }
    for s in &b.statements {
        walk(s, ctx);
    }
    ctx.scopes.pop();
}

fn walk(node: &Node, ctx: &mut Ctx) {
    match &node.value {
        NodeValue::Assignment(a) => {
            // Evaluate the rhs first (nested blocks/assignments), then bind targets.
            walk(&a.rvalue, ctx);
            let mut names = Vec::new();
            collect_target_names(&a.lvalues, &mut names);
            if names.is_empty() {
                return;
            }
            let new_count = names.iter().filter(|n| !ctx.is_declared(n)).count();
            if new_count == names.len() {
                if let Some(si) = &node.source_info {
                    ctx.inserts.push(si.start);
                }
                for n in &names {
                    ctx.declare(n);
                }
            } else if new_count > 0 {
                if let Some(si) = &node.source_info {
                    ctx.mixed.push(si.start);
                }
                for n in &names {
                    if !ctx.is_declared(n) {
                        ctx.declare(n);
                    }
                }
            }
        }
        NodeValue::Block(b) => walk_block(b, ctx),
        NodeValue::ClassDefinition(c) => walk_block(&c.block, ctx),
        NodeValue::ClassExtension(c) => {
            walk(&c.expression, ctx);
            walk_block(&c.block, ctx);
        }
        NodeValue::MethodDefinition(m) => walk_block(&m.block, ctx),
        NodeValue::MethodExtension(m) => walk_block(&m.block, ctx),
        NodeValue::ConstDefinition(c) => walk(&c.rvalue, ctx),
        NodeValue::Map(m) => {
            for k in &m.keys {
                walk(k, ctx);
            }
            for v in &m.values {
                walk(v, ctx);
            }
        }
        NodeValue::List(l) => {
            for v in &l.values {
                walk(v, ctx);
            }
        }
        NodeValue::Set(s) => {
            for v in &s.values {
                walk(v, ctx);
            }
        }
        NodeValue::UserList(u) => {
            for v in &u.values {
                walk(v, ctx);
            }
        }
        NodeValue::MethodCall(c) => {
            if let Some(subj) = &c.subject {
                walk(subj, ctx);
            }
            for e in &c.arguments.expressions {
                walk(e, ctx);
            }
        }
        NodeValue::BinaryOperator(b) => {
            walk(&b.left, ctx);
            walk(&b.right, ctx);
        }
        NodeValue::UnaryOperator(u) => walk(&u.right, ctx),
        NodeValue::BlockReturn(r) => walk(&r.value, ctx),
        NodeValue::MethodReturn(r) => walk(&r.value, ctx),
        NodeValue::YieldReturn(r) => walk(&r.value, ctx),
        NodeValue::Program(p) => {
            for e in &p.expressions {
                walk(e, ctx);
            }
        }
        _ => {}
    }
}

fn line_of(src: &str, off: usize) -> usize {
    src[..off.min(src.len())].bytes().filter(|&b| b == b'\n').count() + 1
}

fn main() {
    let files: Vec<String> = std::env::args().skip(1).collect();
    if files.is_empty() {
        eprintln!("usage: decl-migrate <file.qn>...");
        std::process::exit(2);
    }
    let mut total_inserts = 0usize;
    let mut total_mixed = 0usize;
    for path in &files {
        let src = std::fs::read_to_string(path).expect("read");
        let ast = parse_quoin_string_named(&src, path);
        let mut ctx = Ctx {
            scopes: vec![{
                let mut s = HashSet::new();
                s.insert("self".to_string());
                s
            }],
            inserts: Vec::new(),
            mixed: Vec::new(),
        };
        walk(&ast, &mut ctx);

        for &off in &ctx.mixed {
            eprintln!("MIXED (manual): {}:{}", path, line_of(&src, off));
        }
        total_mixed += ctx.mixed.len();

        if ctx.inserts.is_empty() {
            continue;
        }
        // Apply insertions right-to-left so earlier offsets stay valid.
        let mut offs = ctx.inserts.clone();
        offs.sort_unstable();
        offs.dedup();
        let mut out = src.clone();
        for &off in offs.iter().rev() {
            out.insert_str(off, "var ");
        }
        std::fs::write(path, &out).expect("write");
        total_inserts += offs.len();
        println!("{}: +{} var", path, offs.len());
    }
    eprintln!(
        "\nDONE: {} var insertions across {} files; {} mixed sites need manual review.",
        total_inserts,
        files.len(),
        total_mixed
    );
}
