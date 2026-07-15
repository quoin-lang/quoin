use crate::class_table::{ClassSig, ClassTable};
use crate::instruction::{
    Constant, Instruction, IntBinKind, SharedBytecode, SharedSourceMap, StaticBlock,
};
use crate::parser::ast::{
    AssignmentNode, BinaryOperatorNode, BinaryOperatorType, BlockNode, ClassDefinitionNode,
    DeclKind, DeclarationNode, IdentifierNode, IdentifierType, MethodCallNode, MethodSelectorNode,
    Node, NodeValue, ProgramNode, StringNode, TypeRefNode, UnaryOperatorNode, UnaryOperatorType,
};
use crate::parser::interp::{InterpPart, split_interpolation};
use crate::runtime::elem_tag::ElemTag;
use crate::symbol::Symbol;
use crate::types::{SeenTypes, Type};
use crate::value::{NamespacedName, SourceInfo};

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

mod annotations;
mod assignment;
mod bytecode;
mod checker;
mod lowering;
mod portability;
mod scope;

// Items that moved into satellites but are referenced across the module (children see
// these private re-imports through their `use super::*`).
use annotations::{ident_name, type_from_ref_with_vars};
use bytecode::{CodeBlock, fuse_bytecode, set_jump_offset};
use checker::{NarrowKey, TypeProvenance};
pub use portability::{BlockPortability, Portability};
use scope::Scope;
mod class_info;
mod devirt;
mod inlining;

/// Compile-time context for the class body currently being compiled (Slice 2b). Pushed
/// on a stack while compiling a class def / extension; used to type self-sends by their
/// callee's declared return type and to devirtualize self-sends in a sealed class.
struct ClassCtx {
    /// Unique id for this class-body/extension context within the process — the
    /// AOT sibling-group key (self-calls compile to direct calls only among
    /// methods that share it, i.e. the same method table + receiver shape).
    id: u32,
    /// The class name for a named definition/extension; empty for anonymous
    /// extension targets (e.g. `.meta <-- { … }`), whose sealedness is looked up
    /// through to the nearest named enclosing context.
    name: String,
    /// Selectors defined more than once in this body (typed multimethod
    /// variants) — excluded from AOT candidacy, since a direct call would bypass
    /// the runtime variant scoring.
    multi: HashSet<String>,
    /// Class/mixin-header type parameters (`Iterate(T U)`) — the type variables
    /// this body's method signatures may use (checker-only).
    type_params: Vec<String>,
    /// Method selector → declared return `Type` (methods that annotate a return).
    returns: HashMap<String, Type>,
    /// The class is compile-sealed: `sealed!` appears as a direct (unconditional) body
    /// statement, so its method table is frozen and same-class self-sends can be
    /// devirtualized (Slice 2b-B).
    sealed: bool,
    /// Selector → method body, for inlining a leaf self-send (Phase 5·1). A sealed class can't be
    /// subclassed, so `self.foo` provably resolves to this class's own `foo`; a trivial body is
    /// spliced at the call site instead of dispatched.
    bodies: HashMap<String, Arc<BlockNode>>,
}

/// The warning taxonomy: every checker diagnostic carries one of these stable kind slugs,
/// and a trailing `"* allow: <kind>` comment on the warned line suppresses it. The names are
/// user-facing contract (they appear in pragmas and in the unknown-kind message) — renaming
/// one breaks existing suppressions.
pub const WARNING_KINDS: &[&str] = &[
    "allow-pragma", // a malformed `allow:` pragma itself (unknown kind, no kind, not trailing)
    "annotation",   // type-annotation shape: generic arity, checker-only nesting…
    "caret-discard", // `^` ends a discarded `if:`/`else:` arm — control falls through
    "element-type", // a typed collection rejects an element (literal or insert)
    "key-type",     // a `Map(K V)` key position gets an off-`K` key (checker-only belief)
    "mnu",          // the receiver's class does not respond to the selector
    "nil-receiver", // non-nil-safe send (or operator operand) on a maybe-nil value
    "no-variant",   // no multimethod variant accepts the argument types
    "portability",  // a block literal at an isolate boundary cannot cross (the ship-time scan)
    "return-type",  // an override's return is incompatible with the inherited return
    "type-mismatch", // a value's type contradicts the declared/expected type
    "unknown-type", // an annotation names a type the checker has never seen
];

/// A non-fatal type diagnostic: the message plus the source span it points at, for `path:line:col`
/// rendering (Phase 4). `span` is `None` when a check can't attribute a precise location.
#[derive(Clone, Debug)]
pub struct Diagnostic {
    /// The [`WARNING_KINDS`] slug this diagnostic belongs to — the handle an
    /// `"* allow: <kind>` pragma suppresses it by.
    pub kind: &'static str,
    pub message: String,
    pub span: Option<SourceInfo>,
    /// Secondary "why-chain" notes (Phase 4 provenance): e.g. where a variable got the type that
    /// caused this diagnostic. Rendered indented under the main message, each at its own span.
    pub notes: Vec<Note>,
}

/// A secondary note attached to a [`Diagnostic`] — a message plus the span it points at.
#[derive(Clone, Debug)]
pub struct Note {
    pub message: String,
    pub span: Option<SourceInfo>,
}

/// A fatal compile error: the message plus the source span it points at, so every entry point
/// can render `file:line:col` like a [`Diagnostic`] (at `error` level) instead of a bare string.
/// `span` is `None` only when the failing site has no attributable location. `Display` embeds
/// the location in parentheses — the form for plain-string contexts (`Runtime.eval:`, worker
/// failures, the REPL), matching how those modes already render parse errors; file-based modes
/// render richly via `VmState::report_compile_error`.
#[derive(Clone, Debug)]
pub struct CompileError {
    pub message: String,
    pub span: Option<SourceInfo>,
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.span {
            Some(s) => write!(
                f,
                "{} (line {}, column {})",
                self.message,
                s.line,
                s.column + 1
            ),
            None => write!(f, "{}", self.message),
        }
    }
}

pub struct Compiler {
    scopes: Vec<Scope>,
    temp_counter: usize,
    /// >0 while compiling the body of a `<-`/`<--` block whose target is an
    /// > immediate value type (Integer/Double/Boolean/Nil). Instance variables are
    /// > rejected there so the "value types have no fields" rule surfaces at compile
    /// > time rather than only when a method runs.
    value_type_def_depth: usize,
    /// One-shot flag set right before compiling the block argument of `X.new:{ … }`;
    /// consumed by the next `compile_block` to mark that block's scope `is_init`.
    next_block_is_init: bool,
    /// One-shot narrowing set right before compiling a guard arm block (Phase 3c); the next
    /// `compile_block` installs it into that arm's scope. Mirrors `next_block_is_init`.
    next_block_narrowing: Option<(NarrowKey, Type)>,
    /// One-shot: the declared param types of the `Block(…)`-typed parameter the next block
    /// literal is being passed to, receiver-bound (G4b, GENERICS_ARCH.md §11.3). The literal's
    /// `compile_block` seeds its UNANNOTATED params from these as narrowing-grade beliefs —
    /// never contracts, never devirt.
    next_block_expected: Option<Vec<Type>>,
    /// Sharpened static types of compiled block literals (`Block(args ^Ret)` shapes), keyed by
    /// `BlockNode` address — how `static_type` sees a literal after its body compiled; before
    /// that it is bare `Block` (gradual, a safe miss). Lives only for this compile.
    block_literal_types: HashMap<usize, Type>,
    /// Per-`compile_block` accumulator for the body's ACTUAL return type: the join of the tail
    /// expression and every real (non-inlined) `^` return — `^^` diverges the block and
    /// contributes nothing. Innermost last; drives block-return inference (§11.3).
    block_ret_harvest: Vec<Type>,
    /// Portable-block classification (`portability.rs`): collected only when
    /// [`with_portability`](Self::with_portability) opted in (`qn check`, the
    /// language server) — a plain run skips the per-literal scan.
    collect_portability: bool,
    /// The collected classifications, span-keyed, in compile order.
    block_portability: Vec<BlockPortability>,
    /// One-shot: the next `compile_block` compiles an EXPRESSION-position
    /// literal (a block VALUE — set at the `NodeValue::Block` arm). Method,
    /// class, and guard-decl bodies also flow through `compile_block` but are
    /// not shippable values, so only expression literals classify for
    /// portability (an all-italic class file would be exactly the noise the
    /// whole-block tint is meant to avoid).
    next_block_is_expression: bool,
    /// Block literals in the shipped position of a boundary send
    /// (`Worker.with:`/`host:with:`/`start:`), keyed by `BlockNode` address —
    /// registered by `note_boundary_send`, consumed by `classify_block_literal`
    /// to warn at compile time when the shape can never cross. Always on.
    boundary_block_literals: HashMap<usize, String>,
    /// One-shot request set right before compiling a guard arm block: the key whose narrowed type
    /// the arm's `compile_block` should snapshot at exit (into `captured_arm_exit`) so the join at
    /// the conditional's end can merge the arms' exit states (Phase 3c join/merge).
    next_block_capture: Option<NarrowKey>,
    /// The exit narrowing captured for `next_block_capture`'s key by the most recent arm
    /// `compile_block` — read by `compile_method_call` right after the arm compiles.
    captured_arm_exit: Option<Type>,
    /// Current self-send inlining nesting depth (Phase 5·2), bounded by `MAX_INLINE_DEPTH`.
    inline_depth: usize,
    /// `class name → (selector → method body)` for every class compiled so far in this unit, so an
    /// explicit-receiver `v.x` can inline against *any* in-unit sealed class, not just the one being
    /// compiled (Phase 5·3b). Backward references only; cross-unit bodies aren't available as AST.
    class_bodies: HashMap<String, HashMap<String, Arc<BlockNode>>>,
    /// While splicing a computed method body at an *explicit-receiver* call site (Phase 5·3c), the
    /// local holding the receiver: `self`/`@field`/implicit self-sends in the body are rebound to it
    /// (`self` was the callee `v`, not the caller). `None` outside such a splice.
    self_override: Option<Symbol>,
    /// While splicing a body that has parameters (Phase 5·4): each param name → the temp holding its
    /// argument, so a bare param reference in the body loads that temp. Saved/restored across nesting.
    param_override: HashMap<String, Symbol>,
    /// Monotonic mint counter for splice alpha-renames (Slice 2d v2): `name·N`. Never reset —
    /// uniqueness must hold across every splice site in the compile, not per method.
    splice_rename_counter: u32,
    /// Depth of enclosing PER-ITERATION splices (fused `whileDo:` cond/body, fused `each:`
    /// body) at the current compile point. >0 means a declaring arm spliced here re-executes
    /// in the same frame, so its declarations rebind one cell instead of minting per-execution
    /// generations — observable only if a surviving closure captures one (the v2 hazard check).
    fused_loop_depth: u32,
    /// Stack of per-class compile context, pushed while compiling a class body: method
    /// return types (Slice 2b-A) + the method set + whether the class is sealed (2b-B).
    class_ctx: Vec<ClassCtx>,
    /// Whether this unit's top-level `self` is the nil default (see `compile_program_with`).
    top_level_self_is_nil: bool,
    /// While compiling an *inlined* control-flow block body (Slice 2d), collects the
    /// bytecode positions of top-level `^` (BlockReturn) placeholder jumps so
    /// `inline_block_body` can patch them to land just past the inlined region. `None`
    /// outside inlining (a `^` then compiles to a normal `BlockReturn`). Cleared on entry
    /// to a real nested block so its `^` isn't captured by an enclosing inlined region.
    inline_carets: Option<Vec<usize>>,
    /// Class names known so far — builtins + classes defined by earlier-compiled units +
    /// this program's own defs (seeded by `prescan_class_defs`). Shared across units so a
    /// later unit sees earlier ones' classes; consulted by `resolve_annotation` (Phase 2).
    seen_types: SeenTypes,
    /// Shared class-signature table for the cross-class checks (Phase 3b) — parallel to
    /// `seen_types`, populated from the current unit's AST and threaded across units.
    class_table: ClassTable,
    /// Non-fatal type diagnostics (e.g. `unknown type Foo`) collected during compilation.
    /// Surfaced by the caller; never blocks lowering (gradual best-effort).
    diagnostics: Vec<Diagnostic>,
    /// The current unit's validated `"* allow: <kind>` suppressions, line → kinds
    /// (trailing pragmas with known kind names only — `install_allow_pragmas`).
    /// `warn_with_notes` drops a diagnostic whose span line carries its kind.
    allow_pragmas: HashMap<usize, Vec<String>>,
    /// The span of a fatal compile error, claimed innermost-first: an error site that knows a
    /// precise span records it via `err_at`; otherwise the innermost `compile_node` frame to see
    /// the error fills in its statement-level span. Taken by `compile_program_with` to build the
    /// returned [`CompileError`].
    error_span: Option<SourceInfo>,
    /// Declared return types of the block(s) currently being compiled (`|args ^T|`), innermost
    /// last. A `^`/`^^` return or a block's tail expression is checked (and numeric literals
    /// promoted) against the top entry; `None` = no declared return → not checked. Phase 3a.
    return_type_stack: Vec<Option<Type>>,
    /// Collect AOT candidates (docs/internal/AOT_ARCH.md) while compiling: methods of
    /// sealed classes with all-scalar params and return. Only the runner's
    /// once-per-unit compiles opt in, and only when `QN_AOT=1`.
    collect_aot: bool,
    aot_candidates: Vec<crate::codegen::AotCandidate>,
    /// Source of `ClassCtx::id`.
    class_ctx_counter: u32,
    /// Mint a `template_id` for every compiled block literal, so all its closures
    /// share one inline-cache array (`VmState::ic_registry`). Only the runner's
    /// once-per-unit compiles opt in (`with_template_ids`); eval/REPL/interpolation
    /// compile per evaluation, and per-compile ids would grow the registry without
    /// bound. Default: off.
    mint_template_ids: bool,
}

impl Default for Compiler {
    fn default() -> Self {
        Self::new()
    }
}

impl Compiler {
    pub fn new() -> Self {
        Self {
            scopes: vec![Scope {
                locals: HashSet::new(),
                immutable: HashSet::new(),
                types: HashMap::new(),
                declared_types: HashMap::new(),
                narrowed: HashMap::new(),
                provenance: HashMap::new(),
                is_init: false,
                renaming: false,
                renames: HashMap::new(),
            }],
            temp_counter: 0,
            value_type_def_depth: 0,
            next_block_is_init: false,
            next_block_narrowing: None,
            next_block_expected: None,
            block_literal_types: HashMap::new(),
            block_ret_harvest: Vec::new(),
            collect_portability: false,
            block_portability: Vec::new(),
            next_block_is_expression: false,
            boundary_block_literals: HashMap::new(),
            next_block_capture: None,
            captured_arm_exit: None,
            inline_depth: 0,
            class_bodies: HashMap::new(),
            self_override: None,
            param_override: HashMap::new(),
            splice_rename_counter: 0,
            fused_loop_depth: 0,
            class_ctx: Vec::new(),
            top_level_self_is_nil: false,
            inline_carets: None,
            seen_types: SeenTypes::with_builtins(),
            class_table: ClassTable::new(),
            diagnostics: Vec::new(),
            allow_pragmas: HashMap::new(),
            error_span: None,
            return_type_stack: Vec::new(),
            mint_template_ids: false,
            collect_aot: false,
            aot_candidates: Vec::new(),
            class_ctx_counter: 0,
        }
    }

    /// Opt this compile into template-id minting (shared inline-cache arrays).
    pub fn with_template_ids(mut self) -> Self {
        self.mint_template_ids = true;
        self
    }

    /// Opt this compile into portable-block classification (`portability.rs`):
    /// every block literal gets a three-state verdict from the real boundary
    /// scan, read back via [`block_portability`](Self::block_portability).
    /// `qn check` and the language server turn this on; plain runs skip the
    /// extra per-literal bytecode walk.
    pub fn with_portability(mut self) -> Self {
        self.collect_portability = true;
        self
    }

    /// AOT candidacy (docs/internal/AOT_ARCH.md §3): a method of a sealed class whose
    /// params and return are all scalar, unguarded, and single-variant. The
    /// authoritative bytecode walk happens in `codegen::translate` — this is the
    /// cheap proof-of-eligibility filter; refusal there is silent and safe.
    /// Sealedness looks through anonymous extension contexts (`.meta <-- {…}`)
    /// to the nearest named enclosing class body: the class body (including its
    /// meta extension) runs to completion — sealing both tables — before any
    /// external caller can dispatch, the same argument Phase 5 inlining trusts.
    fn maybe_collect_aot_candidate(
        &mut self,
        selector: &str,
        block_node: &BlockNode,
        bytecode: &CodeBlock,
    ) {
        if !self.collect_aot || !self.mint_template_ids {
            return;
        }
        let Some(imm) = self.class_ctx.last() else {
            return;
        };
        let sealed = imm.sealed
            || self
                .class_ctx
                .iter()
                .rev()
                .find(|c| !c.name.is_empty())
                .is_some_and(|c| c.sealed);
        if imm.multi.contains(selector) {
            crate::codegen::record_refusal(
                selector,
                crate::codegen::RefusalKind::PrecheckMultiVariant,
                "multi-variant (typed multimethod) selector",
            );
            return;
        }
        if block_node.decl_block.is_some() {
            crate::codegen::record_refusal(
                selector,
                crate::codegen::RefusalKind::PrecheckDeclBlock,
                "method has a guard/decl block",
            );
            return;
        }
        // B2 (docs/internal/BLOCK_AOT_ARCH.md §3): an OPEN owner's method may compile —
        // marked so the translator emits no direct sibling calls (every send
        // crosses a dispatch-equivalent seam; a reopen then simply dispatches
        // to its new template, per-dispatch minting making the stale entry
        // unreachable). Sealed owners keep the direct-call fast path.
        let open_owner = !sealed;
        let mut params = Vec::new();
        let mut spec_params = Vec::new();
        for arg in &block_node.arguments {
            if arg.identifier.identifier_type == IdentifierType::Instance {
                crate::codegen::record_refusal(
                    selector,
                    crate::codegen::RefusalKind::PrecheckSignature,
                    "instance-variable parameter",
                );
                return; // not a plain method parameter list
            }
            let Some(hint) = &arg.type_hint else {
                // Speculative-AOT (S0): an UNANNOTATED param no longer ends
                // candidacy — it rides as an Obj placeholder whose real kind
                // the runtime profile supplies at compile time (with an entry
                // precondition, S1). An annotation stays a dispatch GUARANTEE
                // and is preferred whenever present.
                params.push(crate::codegen::AotParam::Obj);
                spec_params.push(true);
                continue;
            };
            // The ERASED dispatch name (`List(Integer)` → `List`), exactly what the
            // runtime guarantees about the arg; the element tag rides separately in
            // `StaticBlock.param_elem_tags`, where the translator picks it up as a
            // `CollectionOf` proof (B1). Every non-scalar name — a class, an erased
            // type variable's `Object`, a nullable — rides as a slot-resident Obj.
            let name = self.dispatch_type_name(hint);
            params.push(crate::codegen::AotParam::from_annotation(&name));
            spec_params.push(false);
        }
        let mut spec_ret = false;
        let ret = match &block_node.return_type {
            // Same erasure as params: `^List(U)` returns a List at runtime
            // (the variables are checker-only); `^T?`/`^Object` compile as Obj
            // returns — this used to end candidacy and silently kept e.g.
            // `Iterate#detect:` interpreted forever.
            Some(rt) => {
                let name = self.dispatch_type_name(rt);
                crate::codegen::AotRet::from_annotation(&name)
            }
            // An absent return annotation was ALSO a candidacy cliff before
            // S0; the profile observes the real return kind for S2 (Obj until
            // then).
            None => {
                spec_ret = true;
                crate::codegen::AotRet::Obj
            }
        };
        // `compile_block` just pushed the compiled body as a block constant.
        let Some(Instruction::Push(Constant::Block(rc))) = bytecode.bytecode.last() else {
            return;
        };
        let group_id = imm.id;
        self.aot_candidates.push(crate::codegen::AotCandidate {
            group_id,
            selector: selector.to_string(),
            block: rc.clone(),
            params,
            ret,
            open_owner,
            role: crate::codegen::AotRole::Method,
            spec_params,
            spec_ret,
            spec_preconditions: Vec::new(),
        });
    }

    /// Collect a nested block LITERAL as a block-template candidate (B3a,
    /// docs/internal/BLOCK_AOT_ARCH.md §3): invoked via `valueWithSelfOrArg:` from the
    /// combinator seams when the registry has a compiled entry. Cheap
    /// prefilter only — translation refusals do the real gating; the prescan
    /// skips the two shapes that always refuse (a nested literal push, a
    /// non-local return) to keep unit-load compile time down.
    fn maybe_collect_block_candidate(&mut self, rc: &Arc<StaticBlock>) {
        if !self.collect_aot || !self.mint_template_ids {
            return;
        }
        let Some(tid) = rc.template_id else {
            return;
        };
        let skip = |why: &str| {
            crate::codegen::record_refusal(
                &format!("block@{tid}"),
                crate::codegen::RefusalKind::PrecheckBlockShape,
                why,
            );
        };
        if rc.param_syms.len() > 1 {
            skip("block takes more than one parameter");
            return;
        }
        if rc.decl_block.is_some() {
            skip("block has a guard/decl block");
            return;
        }
        if rc.name.is_some() {
            skip("named block");
            return;
        }
        // A config literal's stores bind into its own frame (E); the
        // template translator's free-variable write path (env_set) would
        // chain-write instead. Configs are never invoked through the
        // vWSOA seam anyway — this is the defensive mirror.
        if rc.is_init_literal {
            skip("init-literal config block");
            return;
        }
        if rc
            .bytecode
            .0
            .iter()
            .any(|i| matches!(i, Instruction::Push(Constant::Block(_))))
        {
            skip("nested block literal");
            return;
        }
        if rc
            .bytecode
            .0
            .iter()
            .any(|i| matches!(i, Instruction::MethodReturn))
        {
            skip("non-local return (^^) inside the block");
            return;
        }
        self.aot_candidates.push(crate::codegen::AotCandidate {
            // Blocks have no sibling group (no direct calls either way).
            group_id: u32::MAX,
            selector: format!("block@{}", rc.template_id.unwrap()),
            block: rc.clone(),
            params: vec![crate::codegen::AotParam::Obj],
            ret: crate::codegen::AotRet::Obj,
            open_owner: true,
            role: crate::codegen::AotRole::BlockTemplate,
            spec_params: vec![false],
            spec_ret: false,
            spec_preconditions: Vec::new(),
        });
    }

    /// Opt this compile into AOT-candidate collection (implies nothing at
    /// runtime by itself — the runner hands the candidates to codegen).
    pub fn with_aot(mut self) -> Self {
        self.collect_aot = true;
        self
    }

    /// The AOT candidates collected during compilation (drained).
    pub fn take_aot_candidates(&mut self) -> Vec<crate::codegen::AotCandidate> {
        std::mem::take(&mut self.aot_candidates)
    }

    pub fn new_with_locals(locals: HashSet<String>) -> Self {
        Self {
            scopes: vec![Scope {
                locals,
                immutable: HashSet::new(),
                types: HashMap::new(),
                declared_types: HashMap::new(),
                narrowed: HashMap::new(),
                provenance: HashMap::new(),
                is_init: false,
                renaming: false,
                renames: HashMap::new(),
            }],
            temp_counter: 0,
            value_type_def_depth: 0,
            next_block_is_init: false,
            next_block_narrowing: None,
            next_block_expected: None,
            block_literal_types: HashMap::new(),
            block_ret_harvest: Vec::new(),
            collect_portability: false,
            block_portability: Vec::new(),
            next_block_is_expression: false,
            boundary_block_literals: HashMap::new(),
            next_block_capture: None,
            captured_arm_exit: None,
            inline_depth: 0,
            class_bodies: HashMap::new(),
            self_override: None,
            param_override: HashMap::new(),
            splice_rename_counter: 0,
            fused_loop_depth: 0,
            class_ctx: Vec::new(),
            top_level_self_is_nil: false,
            inline_carets: None,
            seen_types: SeenTypes::with_builtins(),
            class_table: ClassTable::new(),
            diagnostics: Vec::new(),
            allow_pragmas: HashMap::new(),
            error_span: None,
            return_type_stack: Vec::new(),
            mint_template_ids: false,
            collect_aot: false,
            aot_candidates: Vec::new(),
            class_ctx_counter: 0,
        }
    }

    /// A method definition at unit top level, outside any class body, when top-level `self`
    /// is the nil default: reject at COMPILE time with the actual fix. At runtime it would
    /// die extending sealed Nil — an error naming a class the user never wrote. Inside a
    /// class body (`class_ctx` non-empty) or under `eval:self:` it is legitimate.
    fn reject_top_level_method(&self, selector: &str) -> Result<(), String> {
        // `scopes.len() == 1` = a true top-level STATEMENT. Inside any block literal
        // (`scopes` grows per block) a method definition is expression material — the
        // test DSL's `.test:name -> { … }` defines on whatever `self` the block gets at
        // runtime, which is exactly the eigenclass mechanism working as intended.
        if self.scopes.len() == 1 && self.class_ctx.is_empty() && self.top_level_self_is_nil {
            return Err(format!(
                "`{selector}` is a method definition, and methods live in classes — at the \
                 top level there is no class to define it on. Put it in a class body \
                 (`Name <- {{ {selector} -> {{ … }} }}`), or bind a block instead: \
                 `var {} = {{ … }}` (call it with `.value`)",
                selector.trim_end_matches(':')
            ));
        }
        Ok(())
    }

    /// Is this `<-`/`<--` target an immediate value type? `true`/`false`/`nil` are
    /// `Identifier` nodes by name, alongside the `Integer`/`Double`/`Boolean`/`Nil`
    /// class names.
    ///
    /// NOTE: this is a *static* check, so it only catches syntactically-literal
    /// targets. A *computed* target that resolves to a value type — e.g.
    /// `(1 + 2) <-- { |@x| test -> { @x } }` — is not recognized here, so the
    /// compiler accepts it. It's harmless rather than wrong (the `@x` reads `nil`
    /// and any `@x =` throws at runtime), but it's also useless. Catching it
    /// requires a *runtime* check at `get_target_class_for_def` time: reject
    /// instance-variable declaration/use when the receiver resolves to a value
    /// type. See QUOIN_TODO.md.
    fn is_value_type_target(node: &Node) -> bool {
        match &node.value {
            // Literal value-type instances: `5 <-- …`, `3.14 <-- …`.
            NodeValue::Integer(_) | NodeValue::Double(_) => true,
            // Class names, plus `true` / `false` / `nil` (which are identifiers by
            // name): `Integer <-- …`, `true <-- …`, etc.
            NodeValue::Identifier(id) => matches!(
                id.name.as_str(),
                "Integer" | "Double" | "Boolean" | "Nil" | "true" | "false" | "nil"
            ),
            _ => false,
        }
    }

    /// Add every top-level `Name <- …` class definition to `seen_types`, so an annotation can
    /// forward-reference a class defined later in the same unit (and so later units see it).
    /// Only simple top-level defs are collected — the common case.
    fn prescan_class_defs(&self, program: &ProgramNode) {
        for expr in &program.expressions {
            if let NodeValue::ClassDefinition(cd) = &expr.value {
                let name = ident_name(&cd.identifier);
                self.seen_types.insert(&name);
                self.class_table.insert(&name, self.class_sig_from_def(cd));
            }
        }
    }

    /// Resolve a type-annotation name to a `Type`, flagging an unknown user class with a
    /// non-fatal `unknown type Foo` diagnostic (Phase 2). Resolution never fails: an unknown
    /// name still yields `Instance(name)` so lowering proceeds (gradual best-effort).
    /// Push a non-fatal type diagnostic of `kind` (a [`WARNING_KINDS`] slug), pointing at
    /// `span` when one is available (Phase 4).
    fn warn(&mut self, kind: &'static str, message: String, span: Option<&SourceInfo>) {
        self.warn_with_notes(kind, message, span, Vec::new());
    }

    /// Like [`warn`](Self::warn) but with secondary why-chain notes (Phase 4 provenance).
    /// The one diagnostic sink — a `"* allow: <kind>` pragma trailing the warned line
    /// (installed by `install_allow_pragmas`) drops the diagnostic here, so every consumer
    /// (`qn check`'s exit code, warning counts, reports) sees the suppressed set.
    fn warn_with_notes(
        &mut self,
        kind: &'static str,
        message: String,
        span: Option<&SourceInfo>,
        notes: Vec<Note>,
    ) {
        if let Some(s) = span
            && self
                .allow_pragmas
                .get(&s.line)
                .is_some_and(|kinds| kinds.iter().any(|k| k == kind))
        {
            return;
        }
        self.diagnostics.push(Diagnostic {
            kind,
            message,
            span: span.cloned(),
            notes,
        });
    }

    /// Validate and install this unit's `"* allow: …` pragmas (scanned by the parser —
    /// comments are pest trivia). Only a *trailing* pragma with known kind names suppresses:
    /// on its own line a pragma would be captured as a doc block by the `"*` adjacency rules,
    /// so that shape gets a warning instead of a silent no-op — as do an unknown kind name
    /// and an empty kind list. Runs before the statement loop so suppression is in place
    /// when the first check fires.
    fn install_allow_pragmas(&mut self, program: &ProgramNode) {
        self.allow_pragmas.clear();
        for p in &program.allow_pragmas {
            let span = Some(&p.span);
            let known: Vec<&String> = p
                .kinds
                .iter()
                .filter(|k| WARNING_KINDS.contains(&k.as_str()))
                .collect();
            if !p.trailing {
                // Prose in an ordinary comment can start with `allow:` (e.g. documenting a
                // selector named `allow:`); only warn when it names a real warning kind.
                if !known.is_empty() {
                    self.warn(
                        "allow-pragma",
                        "an `allow:` pragma must trail the code line it suppresses".to_string(),
                        span,
                    );
                }
                continue;
            }
            if p.kinds.is_empty() {
                self.warn(
                    "allow-pragma",
                    format!(
                        "`allow:` names no warning kind — nothing is suppressed (known kinds: {})",
                        WARNING_KINDS.join(", ")
                    ),
                    span,
                );
                continue;
            }
            for k in &p.kinds {
                if !WARNING_KINDS.contains(&k.as_str()) {
                    self.warn(
                        "allow-pragma",
                        format!(
                            "unknown warning kind `{k}` in `allow:` (known kinds: {})",
                            WARNING_KINDS.join(", ")
                        ),
                        span,
                    );
                }
            }
            if !known.is_empty() {
                self.allow_pragmas
                    .entry(p.line)
                    .or_default()
                    .extend(known.into_iter().cloned());
            }
        }
    }

    /// Lint (QUOIN_TODO): a `^` ending an `if:`/`else:` arm whose send value
    /// is DISCARDED (statement position) is almost always a mistyped `^^` —
    /// the `^` yields the arm's value to a send nobody reads, and control
    /// FALLS THROUGH to the next statement. The legitimate `^` uses (early
    /// exit from iteration blocks, value-producing arms) don't have this
    /// shape. Statement loops call this on every non-final statement.
    fn check_discarded_caret_arm(&mut self, stmt: &Node) {
        let NodeValue::MethodCall(call) = &stmt.value else {
            return;
        };
        let idents = &call.arguments.signature.identifiers;
        let kws: Vec<&str> = idents.iter().map(|i| i.name.as_str()).collect();
        if !matches!(kws.as_slice(), ["if"] | ["else"] | ["if", "else"]) {
            return;
        }
        for arg in &call.arguments.expressions {
            let NodeValue::Block(b) = &arg.value else {
                continue;
            };
            let Some(last) = b.statements.last() else {
                continue;
            };
            if matches!(&last.value, NodeValue::BlockReturn(_)) {
                self.warn(
                    "caret-discard",
                    "`^` returns from this block, but the surrounding `if:`/`else:` \
                     value is discarded — control falls through to the next \
                     statement; a method return here is `^^`"
                        .to_string(),
                    last.source_info.as_ref(),
                );
            }
        }
    }

    /// The non-fatal type diagnostics collected during compilation (Phase 2 warnings).
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// The portable-block classifications collected under
    /// [`with_portability`](Self::with_portability), span-keyed.
    pub fn block_portability(&self) -> &[BlockPortability] {
        &self.block_portability
    }

    /// Use a shared class-name accumulator instead of this compiler's own. The runner threads
    /// one set through every unit (via the VM), so a later unit sees the classes that
    /// earlier-compiled units — the prelude, `use`d modules — defined.
    pub fn set_seen_types(&mut self, seen: SeenTypes) {
        self.seen_types = seen;
    }

    /// Use a shared class-signature table (threaded alongside `seen_types`).
    pub fn set_class_table(&mut self, table: ClassTable) {
        self.class_table = table;
    }

    /// The devirtualized Integer instruction for a binary operator, if it has one.
    fn int_devirt_op(operator: &BinaryOperatorType) -> Option<Instruction> {
        use BinaryOperatorType::*;
        Some(match operator {
            Add => Instruction::IntAdd,
            Sub => Instruction::IntSub,
            Mul => Instruction::IntMul,
            Div => Instruction::IntDiv,
            Mod => Instruction::IntMod,
            Lt => Instruction::IntLt,
            LtEq => Instruction::IntLe,
            Gt => Instruction::IntGt,
            GtEq => Instruction::IntGe,
            Eq => Instruction::IntEq,
            NotEq => Instruction::IntNe,
            _ => return None,
        })
    }

    /// The devirtualized Double instruction for a binary operator, if it has one (the f64 mirror
    /// of `int_devirt_op`).
    fn double_devirt_op(operator: &BinaryOperatorType) -> Option<Instruction> {
        use BinaryOperatorType::*;
        Some(match operator {
            Add => Instruction::DoubleAdd,
            Sub => Instruction::DoubleSub,
            Mul => Instruction::DoubleMul,
            Div => Instruction::DoubleDiv,
            Mod => Instruction::DoubleMod,
            Lt => Instruction::DoubleLt,
            LtEq => Instruction::DoubleLe,
            Gt => Instruction::DoubleGt,
            GtEq => Instruction::DoubleGe,
            Eq => Instruction::DoubleEq,
            NotEq => Instruction::DoubleNe,
            _ => return None,
        })
    }

    pub fn compile_program(&mut self, program: &ProgramNode) -> Result<StaticBlock, CompileError> {
        self.compile_program_with(program, true)
    }

    /// Compile a top-level program. `define_self` emits the default top-level `self = nil`;
    /// pass `false` when the unit runs *as a method* with a receiver (`eval:self:`), where the
    /// frame setup (`start_block_as_method`) binds `self` to the receiver — otherwise this
    /// `self = nil` init would clobber it. `self` still compiles as a local either way
    /// (`is_local` special-cases it), resolving through the env (receiver, or nil when unbound).
    pub fn compile_program_with(
        &mut self,
        program: &ProgramNode,
        define_self: bool,
    ) -> Result<StaticBlock, CompileError> {
        self.error_span = None;
        self.compile_program_inner(program, define_self)
            .map_err(|message| CompileError {
                message,
                span: self.error_span.take(),
            })
    }

    /// The body of `compile_program_with`, with the internal `String` error type: the span
    /// travels out-of-band in `self.error_span` (claimed by the innermost `compile_node`
    /// frame or a precise `err_at` site) and is attached by the public wrapper.
    fn compile_program_inner(
        &mut self,
        program: &ProgramNode,
        define_self: bool,
    ) -> Result<StaticBlock, String> {
        // Remembered so the MethodDefinition arm can reject a TOP-LEVEL `sel -> { … }` when
        // `self` is the nil default: it would try to extend sealed Nil at runtime, and that
        // error ("Cannot extend sealed class [/]Nil") names a class the user never wrote
        // (RELEASE_PREP Tier 4a). `eval:self:` passes `define_self: false` (a real receiver
        // is bound), where a top-level definition legitimately targets that receiver.
        self.top_level_self_is_nil = define_self;
        // Suppressions first: the pragma set must be live before any check can warn.
        self.install_allow_pragmas(program);
        // Pre-scan this unit's class defs so annotations can forward-reference them (and so
        // later-compiled units see them via the shared `seen_types`).
        self.prescan_class_defs(program);
        let mut cb = CodeBlock::new();

        cb.current_source = program.source_info.clone();
        if define_self {
            cb.push(Instruction::Push(Constant::Nil));
            cb.push(Instruction::DefineLocal(Symbol::intern("self")));
            self.scopes[0].locals.insert("self".to_string());
        }

        let len = program.expressions.len();
        for (idx, expr) in program.expressions.iter().enumerate() {
            cb.current_source = expr.source_info.clone();
            self.compile_node(expr, &mut cb)?;
            if idx < len - 1 {
                self.check_discarded_caret_arm(expr);
                cb.push(Instruction::Pop);
            }
        }

        cb.current_source = program.source_info.clone();
        if len == 0 {
            cb.push(Instruction::Push(Constant::Nil));
        }

        cb.push(Instruction::Return);

        let (bytecode, source_map) = fuse_bytecode(cb.bytecode, cb.source_map);
        Ok(StaticBlock {
            spec_state: Default::default(),
            uses_self: Default::default(),
            is_closed: Default::default(),
            name: None,
            is_nested_block: false,
            is_init_literal: false,
            param_syms: Vec::new(),
            param_types: Vec::new(),
            param_elem_tags: Vec::new(),
            bytecode: SharedBytecode(Arc::new(bytecode)),
            source_info: program.source_info.clone(),
            decl_block: None,
            source_map: SharedSourceMap(Arc::new(source_map)),
            template_id: self
                .mint_template_ids
                .then(crate::instruction::fresh_template_id),
        })
    }

    fn compile_node(&mut self, node: &Node, bytecode: &mut CodeBlock) -> Result<(), String> {
        let prev_source = bytecode.current_source.clone();
        bytecode.current_source = node.source_info.clone();
        let res = self.compile_node_internal(node, bytecode);
        if res.is_err() && self.error_span.is_none() {
            // Claim the error's span innermost-first: the deepest `compile_node` frame sees
            // the error while its node's span is still current; enclosing frames find the
            // claim already made. An `err_at` site with a tighter span pre-empts this.
            self.error_span = bytecode.current_source.clone();
        }
        bytecode.current_source = prev_source;
        res
    }

    /// Record `span` as the compile error's location and pass the message through — for error
    /// sites that know a tighter span (the offending identifier) than the enclosing statement,
    /// which `compile_node` would otherwise fill in. First claim wins; `None` leaves the
    /// statement-level fallback in place.
    fn err_at(&mut self, span: &Option<SourceInfo>, message: String) -> String {
        if self.error_span.is_none() && span.is_some() {
            self.error_span = span.clone();
        }
        message
    }
}

#[cfg(test)]
mod tests;
