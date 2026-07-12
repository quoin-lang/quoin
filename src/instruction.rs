use crate::symbol::Symbol;
use crate::value::{NamespacedName, SourceInfo};

use gc_arena::Collect;
use std::ops::Deref;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq)]
pub struct SharedBytecode(pub Arc<Vec<Instruction>>);

unsafe impl<'gc> Collect<'gc> for SharedBytecode {
    const NEEDS_TRACE: bool = false;
}

impl Deref for SharedBytecode {
    type Target = [Instruction];
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<Vec<Instruction>> for SharedBytecode {
    fn from(v: Vec<Instruction>) -> Self {
        SharedBytecode(Arc::new(v))
    }
}

impl PartialEq<Vec<Instruction>> for SharedBytecode {
    fn eq(&self, other: &Vec<Instruction>) -> bool {
        self.0.as_ref() == other
    }
}

impl PartialEq<SharedBytecode> for Vec<Instruction> {
    fn eq(&self, other: &SharedBytecode) -> bool {
        self == other.0.as_ref()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SharedSourceMap(pub Arc<Vec<Option<SourceInfo>>>);

unsafe impl<'gc> Collect<'gc> for SharedSourceMap {
    const NEEDS_TRACE: bool = false;
}

impl Deref for SharedSourceMap {
    type Target = [Option<SourceInfo>];
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<Vec<Option<SourceInfo>>> for SharedSourceMap {
    fn from(v: Vec<Option<SourceInfo>>) -> Self {
        SharedSourceMap(Arc::new(v))
    }
}

impl PartialEq<Vec<Option<SourceInfo>>> for SharedSourceMap {
    fn eq(&self, other: &Vec<Option<SourceInfo>>) -> bool {
        self.0.as_ref() == other
    }
}

impl PartialEq<SharedSourceMap> for Vec<Option<SourceInfo>> {
    fn eq(&self, other: &SharedSourceMap) -> bool {
        self == other.0.as_ref()
    }
}

#[derive(Clone, Debug, Collect)]
#[collect(require_static)]
pub struct StaticBlock {
    pub name: Option<String>,
    pub is_nested_block: bool,
    /// Parameter names interned at compile time; the runtime `Block` reads them
    /// through its shared `template` reference.
    pub param_syms: Vec<Symbol>,
    pub param_types: Vec<String>,
    /// Per-param element-tag requirement for tag-aware dispatch: a
    /// `List(Integer)` param matches only Integer-tagged lists (the base class
    /// lives in `param_types`; docs/internal/GENERICS_ARCH.md §5). Empty = no
    /// requirements (every pre-generics block).
    pub param_elem_tags: Vec<Option<crate::runtime::elem_tag::ElemTag>>,
    pub bytecode: SharedBytecode,
    pub source_info: Option<SourceInfo>,
    pub decl_block: Option<Arc<StaticBlock>>,
    pub source_map: SharedSourceMap,
    /// Compiler-minted unique id for this block literal. Every closure
    /// materialized from the same literal shares one inline-cache array keyed by
    /// this id (`VmState::ic_registry`), so call sites stay warm across
    /// re-materialization. `None` — runtime-built blocks (eval, string
    /// interpolation, runner entry) — keeps a private per-closure cache, since a
    /// per-evaluation compile would otherwise grow the registry without bound.
    pub template_id: Option<u32>,
    /// A `new:{...}` CONFIG-BLOCK literal. Its assignments are the
    /// field-binding DSL: they always BIND INTO THE BLOCK'S OWN frame — the
    /// syntax only compiles in `new:`-argument position, and as of (E) the
    /// semantics are STATIC (caller-independent): `store_set_local` honors
    /// this flag even when a user-defined `new:` invokes the block as a
    /// plain closure (previously such an invocation chain-walked the write —
    /// a caller-dependent corner nothing could reason about, including the
    /// AOT materialization gates).
    pub is_init_literal: bool,
    /// Speculative-AOT observation state (spec::NOT_SPECULATIVE/OBSERVING/
    /// RESOLVED, docs/internal/SPECULATIVE_AOT_ARCH.md S0). Lives HERE so the
    /// method-entry gate is one byte off a template line that entry binding
    /// touches anyway — a side table would cost a dependent pointer chase on
    /// every method call. Shared by all closures of the literal (one `Arc`).
    pub spec_state: SpecState,
    /// Memoized closed-template scan (see `template_is_closed`).
    pub is_closed: UsesSelfFlag,
    /// Memoized "does this block (or a nested literal) reference `self` or
    /// touch `@fields`?" — the self-or-arg seams (`valueWithSelfOrArg:`,
    /// compiled outcalls) resolve LEXICAL self through the env chain only
    /// for blocks that need it; the common no-self block keeps the free
    /// path. 0 = unscanned, 1 = no, 2 = yes.
    pub uses_self: UsesSelfFlag,
}

/// One atomic byte, same shape as `SpecState`: shared by every closure of
/// the literal through the template `Arc` (must be `Send`+`Sync` for C2's
/// portable blocks); Relaxed everywhere — a racing observer merely rescans.
/// `Clone` gives the copy an independent (unscanned) flag.
#[derive(Debug, Default)]
pub struct UsesSelfFlag(std::sync::atomic::AtomicU8);

impl Clone for UsesSelfFlag {
    fn clone(&self) -> Self {
        Self::default()
    }
}

/// Whether the template is CLOSED — no free-variable reads, no `self`, no
/// fields, no nested literal that breaks the same rules. A closed template's
/// closures are all behaviorally identical, so materialization may return a
/// cached instance (per VM): constant-closure promotion. Memoized like
/// `uses_self`.
pub fn template_is_closed(sb: &StaticBlock) -> bool {
    use std::sync::atomic::Ordering;
    match sb.is_closed.0.load(Ordering::Relaxed) {
        1 => return false, // note: 1 = NOT closed (matches UsesSelfFlag polarity: 1=no-hit)
        2 => return true,
        _ => {}
    }
    let closed = scan_closed(sb);
    sb.is_closed
        .0
        .store(if closed { 2 } else { 1 }, Ordering::Relaxed);
    closed
}

fn scan_closed(sb: &StaticBlock) -> bool {
    if template_uses_self(sb) {
        return false;
    }
    let mut defined: std::collections::HashSet<Symbol> = sb.param_syms.iter().copied().collect();
    for inst in sb.bytecode.iter() {
        // Locals defined in this body are not free.
        if let Instruction::DefineLocal(sym) = inst {
            defined.insert(*sym);
        }
    }
    for inst in sb.bytecode.iter() {
        let free = match inst {
            Instruction::LoadLocal(s)
            | Instruction::StoreLocal(s)
            | Instruction::StoreLocalKeep(s) => !defined.contains(s),
            Instruction::SendLocal(v, _, _) => !defined.contains(v),
            Instruction::SendLocalLocal(a, b, _, _) => !defined.contains(a) || !defined.contains(b),
            Instruction::SendLocalConst(a, _, _, _) => !defined.contains(a),
            Instruction::IntBinLL(a, b, _) | Instruction::DoubleBinLL(a, b, _) => {
                !defined.contains(a) || !defined.contains(b)
            }
            Instruction::IntBinLC(a, _, _) | Instruction::DoubleBinLC(a, _, _) => {
                !defined.contains(a)
            }
            Instruction::MethodReturn => true, // ^^ needs a home
            _ => false,
        };
        if free {
            return false;
        }
        if let Instruction::Push(Constant::Block(inner)) = inst
            && !scan_closed(inner)
        {
            return false;
        }
        if let Some((_, _, Some(Constant::Block(inner)))) = inst.send_parts()
            && !scan_closed(inner)
        {
            return false;
        }
    }
    true
}

/// Whether the template references `self` — directly (a bare `self` read or
/// an implicit-self send lowers to a self-symbol local op), through
/// `@field` access, or inside any NESTED block literal (a closure it
/// materializes resolves `self` through the env this frame provides).
/// Conservative on the field ops; memoized per template.
pub fn template_uses_self(sb: &StaticBlock) -> bool {
    use std::sync::atomic::Ordering;
    match sb.uses_self.0.load(Ordering::Relaxed) {
        1 => return false,
        2 => return true,
        _ => {}
    }
    let self_sym = crate::symbol::self_symbol();
    let mut uses = false;
    for inst in sb.bytecode.iter() {
        let hit = match inst {
            Instruction::LoadField(_)
            | Instruction::StoreField(_)
            | Instruction::StoreFieldKeep(_)
            | Instruction::SendField(_, _, _) => true,
            Instruction::LoadLocal(s) | Instruction::StoreLocal(s) => *s == self_sym,
            Instruction::SendLocal(v, _, _) => *v == self_sym,
            Instruction::SendLocalLocal(a, b, _, _) => *a == self_sym || *b == self_sym,
            Instruction::SendLocalConst(a, _, _, _) => *a == self_sym,
            Instruction::IntBinLL(a, b, _) | Instruction::DoubleBinLL(a, b, _) => {
                *a == self_sym || *b == self_sym
            }
            Instruction::IntBinLC(a, _, _) | Instruction::DoubleBinLC(a, _, _) => *a == self_sym,
            _ => false,
        };
        if hit {
            uses = true;
            break;
        }
        if let Instruction::Push(Constant::Block(inner)) = inst
            && template_uses_self(inner)
        {
            uses = true;
            break;
        }
        if let Some((_, _, Some(Constant::Block(inner)))) = inst.send_parts()
            && template_uses_self(inner)
        {
            uses = true;
            break;
        }
    }
    sb.uses_self
        .0
        .store(if uses { 2 } else { 1 }, Ordering::Relaxed);
    uses
}

/// One atomic byte (was `Cell<u8>`): the flag is shared by every closure of
/// the literal through the template `Arc`, which must be `Send` for C2's
/// portable blocks (docs/internal/CONCURRENCY_ARCH.md §5/§10). Relaxed everywhere —
/// the flag is advisory tiering state; a racing observer only costs a
/// conservative re-check. `Clone` gives the copy an independent flag, the
/// same semantics `Cell` had under `#[derive(Clone)]`.
#[derive(Debug, Default)]
pub struct SpecState(std::sync::atomic::AtomicU8);

impl SpecState {
    pub fn get(&self) -> u8 {
        self.0.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn set(&self, v: u8) {
        self.0.store(v, std::sync::atomic::Ordering::Relaxed)
    }
}

impl Clone for SpecState {
    fn clone(&self) -> Self {
        SpecState(std::sync::atomic::AtomicU8::new(self.get()))
    }
}

unsafe impl<'gc> gc_arena::Collect<'gc> for SpecState {
    const NEEDS_TRACE: bool = false;
}

/// Mint a globally unique template id (compile time only; ids are never reused,
/// so a registry entry keyed by one is a stable call-site identity forever).
pub fn fresh_template_id() -> u32 {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

// Manual PartialEq: `template_id` is identity metadata, not structure — two
// otherwise-identical literals from different compiles should still compare
// equal (compiler tests build expected bytecode by hand).
impl PartialEq for StaticBlock {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.is_nested_block == other.is_nested_block
            && self.is_init_literal == other.is_init_literal
            && self.param_syms == other.param_syms
            && self.param_types == other.param_types
            && self.bytecode == other.bytecode
            && self.source_info == other.source_info
            && self.decl_block == other.decl_block
            && self.source_map == other.source_map
    }
}

#[derive(Clone, Debug, Collect, PartialEq)]
#[collect(require_static)]
pub enum Constant {
    Nil,
    Bool(bool),
    Int(i64),
    Double(f64),
    String(String),
    Symbol(String),
    Block(Arc<StaticBlock>),
}

impl Constant {
    /// Wrap a [`StaticBlock`] into a `Constant::Block` (the variant carries an `Rc`
    /// so materialization is a refcount bump). Convenience for tests/builders.
    pub fn block(sb: StaticBlock) -> Constant {
        Constant::Block(Arc::new(sb))
    }

    /// The integer value if this is an `Int` literal — for `IntBinLC`'s fast path.
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Constant::Int(i) => Some(*i),
            _ => None,
        }
    }

    /// The float value if this is a `Double` literal — for `DoubleBinLC`'s fast path.
    pub fn as_double(&self) -> Option<f64> {
        match self {
            Constant::Double(d) => Some(*d),
            _ => None,
        }
    }
}

/// The specific integer binary op carried by the fused `IntBinLL`/`IntBinLC`
/// superinstructions (Slice a1) — mirrors the standalone `IntAdd`..`IntNe` ops, each mapping
/// to its arithmetic/comparison result and its fallback send selector.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntBinKind {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

impl IntBinKind {
    /// The keyword selector to fall back to when an operand isn't an `Int` at runtime.
    pub fn selector(self) -> &'static str {
        match self {
            IntBinKind::Add => "+:",
            IntBinKind::Sub => "-:",
            IntBinKind::Mul => "*:",
            IntBinKind::Div => "/:",
            IntBinKind::Mod => "%:",
            IntBinKind::Lt => "<:",
            IntBinKind::Le => "<=:",
            IntBinKind::Gt => ">:",
            IntBinKind::Ge => ">=:",
            IntBinKind::Eq => "==:",
            IntBinKind::Ne => "!=:",
        }
    }

    /// The reverse map, for TRANSLATION-TIME devirtualization of generic
    /// sends whose operands are proven scalars (S2): sealed Integer/Double
    /// arithmetic is frozen, so `C(Int) '+:' C(Int)` may compile to the
    /// machine op — the same guarantee the compiler's typed devirt uses.
    pub fn from_selector(sel: &str) -> Option<IntBinKind> {
        Some(match sel {
            "+:" => IntBinKind::Add,
            "-:" => IntBinKind::Sub,
            "*:" => IntBinKind::Mul,
            "/:" => IntBinKind::Div,
            "%:" => IntBinKind::Mod,
            "<:" => IntBinKind::Lt,
            "<=:" => IntBinKind::Le,
            ">:" => IntBinKind::Gt,
            ">=:" => IntBinKind::Ge,
            "==:" => IntBinKind::Eq,
            "!=:" => IntBinKind::Ne,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, Collect, PartialEq)]
#[collect(require_static)]
pub enum Instruction {
    LoadLocal(Symbol),
    DefineLocal(Symbol),
    StoreLocal(Symbol),
    LoadGlobal(NamespacedName),
    StoreGlobal(NamespacedName, bool),
    Push(Constant),
    Pop,
    Dup,
    Send(Symbol, usize), // selector, num_args
    // Superinstructions: a single fused op for the hot `<operand-load>; Send` pairs (the
    // last operand of a send is overwhelmingly a local / constant / field — see
    // profiling/superinstructions). Each pushes its operand then runs the normal send,
    // saving one dispatch-loop step per send. Produced by the `fuse_bytecode` peephole
    // pass; never emitted directly by the AST compiler.
    SendLocal(Symbol, Symbol, usize), // var, selector, num_args  (was LoadLocal; Send)
    SendConst(Constant, Symbol, usize), // constant, selector, num_args  (was Push; Send)
    SendField(String, Symbol, usize), // field, selector, num_args  (was LoadField; Send)
    // Store-and-keep superinstructions: a `Dup; Store*` pair (an assignment whose value is
    // used as an expression) fused into one op that stores the *top* of stack without
    // popping it. The statement-position form `Dup; Store*; Pop` is instead collapsed to a
    // plain `Store*` (both by the `fuse_bytecode` pass). Mirror DefineLocal/StoreLocal/
    // StoreField but peek instead of pop.
    DefineLocalKeep(Symbol),
    StoreLocalKeep(Symbol),
    StoreFieldKeep(String),
    // 3-instruction sends: absorb a *second* operand-load into a fused send, so one op
    // pushes two operands (left-to-right) then dispatches. Covers the two hottest
    // receiver+last-operand shapes — `LoadLocal; LoadLocal; Send` (e.g. `i < n`) and
    // `LoadLocal; Push; Send` (e.g. `n - 1`). The operands are just the last two pushed
    // before the send (receiver + arg for a 1-arg send); produced by `fuse_bytecode`.
    SendLocalLocal(Symbol, Symbol, Symbol, usize), // local, local, selector, num_args
    SendLocalConst(Symbol, Constant, Symbol, usize), // local, constant, selector, num_args
    // Devirtualized Integer operators (Slice 2a): the compiler emits these instead of a
    // `Send("+:", 1)` etc. when both operands are statically `Integer` (a sealed value
    // type). Each pops two `Value::Int`s and pushes the result directly — no method
    // lookup, no dispatch. Semantics match Integer's native ops exactly: `+`/`-`/`*` wrap
    // like i64; `/`/`%` raise "Division by zero" on a zero divisor; compares yield a Bool.
    IntAdd,
    IntSub,
    IntMul,
    IntDiv,
    IntMod,
    IntLt,
    IntLe,
    IntGt,
    IntGe,
    IntEq,
    IntNe,
    // Fused integer superinstructions (Slice a1): the peephole pass collapses
    // `LoadLocal; <LoadLocal|Push>; IntXxx` — the two hottest arithmetic shapes (`i < n`,
    // `n - 1`) — into one op that loads both operands and computes directly, saving two
    // dispatch-loop steps. A non-Int operand falls back to the real send (same contract as
    // the standalone `Int` ops above).
    IntBinLL(Symbol, Symbol, IntBinKind),   // local, local, op
    IntBinLC(Symbol, Constant, IntBinKind), // local, constant, op
    // Devirtualized Double operators (mirror of the Integer ops above): emitted when both
    // operands are statically `Double` (a sealed value type). Each pops two `Value::Double`s
    // and computes directly. Semantics match Double's native ops exactly — plain IEEE-754
    // f64: `/`/`%` yield inf/NaN on a zero divisor (they do NOT raise, unlike Integer); `==`
    // is f64 equality (`NaN != NaN`); compares yield a Bool. A non-Double operand falls back
    // to the real send. The fused `DoubleBinLL`/`DoubleBinLC` reuse `IntBinKind` (the operator
    // kind is type-agnostic — same `+:`/`-:`/… selectors).
    DoubleAdd,
    DoubleSub,
    DoubleMul,
    DoubleDiv,
    DoubleMod,
    DoubleLt,
    DoubleLe,
    DoubleGt,
    DoubleGe,
    DoubleEq,
    DoubleNe,
    DoubleBinLL(Symbol, Symbol, IntBinKind), // local, local, op
    DoubleBinLC(Symbol, Constant, IntBinKind), // local, constant, op
    // Devirtualized List accessors (Slice 2e): emitted instead of `Send("at:", 1)` /
    // `Send("at:put:", 2)` / `Send("add:", 1)` when the receiver is statically a `List` (a
    // sealed value type — its access methods can't be redefined). Each pops the same
    // operands a send would and does the indexed op directly on the backing `Vec`, matching
    // native semantics (OOB read → nil; OOB write → IndexError; both mutators evaluate to the
    // receiver). If the receiver isn't a native list at runtime (a `List`-typed local
    // reassigned to something else), each falls back to the real send — a pure speedup.
    ListGet,
    ListSet,
    ListPush,
    // Devirtualized Map accessors (mirror of the List ops): emitted when the receiver is
    // statically a `Map` (a sealed value type). Map is `IndexMap<String, Value>` — its key must be
    // a String at runtime (else fall back to the send, matching native `at:`). `MapGet` (`at:`) →
    // value or nil; `MapSet` (`at:put:`) → the receiver. Each falls back to the real send if the
    // receiver isn't a native map at runtime.
    //
    // Set has NO devirt op: native `Set#contains?:`/`add:` dispatch `==:` per element (structural
    // for List/Map, custom for user classes), which a direct raw-equality op can't replicate.
    MapGet,
    MapSet,
    Return,
    Yeet,
    BlockReturn,
    MethodReturn,
    Jump(isize),
    IfJump(isize),
    ElseJump(isize),
    // Guard for control-flow inlining on a non-statically-Bool receiver (Slice 2d, option
    // C). Peeks the stack top (the conditional's receiver): if it is *not* a `Bool`, jump
    // by the offset to a cold path that performs the real `if:`/`if:else:` send (preserving
    // MessageNotUnderstood / a user-defined `if:else:`), leaving the receiver on the stack;
    // if it *is* a `Bool`, fall through to the inlined branch (which consumes it). Never pops.
    BranchIfNotBool(isize),
    // Guard for fused `each:` loops (B1, docs/internal/BLOCK_AOT_ARCH.md §3). Peeks the stack top
    // (the `each:` receiver): a native List falls through to the inlined index loop —
    // List is sealed, so native-List-ness alone decides that dispatch would select the
    // `List#each:` primitive the loop implements. Anything else jumps to a cold path
    // performing the real `each:` send (a custom class's own `each:`, Set/Map/Generator,
    // or MNU — exact semantics), with the receiver left on the stack. Never pops.
    //
    // The second field is the argument block's template id, when the literal minted one:
    // the interpreted arm ALSO takes the cold path once that template is COMPILED —
    // the real send reaches the compiled block per element (invoke_block), which beats
    // the interpreted splice ~2x (measured: 53 vs 100 ns/element on `(x*3)+1`) — and
    // the guard feeds the template's warmth/argument observation from the list so a
    // splice-only program still tiers up. Compiled METHODS ignore the field: their
    // translated in-frame fused loop is already optimal.
    BranchIfNotList(isize, Option<u32>),
    /// Strict-Boolean loop condition (BUGS.md Finding 14). Peeks the stack
    /// top (a `whileDo:` condition value) WITHOUT popping: a `Bool` falls
    /// through to the loop's `ElseJump`; anything else raises
    /// `MessageNotUnderstood` (matching the dispatched `if:` contract — the
    /// guest `whileDo:` already raises via `.if:`, only the fused loop
    /// coerced). Emitted only when the condition is not statically `Bool`,
    /// so `{ i < n }`-style loops pay nothing.
    RequireBool,
    /// Guard for fused instantiation (M2, docs/internal/MATERIALIZATION_ARCH.md). Peeks the stack
    /// top (the `new:` receiver): if `new:` on it resolves to the BUILT-IN
    /// `Callable::New` — no user `new:` anywhere in the class-side chain — and the class
    /// is instantiable, fall through to the inline field-expression path; otherwise jump
    /// by the offset to a cold path performing the real `new:` send with a materialized
    /// config closure (a user meta `new:`, an abstract class' error, a non-class
    /// receiver's MNU — exact semantics). Never pops; the verdict is cached per site
    /// ((class-ptr, epoch)-guarded, `IC_PLAINNEW_KIND`).
    BranchIfNotPlainNew(isize),
    /// Fused instantiation body (M2): the stack holds the receiver class then the n
    /// field values named here, in order. Allocates the object, binds the named fields
    /// (unknown names silently dropped — `finalize_instantiation`'s exact contract),
    /// runs the init chain through the memoized plan when one exists, and replaces the
    /// window with the finished object. Reached only via `BranchIfNotPlainNew`.
    NewWithFields(Arc<Vec<Symbol>>),
    /// The devirtualized `count` of a native List (the fused `each:` loop bound). Pops
    /// the receiver, pushes its element count; a non-List receiver falls back to the
    /// real `count` send, like every devirt op.
    ListLen,
    NewList(usize), // num_elements
    NewMap(usize),  // num_pairs (key/value count)
    NewSet(usize),  // num_elements
    /// Verify every element of the fresh collection literal on top of the
    /// stack against the tag, then stamp the tag (annotation-driven tagged
    /// literals: `var x: List(Integer) = #(...)` — docs/internal/GENERICS_ARCH.md §4.2).
    TagCollection(crate::runtime::elem_tag::ElemTag),
    NewRegex,
    DefineClass {
        name: NamespacedName,
        parent_name: Option<NamespacedName>,
        instance_vars: Vec<String>,
        /// Where the definition sits in source, recorded into `VmState::class_meta` so doc
        /// extraction can find the `"*` block above it (docs/internal/DOCS_ARCH.md §4). Methods carry
        /// their own location on the template; classes had nowhere to keep one until this.
        source: Option<SourceInfo>,
    },
    ExecuteBlockWithSelf,
    /// A statically-named class reopen (`Name <-- { … }`) records where it happened, so the
    /// `"*` block above the reopen is reachable for docs (docs/internal/DOCS_ARCH.md §4 — extension
    /// sites list beneath the definition's doc). No stack effect; emitted just before the
    /// reopen's `ExecuteBlockWithSelf`. A computed-target reopen emits nothing.
    RecordClassSite {
        name: NamespacedName,
        source: SourceInfo,
    },
    DefineMethod(String),
    OverrideMethod(String),
    LoadField(String),
    // Like `LoadField`, but reads the field off the object popped from the top of the stack
    // instead of `self`. Emitted when a field accessor (`x -> { @x }`) on a statically-known
    // sealed class is inlined at an explicit-receiver call site (`v.x`) — Phase 5·3.
    LoadFieldOf(String),
    StoreField(String),
    /// `use (pkg:)? path;` — load a file once. `package` is `None` for stdlib; `path`
    /// has `.qn` implied; `glob` loads every `.qn` in the directory (Stage 2).
    Use {
        package: Option<String>,
        path: String,
        glob: bool,
    },
}

impl Instruction {
    /// The selector-carrying send forms, exhaustively: `(selector, argc,
    /// fused constant operand if the form carries one)`. Every scan that
    /// reasons about a body's sends (AOT purity sets, materialization-nest
    /// gates, cold-path send identification) must go through this — four
    /// hand-copied match lists had already drifted apart (two silently
    /// missed `SendField`). A new send variant added to the enum is handled
    /// here or its selector is invisible to every gate at once, loudly.
    pub fn send_parts(&self) -> Option<(&Symbol, usize, Option<&Constant>)> {
        match self {
            Instruction::Send(s, n) => Some((s, *n, None)),
            Instruction::SendLocal(_, s, n) => Some((s, *n, None)),
            Instruction::SendField(_, s, n) => Some((s, *n, None)),
            Instruction::SendLocalLocal(_, _, s, n) => Some((s, *n, None)),
            Instruction::SendConst(c, s, n) => Some((s, *n, Some(c))),
            Instruction::SendLocalConst(_, c, s, n) => Some((s, *n, Some(c))),
            _ => None,
        }
    }
}
