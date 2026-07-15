//! Portable-block classification (the IDE surface) and the boundary-crossing
//! warning. Both run the REAL boundary scan (`crate::worker::scan_portable`)
//! over the literal's compiled template — the same function the ship-time
//! check runs — so the editor can never disagree with the boundary. Shape
//! rules are definitive at compile time; capture VALUES are ship-time, so
//! their static types sharpen the verdict best-effort — the gradual
//! checker's stance throughout (TYPE_SYSTEM_ARCH.md).

use super::*;

/// One block literal's classification, keyed by its source span (the same
/// keying `qn test --coverage` uses). Collected only under
/// [`Compiler::with_portability`] (`qn check`, the language server) — a plain
/// `qn run` skips the second bytecode walk per literal.
#[derive(Clone, Debug)]
pub struct BlockPortability {
    pub filename: String,
    /// Byte offsets of the whole `{ … }` literal.
    pub start: usize,
    pub end: usize,
    /// 1-indexed position of the literal's opening brace.
    pub line: usize,
    pub column: usize,
    pub state: Portability,
    /// `NonPortable` only: the refusal, verbatim from the boundary scan — or
    /// the never-encodable capture's description. Empty otherwise.
    pub reason: String,
    /// `Conditional` only: captures whose static types the checker couldn't
    /// pin to the portable set — the literal ships iff these hold portable
    /// values at snapshot time.
    pub unknown_captures: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Portability {
    /// Shape-clean, and every capture's static type is data-encodable (or
    /// there are no captures at all): ships, unconditionally.
    Portable,
    /// Shape-clean, but some captures' types are unknown — or Block-typed: a
    /// captured block ships recursively, so its portability is its runtime
    /// value's.
    Conditional,
    /// A shape violation, or a capture whose static type can never encode:
    /// shipping would always refuse.
    NonPortable,
}

impl Portability {
    /// The state slug shared by `qn check --json` and the LSP's semantic
    /// token modifiers.
    pub fn slug(self) -> &'static str {
        match self {
            Portability::Portable => "portable",
            Portability::Conditional => "conditional",
            Portability::NonPortable => "non-portable",
        }
    }
}

/// Can a value of this static type cross as wire data? Mirrors
/// `value_to_wire`'s accepting arms: scalars, String, Bytes, big numerics,
/// and List/Map of portable elements. Note the deliberate absences: Sets do
/// not cross, Symbols do not cross, and any other instance type refuses.
enum TypeShips {
    Always,
    Unknown,
    Never(String),
}

fn type_ships(t: &Type) -> TypeShips {
    match t {
        Type::Int | Type::Double | Type::Bool | Type::String | Type::Nil => TypeShips::Always,
        Type::Nullable(inner) => type_ships(inner),
        Type::ListOf(elem) | Type::MapOf(elem) => match type_ships(elem) {
            TypeShips::Always => TypeShips::Always,
            TypeShips::Unknown => TypeShips::Unknown,
            TypeShips::Never(why) => TypeShips::Never(why),
        },
        // Untagged collections: the elements decide, and nothing is known.
        Type::List | Type::Map => TypeShips::Unknown,
        Type::Instance(name) => match &**name {
            "Bytes" | "BigInteger" | "BigDecimal" => TypeShips::Always,
            other => TypeShips::Never(other.to_string()),
        },
        // A captured block ships recursively; its portability is the runtime
        // value's own classification.
        Type::Block | Type::BlockOf { .. } => TypeShips::Unknown,
        Type::Set | Type::SetOf(_) => TypeShips::Never("Set".to_string()),
        Type::Any | Type::Var(_) | Type::Never => TypeShips::Unknown,
    }
}

impl Compiler {
    /// `compile_method_call`'s pre-pass (a pure AST analysis, beside
    /// `check_mnu`): a block LITERAL in the shipped position of a statically
    /// recognizable boundary send — `Worker.with:` / `Worker.host:with:` /
    /// `Worker.start:` and their keyword extensions — registers its node here;
    /// `classify_block_literal` scans it once its template exists and warns if
    /// the shape can never cross.
    pub(super) fn note_boundary_send(&mut self, call: &MethodCallNode) {
        let Some(subject) = &call.subject else { return };
        let NodeValue::Identifier(id) = &subject.value else {
            return;
        };
        // No shadow check needed: locals cannot be uppercase, and the builtin
        // global is runtime-sealed — `Worker` is the boundary, best-effort.
        if id.name != "Worker" {
            return;
        }
        let idents = &call.arguments.signature.identifiers;
        let exprs = &call.arguments.expressions;
        let block_pos = match idents.first().map(|i| i.name.as_str()) {
            Some("with") | Some("start") => 0,
            Some("host") if idents.get(1).is_some_and(|i| i.name == "with") => 1,
            _ => return,
        };
        let Some(arg) = exprs.get(block_pos) else {
            return;
        };
        if let NodeValue::Block(b) = &arg.value {
            let selector: String = idents.iter().map(|i| format!("{}:", i.name)).collect();
            self.boundary_block_literals
                .insert(b as *const BlockNode as usize, selector);
        }
    }

    /// `compile_block`'s tail hook, called once the literal's template exists
    /// and the ENCLOSING scope is active again (so capture names resolve to
    /// the types they have at the literal's position). Does two jobs:
    ///
    /// - The boundary warning (always on): a literal registered by
    ///   [`note_boundary_send`] whose shape can never cross warns with the
    ///   scan's reason — the ship-time error, moved to edit time.
    /// - Classification (under [`Compiler::with_portability`] only): the
    ///   three-state verdict for the IDE, span-keyed.
    pub(super) fn classify_block_literal(&mut self, node_key: usize, template: &Arc<StaticBlock>) {
        let boundary_selector = self.boundary_block_literals.remove(&node_key);
        if boundary_selector.is_none() && !self.collect_portability {
            return;
        }
        let scan = crate::worker::scan_portable(template);
        if let (Some(selector), Err(why)) = (&boundary_selector, &scan) {
            self.warn(
                "portability",
                format!(
                    "this block crosses an isolate boundary (Worker.{selector}) but is not \
                     portable: {why}"
                ),
                template.source_info.as_ref(),
            );
        }
        if !self.collect_portability {
            return;
        }
        let Some(si) = template.source_info.as_ref() else {
            return;
        };
        let (state, reason, unknown_captures) = match &scan {
            Err(why) => (Portability::NonPortable, why.clone(), Vec::new()),
            Ok((frees, _globals)) => {
                let mut unknown = Vec::new();
                let mut never: Option<(String, String)> = None;
                for sym in frees {
                    let name = sym.as_str();
                    match type_ships(&self.local_type(name)) {
                        TypeShips::Always => {}
                        TypeShips::Unknown => unknown.push(name.to_string()),
                        TypeShips::Never(ty) => {
                            if never.is_none() {
                                never = Some((name.to_string(), ty));
                            }
                        }
                    }
                }
                match never {
                    Some((name, ty)) => (
                        Portability::NonPortable,
                        format!("capture '{name}' holds a {ty} — not portable data"),
                        Vec::new(),
                    ),
                    None if !unknown.is_empty() => {
                        (Portability::Conditional, String::new(), unknown)
                    }
                    None => (Portability::Portable, String::new(), Vec::new()),
                }
            }
        };
        self.block_portability.push(BlockPortability {
            filename: si.filename.clone(),
            start: si.start,
            end: si.end,
            line: si.line,
            column: si.column,
            state,
            reason,
            unknown_captures,
        });
    }
}
