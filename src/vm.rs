use crate::dispatch::{Callable, MethodCacheKey};
use crate::error::QuoinError;
use crate::fiber::{VMYielder, YieldReason};
use crate::highlighter::{HighlightSpan, format_ansi, highlight_resilient, highlight_to_ansi};
use crate::instruction::{Constant, Instruction, IntBinKind, SharedBytecode, StaticBlock};
use crate::io_backend::StreamId;
use crate::packages::{FsResolver, LoadedUnit, PackageResolver};
use crate::runtime::elem_tag;
use crate::runtime::fiber::NativeFiberState;
use crate::runtime::list::NativeListState;
use crate::runtime::map::NativeMapState;
use crate::runtime::method::{MethodBody, NativeMethodState};
use crate::runtime::regex::NativeRegexState;
use crate::runtime::runtime::{load_glob, load_unit};
use crate::runtime::set::NativeSetState;
use crate::runtime::streams::NativeStream;
use crate::symbol::{Symbol, init_colon_symbol, init_symbol, new_colon_symbol, self_symbol};
use crate::value::{
    AnyCollect, Block, Class, EnvFrame, Fields, InitEntry, InitPlan, NamespacedName, NativeCall,
    NativeClass, NativeFunc, NativeNewPolicy, Object, ObjectPayload, SourceInfo, Value,
};
use crate::{ansi_colorizer, devirt_ops, gc, gcl};
use std::sync::Arc;

use gc_arena::metrics::Pacing;
use gc_arena::{Collect, Gc, Mutation, lock::RefLock};
use regex::Regex;
use rustc_hash::FxHashMap;
use std::collections::{HashMap, VecDeque};
use std::mem::transmute;
use std::path::Path;
use std::{cmp, fs};

/// GC pacing for a VM arena. `qn` programs are overwhelmingly batch runs where the heap is
/// transient and only throughput matters, so the incremental collector sleeps far longer
/// between cycles than gc_arena's memory-conservative default (`sleep_factor` 0.5) — 4.0
/// trades ~2× peak heap for a large drop in collection overhead on allocation-heavy code
/// (profiling/gc-pacing). `QN_GC_SLEEP=<f64>` overrides it (lower = less memory, higher =
/// more throughput). Never affects results — looser pacing only collects less often.
pub fn gc_pacing() -> Pacing {
    let mut pacing = Pacing::DEFAULT;
    // Under GC stress, keep gc_arena's conservative default sleep so collection stays
    // aggressive — the stress harness exists to surface tracing bugs, which a loose
    // throughput pacing would mask. The throughput override applies only to normal runs.
    if crate::tuning::gc_stress() {
        return pacing;
    }
    pacing.sleep_factor = std::env::var("QN_GC_SLEEP")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .unwrap_or(4.0);
    pacing
}

/// Argument arity an inline-cache slot's guard can encode (larger sends bypass the IC).
pub const IC_MAX_ARGS: usize = 2;

/// The shared (or private) per-template inline-cache cell: one lazily-allocated
/// per-`ip` slot array. Hoisted into `Frame` at push time so the per-send probe
/// reads it off the hot frame instead of chasing `Block::inline_cache`.
pub type InlineCacheCell<'gc> = Gc<'gc, RefLock<Option<Box<[ICSlot<'gc>]>>>>;

/// `ICSlot::recv_kind` value marking a *field-slot* entry (the `LoadField`/`StoreField`
/// family) rather than a send entry: `recv_ptr` holds the receiver's class pointer and
/// `arg_ptrs[0]` the field's slot index. Send and field instructions never share an `ip`,
/// so one per-template array serves both; a field entry can never satisfy `ic_probe`
/// (its `callable` is `None`), and `value_type_guard` kinds are tiny, far from this.
pub const IC_FIELD_KIND: u8 = u8::MAX;

/// `ICSlot::recv_kind` value marking a *fused-instantiation verdict* entry
/// (`BranchIfNotPlainNew`, M2): `recv_ptr` holds the receiver-class pointer and
/// `arg_ptrs[0]` the cached verdict (1 = plain `Callable::New` + instantiable).
/// Same sharing rules as field entries: the branch instruction never shares an
/// `ip` with a send, and a verdict entry can never satisfy `ic_probe`.
pub const IC_PLAINNEW_KIND: u8 = u8::MAX - 1;

/// Refusal hint for a native class that didn't name its constructors
/// ([`NativeNewPolicy::Refuse`]`(None)`, the builder default).
const NATIVE_NEW_GENERIC_HINT: &str =
    "it has no default constructor (see its class-side constructor methods)";

/// One compiled outcall site's direct-dispatch cache (D2, the "AOT IC" —
/// docs/OUTCALL_ARCH.md): the same epoch + receiver/arg type-shape guards as
/// [`ICSlot`] (dispatch is multimethod — arg shapes select typed variants),
/// but resolving straight to the compiled entry plus the callee block's
/// lexical env, so a warm compiled→compiled call skips the `ic_registry`
/// hash, the IC borrow/probe, and the whole `Callable` dispatch. Filled only
/// when the interpreted IC at the same `(template, ip)` also filled
/// (probe-after-fill), so the cacheability rules can never drift from
/// `ic_fill_cell`'s. Indexed by a translation-minted site id — an array
/// index, never a hash.
#[derive(Collect, Clone, Copy)]
#[collect(no_drop)]
pub struct AotSiteCell<'gc> {
    epoch: u64,
    /// D3a: fast-path hit streak since fill (bump-free). Crossing
    /// `QN_DIRECT_WARM` queues the CALLER for retranslation (§3.3).
    pub hits: u32,
    recv_kind: u8,
    recv_ptr: usize,
    n_args: u8,
    arg_kinds: [u8; IC_MAX_ARGS],
    arg_ptrs: [usize; IC_MAX_ARGS],
    #[collect(require_static)]
    pub entry: Option<&'static crate::codegen::AotEntry>,
    pub parent_env: Option<Gc<'gc, RefLock<EnvFrame<'gc>>>>,
    /// Block-call sites: the receiver closure observed at fill (identity
    /// bake source; rooted here while the cell lives, then pinned into
    /// `aot_baked_roots` when an edge bakes it).
    pub recv_val: Option<Value<'gc>>,
}

impl Default for AotSiteCell<'_> {
    fn default() -> Self {
        AotSiteCell {
            epoch: 0,
            hits: 0,
            recv_kind: 0,
            recv_ptr: 0,
            n_args: 0,
            arg_kinds: [0; IC_MAX_ARGS],
            arg_ptrs: [0; IC_MAX_ARGS],
            entry: None,
            parent_env: None,
            recv_val: None,
        }
    }
}

/// A monomorphic inline-cache entry: a resolved method memoized at one call site. Lives in the
/// executing [`Block`]'s per-`ip` cache array ([`Block::inline_cache`]), so the block+ip *is*
/// the call-site identity — and because the executing block roots its own array, there is no
/// pointer-reuse (ABA) to guard against. A hit still requires a live `epoch` (method tables
/// unchanged) and matching receiver + argument type-shape guards `(kind, ptr)`; for immediates
/// the `kind` alone fixes the class, so the probe reads a cheap `Value` discriminant instead of
/// deriving it. `epoch == 0` is an empty slot. Only guard-free resolutions are stored.
#[derive(Clone, Collect)]
#[collect(no_drop)]
pub struct ICSlot<'gc> {
    #[collect(require_static)]
    pub epoch: u64,
    #[collect(require_static)]
    pub recv_kind: u8,
    #[collect(require_static)]
    pub recv_ptr: usize,
    #[collect(require_static)]
    pub n_args: u8,
    #[collect(require_static)]
    pub arg_kinds: [u8; IC_MAX_ARGS],
    #[collect(require_static)]
    pub arg_ptrs: [usize; IC_MAX_ARGS],
    pub callable: Option<Callable<'gc>>,
}

impl<'gc> std::fmt::Debug for ICSlot<'gc> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Deliberately does not recurse into `callable` (which would require `Callable: Debug`,
        // and `Callable` holds a `Gc<Block>` → a `Block: Debug` cycle back through this field).
        write!(
            f,
            "ICSlot {{ epoch: {}, cached: {} }}",
            self.epoch,
            self.callable.is_some()
        )
    }
}

impl<'gc> ICSlot<'gc> {
    pub const fn empty() -> Self {
        Self {
            epoch: 0,
            recv_kind: 0,
            recv_ptr: 0,
            n_args: 0,
            arg_kinds: [0; IC_MAX_ARGS],
            arg_ptrs: [0; IC_MAX_ARGS],
            callable: None,
        }
    }
}

/// The `(kind, ptr)` type-shape guard for a value: `kind` is the `Value` discriminant
/// (immediates distinguished down to `true`/`false`, so it alone fixes the class), `ptr` is
/// the class pointer for objects/classes (`0` for immediates). Matches what
/// `MethodCacheKey` keys on, so a guard match ⇒ the same dispatch result.
#[inline]
pub(crate) fn value_type_guard<'gc>(v: Value<'gc>) -> (u8, usize) {
    match v {
        Value::Int(_) => (0, 0),
        Value::Double(_) => (1, 0),
        Value::Bool(true) => (2, 0),
        Value::Bool(false) => (3, 0),
        Value::Nil => (4, 0),
        Value::Object(o) => (5, Gc::as_ptr(o.borrow().class) as usize),
        Value::Class(c) => (6, Gc::as_ptr(c) as usize),
        Value::ClassMeta(c) => (7, Gc::as_ptr(c) as usize),
    }
}

/// A method call queued to run when its frame completes normally (a "defer").
#[derive(Clone, Collect)]
#[collect(no_drop)]
pub struct DeferredCall<'gc> {
    pub receiver: Value<'gc>,
    #[collect(require_static)]
    pub selector: String,
    pub args: Vec<Value<'gc>>,
}

#[derive(Collect)]
#[collect(no_drop)]
pub struct Frame<'gc> {
    pub id: usize,
    pub is_nested_block: bool,
    pub enclosing_method_id: Option<usize>,
    pub block: Gc<'gc, Block<'gc>>,
    /// `block.inline_cache`, hoisted at frame push: the probe on every send/field
    /// reads the cell without the extra hop through the (template-thin) `Block`.
    pub ic: InlineCacheCell<'gc>,
    pub ip: usize,
    pub env: Gc<'gc, RefLock<EnvFrame<'gc>>>,
    pub instantiating_obj: Option<Gc<'gc, RefLock<Object<'gc>>>>,
    pub receiver: Option<Value<'gc>>,
    pub selector: Option<Symbol>,
    pub args: Vec<Value<'gc>>,
    pub stack_base: usize,
    /// Speculative-AOT (S0): the template id iff this is a method frame
    /// whose template was OBSERVING at push time (0 = not observing; real ids
    /// start at 1) — computed once at push, so the pop-side return
    /// observation is an in-struct integer test, not a pointer chase. Packed
    /// as a bare u32 to ride Frame padding.
    pub spec_tid: u32,
    pub return_receiver: bool,
    /// Calls queued (e.g. by `mix:`) to run when this frame returns normally.
    pub defers: Vec<DeferredCall<'gc>>,
    /// If set, and a deferred call throws, remove this global before propagating
    /// (used so a class whose mixin requirements fail is never left registered).
    #[collect(require_static)]
    pub unregister_on_defer_failure: Option<NamespacedName>,
}

/// A live compiled METHOD invocation, addressable as a `^^` target (S5).
/// Compiled methods push no interpreter [`Frame`]; this is the sliver the
/// `MethodReturn` unwind needs instead: `frames_len`/`stack_base` snapshot the
/// interpreter stacks at entry, so the unwind pops outcall frames down to
/// `frames_len` and delivers the value at `stack_base` (the frame's slot-window
/// base). `id` is minted from `next_frame_id` — one counter for interpreter
/// frames and compiled invocations, so a target id is never ambiguous.
#[derive(Clone, Copy)]
pub struct AotFrameMark {
    pub id: usize,
    pub frames_len: usize,
    pub stack_base: usize,
}

/// The PER-TASK slice of compiled-execution state, swapped with the task
/// context as ONE unit (`std::mem::take` in `save_task_context` /
/// `load_task_context`). Every field here describes something frozen on the
/// task's own coroutine stack while it parks — fuel/depth budgets, the
/// compiled-call nesting count, the lexical/`^^` context of in-flight
/// compiled frames — so leaking any of it to the next task corrupts that
/// task's compiled execution (the `aot.enclosing_env` and `outcall_nesting`
/// bugs, found one arc apart). ADDING A FIELD HERE is the whole protocol:
/// the swap sites move the struct wholesale and `Default` covers fresh
/// tasks and resets. Process-global AOT state (pending candidates, the
/// observation budget, `aot_pending_error` — set and taken within one
/// straight-line `codegen::invoke`) stays on `VmState` directly;
/// `native_reentry_depth` predates this struct and rides the task context
/// as its own field.
#[derive(Collect, Default)]
#[collect(no_drop)]
pub struct AotTaskState<'gc> {
    /// Fuel/depth counters (docs/AOT_ARCH.md §5): compiled code decrements
    /// `fuel` in every prologue and checkpoints (cancellation + cooperative
    /// yield) at zero; `depth` caps compiled-call recursion on the real
    /// coroutine stack (which bypasses `MAX_NATIVE_REENTRY`).
    #[collect(require_static)]
    pub fuel: i64,
    #[collect(require_static)]
    pub depth: i64,
    /// Rust-stack nesting depth of compiled-call re-entries (outcalls into
    /// `call_method_cached`); dispatch stops entering compiled bodies past
    /// `spec::MAX_OUTCALL_NESTING` (see there).
    #[collect(require_static)]
    pub outcall_nesting: u32,
    /// The ENCLOSING lexical environment of the currently-executing compiled
    /// frame (the invoked block/method's own `parent_env`) — the parent a
    /// cold-path `make_closure` snapshot must chain to, so a materialized
    /// closure's free names resolve through the full lexical chain exactly
    /// as interpreted (B3b). Saved/restored around each compiled invocation.
    pub enclosing_env: Option<Gc<'gc, RefLock<EnvFrame<'gc>>>>,
    /// The `^^` home a `make_closure`-materialized closure must carry as its
    /// `enclosing_method_id` (S5): compiled METHOD invocations mint a frame
    /// id and put it here; a compiled BLOCK template propagates the invoked
    /// closure's own home. Saved/restored around each compiled invocation.
    #[collect(require_static)]
    pub home_frame_id: Option<usize>,
    /// The live compiled METHOD invocations on this task, addressable as
    /// `^^` targets (S5): a compiled method has no interpreter `Frame`, so
    /// the `MethodReturn` unwind finds it here. Pushed/popped by
    /// `codegen::invoke`; entries index this task's `frames`/`stack`.
    #[collect(require_static)]
    pub frame_marks: Vec<AotFrameMark>,
    /// Set by the `MethodReturn` unwind when the `^^` home is a live
    /// compiled invocation: the delivered value sits at that frame's window
    /// base, and the matching `codegen::invoke` consumes both as its
    /// ordinary return.
    #[collect(require_static)]
    pub nlr_target: Option<usize>,
}

#[derive(Collect)]
#[collect(no_drop)]
pub struct BuiltinCache<'gc> {
    pub nil_class: Option<Gc<'gc, RefLock<Class<'gc>>>>,
    pub boolean_class: Option<Gc<'gc, RefLock<Class<'gc>>>>,
    pub integer_class: Option<Gc<'gc, RefLock<Class<'gc>>>>,
    pub double_class: Option<Gc<'gc, RefLock<Class<'gc>>>>,
    pub string_class: Option<Gc<'gc, RefLock<Class<'gc>>>>,
    pub list_class: Option<Gc<'gc, RefLock<Class<'gc>>>>,
    pub map_class: Option<Gc<'gc, RefLock<Class<'gc>>>>,
    pub regex_class: Option<Gc<'gc, RefLock<Class<'gc>>>>,
    pub block_class: Option<Gc<'gc, RefLock<Class<'gc>>>>,
    // `true <-- {…}` / `false <-- {…}` need separate method tables; an immediate
    // carries no per-instance class, so the synthesized singletons live here.
    pub true_class: Option<Gc<'gc, RefLock<Class<'gc>>>>,
    pub false_class: Option<Gc<'gc, RefLock<Class<'gc>>>>,
}

impl<'gc> BuiltinCache<'gc> {
    pub fn new() -> Self {
        Self {
            nil_class: None,
            boolean_class: None,
            integer_class: None,
            double_class: None,
            string_class: None,
            list_class: None,
            map_class: None,
            regex_class: None,
            block_class: None,
            true_class: None,
            false_class: None,
        }
    }
}

#[derive(Clone, Debug, Default, Collect)]
#[collect(require_static)]
pub struct VmOptions {
    pub arguments: Vec<String>,
    pub supports_color: bool,
    pub console_width: Option<u16>,
    /// Shared compile-time class-name accumulator (Phase 2). Threaded into every `Compiler`
    /// this VM spawns for `use`-loads, and used by the runner for the top-level program, so a
    /// unit sees the classes earlier-compiled units defined. Not a runtime knob — it rides
    /// here because `VmOptions` is the value already cloned into every VM.
    pub seen_types: crate::types::SeenTypes,
    /// Shared compile-time class-signature table (Phase 3b) — parallel to `seen_types`, threaded
    /// the same way. Carries parent/mixins/method-set/sealed for cross-class checks (subtyping,
    /// MNU).
    pub class_table: crate::class_table::ClassTable,
    /// Directory that `use self:…` resolves against: the entry script's directory, so a
    /// script means the same thing wherever it is invoked from. Empty (the default) is
    /// CWD-relative, which is what the script-less modes want (`repl`, `-e`, `test`).
    pub self_root: std::path::PathBuf,
}

// The scheduler / task / guest-fiber subsystem lives in `vm_scheduler.rs` (still
// intrinsically VM state); its public types are re-exported here so callers that
// `use crate::vm::{Task, Wake, ...}` are unaffected by the move.
pub use crate::vm_scheduler::{GatherState, Scheduler, Task, TaskId, Wake};

/// Which standard stream a write targets. Lets the output sink tag captured output with the
/// matching DAP `output`-event category and route it instead of touching fd 1/2.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StdStream {
    Out,
    Err,
}

/// A captured chunk of program output, buffered when `VmState.capture_output` is on and later
/// drained (by the DAP driver) into `output` events. Plain `'static` data — no `Gc`.
#[derive(Debug)]
pub struct OutputChunk {
    pub stream: StdStream,
    pub bytes: Vec<u8>,
}

/// In-flight exception + `catch:`/`throw` unwind bookkeeping, grouped out of `VmState` (cf.
/// [`Scheduler`]). Stored inline by value.
#[derive(Collect)]
#[collect(no_drop)]
pub struct Exceptions<'gc> {
    /// The exception currently being raised / handled, or `None`.
    pub active: Option<Value<'gc>>,
    /// Arguments of the most recent send that failed *in place* (no callee frame of its own) — read
    /// only by the stack-trace formatter (`annotate_error`). Set on the in-place-error branches of
    /// the `Send` handler / `Callable::call`, not on every send.
    pub last_send_args: Vec<Value<'gc>>,
    /// Declared exception types of each enclosing `catch:` whose protected block is currently running
    /// (one `Vec` per active catch; `"Object"` marks a catch-all). The debugger's break-on-uncaught
    /// searches this at a throw. Pushed/popped by the `catch` natives. Plain strings — no `Gc`.
    pub handler_stack: Vec<Vec<String>>,
    /// Set when a typed `catch:` re-raises (no handler matched), so break-on-uncaught fires once at
    /// the innermost throw site rather than again at every catch it bubbles through.
    pub reraised: bool,
}

/// `use` / package resolution + load-once state, grouped out of `VmState`.
#[derive(Collect)]
#[collect(no_drop)]
pub struct Modules<'gc> {
    /// Resolves `use (pkg:)? path` to source — the filesystem-agnostic seam, swappable per host (FS
    /// on the CLI, in-memory on WASM/embedded). See `src/packages.rs`.
    #[collect(require_static)]
    pub resolver: Box<dyn PackageResolver>,
    /// Run-once registry for `use`, in load order (a `Vec`, not a set: run order *is* load order). A
    /// per-entry status breaks cycles. See `USE_ARCH.md`.
    #[collect(require_static)]
    pub loaded: Vec<LoadedUnit>,
    /// Loaded extension *packages* (`Extension loadPackage:`), keyed by canonical package directory →
    /// the live `Extension` value, so a repeat `loadPackage:` of the same folder is idempotent. The
    /// installed classes also root the extension, but this is its canonical owner for the session.
    /// See `src/runtime/extension.rs` / `docs/EXT_PACKAGING.md`.
    pub packages: Gc<'gc, RefLock<HashMap<String, Value<'gc>>>>,
}

/// Output-capture redirect: when on (the DAP adapter sets it), `write_std` buffers `[IO]Handle`
/// stdout/stderr writes into `chunks` instead of fd 1/2 — so the debuggee's output becomes DAP
/// `output` events rather than corrupting the protocol stream. Off by default. No `Gc` → static.
pub struct OutputCapture {
    pub capture: bool,
    pub chunks: Vec<OutputChunk>,
}

/// Memoized method resolution, grouped out of `VmState`.
#[derive(Collect)]
#[collect(no_drop)]
pub struct DispatchCache<'gc> {
    /// `(searched-class ptr, selector, class-side, arg-class ptrs)` → resolved method (or `None`
    /// when the hierarchy has no match). Populated by `lookup_method_in_class_hierarchy` for
    /// guard-free, non-eigenclass lookups; cleared whenever a class's method table changes
    /// (`invalidate_method_cache`). Traced so cached `Value`s stay live; the key's class *pointers*
    /// are sound because named classes are globals-rooted. All-integer key → `FxHashMap`.
    pub entries: FxHashMap<MethodCacheKey, Option<Value<'gc>>>,
    /// Scratch flag marking the in-progress lookup's result un-memoizable (a guarded candidate was
    /// examined — its outcome depends on argument values, not just types). Saved/restored around each
    /// `lookup_method_in_class_hierarchy` call for re-entrancy.
    #[collect(require_static)]
    pub uncacheable: bool,
}

/// The session's async-I/O backend plus the deferred resource-reap queues, grouped out of `VmState`.
/// All `Rc`/non-`Gc` → static.
pub struct Io {
    /// The reactor + the `StreamId -> fd` registry. Held here so it **persists across separate driver
    /// runs** — most importantly the REPL, where a long-lived resource opened on one line (an
    /// extension socket, a file, a TCP/TLS connection) must survive into the next.
    pub backend: crate::io_backend::SmolBackend,
    /// fds whose QN `TcpSocket` handle has been closed or collected, awaiting a synchronous
    /// `IoBackend::close` by the driver. A non-GC queue (the handle's `Drop` can only push a plain
    /// `StreamId`); a shared `Rc` clone lives in each socket handle. See `docs/ASYNC_ARCH.md`.
    pub socket_reap: std::rc::Rc<std::cell::RefCell<Vec<StreamId>>>,
    /// Extension ids whose `Extension` handle was dropped (GC'd), awaiting bulk-release of the
    /// host-value handles they held (`HandleTable::release_for_ext`). A non-GC queue mirroring
    /// `socket_reap`; a shared `Rc` clone lives in each `Extension` handle.
    pub ext_handle_reap: std::rc::Rc<std::cell::RefCell<Vec<u64>>>,
}

/// Per-instruction instrumentation hooks, grouped out of `VmState`. Both `None` on a normal run, so
/// the hot-path cost is a single bool load each. Plain data (no `Gc`) → static.
pub struct Instrumentation {
    /// Attached debugger session (see `src/debug.rs`), consulted once per instruction when `Some`.
    pub debug: Option<crate::debug::DebugState>,
    /// Active Quoin-level coverage collector (see `src/coverage.rs`), the same hot-path cost model.
    pub coverage: Option<crate::coverage::CoverageState>,
}

/// Non-hot, non-GC per-class metadata (see `VmState::class_meta`): where the class was
/// defined, and — for a native class — its `.class_doc(..)` text. Quoin classes get their doc
/// lazily, from the `"*` block above `source` (docs/DOCS_ARCH.md §4); native classes carry it
/// here because they have no source to scan.
#[derive(Default, Clone, Debug)]
pub struct ClassMeta {
    pub source: Option<SourceInfo>,
    pub doc: Option<String>,
    /// Every statically-named reopen site (`Name <-- { … }`), in load order. The doc block
    /// above a reopen documents the extension; for a native class this is where its qnlib
    /// class doc lives.
    pub extensions: Vec<SourceInfo>,
}

#[derive(Collect)]
#[collect(no_drop)]
pub struct VmState<'gc> {
    pub stack: crate::value::SlotStack<'gc>,
    pub frames: Vec<Frame<'gc>>,
    /// FxHash, not SipHash: `LoadGlobal` probes this once per instantiation
    /// (`TreeNode` etc. resolve here before every `.new:`), the last
    /// default-hasher map on the allocation path.
    pub globals: Gc<'gc, RefLock<FxHashMap<NamespacedName, Value<'gc>>>>,
    /// Intern pool for symbols: one canonical `Symbol` value per name, so symbols
    /// compare by identity. Rooted here and traced as part of `VmState`.
    pub symbol_table: Gc<'gc, RefLock<HashMap<String, Value<'gc>>>>,
    /// Shared INNER BUFFERS for string literals: one `Gc<String>` per
    /// distinct literal content, so materializing a literal costs one Object
    /// alloc + a pointer instead of two GC allocs + a byte copy per push.
    /// Only the buffer is shared — each push still mints a fresh Object
    /// wrapper, because string VALUES have observable identity (a user can
    /// eigenclass one: `s <-- {...}`), while the immutable payload does not.
    /// Bounded by the program's distinct literals.
    pub string_literal_buffers: FxHashMap<String, Gc<'gc, String>>,
    /// Name of the class just created by `DefineClass`, consumed by the next
    /// `ExecuteBlockWithSelf` to mark the class body's frame for unregister-on-
    /// defer-failure. Only a *new* class definition sets this (not an extension).
    #[collect(require_static)]
    pub pending_class_def: Option<NamespacedName>,
    pub next_frame_id: usize,
    /// Depth of native → Quoin re-entry currently on the *real* Rust/C stack: each
    /// `call_method` / `call_method_value` / `execute_block` from native code drives a
    /// nested `step` loop on a native frame that does not return until its subtree
    /// completes. Pure-Quoin recursion grows the heap frame stack (bounded, catchable),
    /// but native re-entry grows the machine stack, so unbounded re-entry — a custom
    /// `==:` / `hash` / comparator / render hook that re-enters the same native op —
    /// overflows it and aborts the process uncatchably. Bumped on entry, dropped on
    /// exit (via `NativeReentryGuard`), and capped at `MAX_NATIVE_REENTRY`.
    #[collect(require_static)]
    pub native_reentry_depth: usize,
    /// The process's standard input, as a stream, created on first use and reused forever after.
    ///
    /// Memoized because a stream *buffers*: two streams over fd 0 would each hold bytes the other
    /// never sees, so `readLine` twice through two handles would silently drop input. Lazy because
    /// opening it is an `await_io`, and the benchmark harness (which still runs the prelude) has
    /// no scheduler to park on.
    pub stdin_stream: Option<Value<'gc>>,
    /// Per-class metadata that lives outside the hot `Class` struct: the source location of
    /// the definition (recorded by `DefineClass`, used by doc extraction to find the `"*`
    /// block above it) and a native class's `.class_doc(..)` text. A side table rather than
    /// fields on `Class` so the many `Class` construction sites stay untouched.
    #[collect(require_static)]
    pub class_meta: FxHashMap<NamespacedName, ClassMeta>,
    /// Every open *buffered* write stream (`[IO]File.create:` / `append:`), so the driver can
    /// flush them when the program ends — C's atexit flush, which a GC finaliser cannot do
    /// because a `Drop` may not perform async I/O. `close` removes its stream from this list.
    /// Rooted: a stream a program stops referencing must still be flushed, not collected with
    /// its tail unwritten. Signal death still loses the buffer, as it does in C.
    pub open_write_streams: Vec<Value<'gc>>,
    /// Lowest usable address of the coroutine stack currently running (`Fiber::stack_limit`),
    /// refreshed by the driver before every `resume`. `0` means "not on a known coroutine"
    /// (the benchmark harness steps the VM straight on the OS thread stack), which disables
    /// the check. Read by `ensure_stack_headroom`.
    #[collect(require_static)]
    pub stack_limit: usize,
    /// Set by `Runtime.exit:` — the guest requested process exit with this status.
    /// The raising task also unwinds with `QuoinError::ExitRequested`; this flag is
    /// what makes the exit PROCESS-wide: the driver checks it each loop iteration,
    /// so an exit requested inside a spawned task (whose unwind lands in the task's
    /// join result, not the driver) still stops the world promptly.
    #[collect(require_static)]
    pub requested_exit: Option<i32>,

    /// The per-task compiled-execution slice (see [`AotTaskState`]) —
    /// swapped as ONE unit with the task context, since another task may run
    /// while this one is suspended at a checkpoint or parked mid-outcall.
    pub aot: AotTaskState<'gc>,
    /// Error channel for compiled code (docs/AOT_ARCH.md v0.2): helpers store a
    /// full `QuoinError` here and return `TAG_ERR`; `codegen::invoke` takes it.
    /// A thrown Quoin *value* needs no slot here — it travels as
    /// `QuoinError::Thrown` with the value GC-rooted in `exceptions.active`,
    /// exactly as across any native boundary.
    #[collect(require_static)]
    pub aot_pending_error: Option<QuoinError>,
    /// Block templates collected at unit load but NOT yet compiled (B3a lazy
    /// compilation): most literals are never invoked, and eager Cranelift
    /// work for all of them cost ~+34ms startup. A template compiles once
    /// warm at the `valueWithSelfOrArg:` seam (`codegen::block_entry_for`);
    /// refusals tombstone so they never retry. The tuple is (invocations,
    /// observed arg-kind lattice, candidate): the warmth window doubles as
    /// the block's S1-style argument observation, so a monomorphic-scalar
    /// block compiles its param into a register lane with an entry
    /// precondition instead of a slot-resident Obj.
    #[collect(require_static)]
    pub aot_pending_blocks: rustc_hash::FxHashMap<u32, (u32, u8, crate::codegen::AotCandidate)>,
    #[collect(require_static)]
    pub aot_refused_blocks: rustc_hash::FxHashSet<u32>,
    /// Speculative pending methods: template id → warmth + kind profile +
    /// the candidate S1 will compile from it. The fast-path observation gate
    /// is NOT here — it is `aot_spec_obs_left` below plus the `spec_state`
    /// Cell on each `StaticBlock`.
    #[collect(require_static)]
    pub aot_pending_spec: crate::codegen::spec::SpecPendingMap,
    /// Speculative methods promoted to compiled entries (S1) — stats only.
    #[collect(require_static)]
    pub aot_spec_promoted: u32,
    /// Remaining process-wide observation budget (spec::OBSERVE_BUDGET).
    /// Checked FIRST at method entry — one load from this hot struct — so
    /// once spent, observation costs one predicted branch per call, total.
    #[collect(require_static)]
    pub aot_spec_obs_left: u32,

    pub builtin_cache: Gc<'gc, RefLock<BuiltinCache<'gc>>>,
    pub active_native_args: Vec<NativeCall<'gc>>,
    /// Root slot for [`InitPlan`]s whose init chains are RUNNING: a user
    /// init can park, and can invalidate/replace the class's cached plan
    /// (reopen bumps the epoch) — the running iteration's plan must stay
    /// alive regardless. Pushed/popped around each chain.
    pub active_init_plans: Vec<Gc<'gc, InitPlan<'gc>>>,
    pub last_popped_env: Option<Gc<'gc, RefLock<EnvFrame<'gc>>>>,

    /// The REPL's persistent top-level environment. `Some` only under `qn repl`: each
    /// evaluated line runs in a frame whose env *is* this one (not a fresh child), so
    /// top-level `x = 5` binds here and is visible on later lines. GC-rooted via `VmState`
    /// so it survives between `arena.mutate_root` calls. `None` in every other mode.
    pub repl_env: Option<Gc<'gc, RefLock<EnvFrame<'gc>>>>,

    /// Coroutine / guest-fiber scheduler state, grouped out for legibility (see
    /// the [`Scheduler`] struct). Stored inline by value — no indirection.
    pub sched: Scheduler<'gc>,

    #[collect(require_static)]
    pub options: VmOptions,

    /// In-flight exception + `catch:`/`throw` unwind state ([`Exceptions`]).
    pub exceptions: Exceptions<'gc>,
    /// `use` / package resolution + load-once state ([`Modules`]).
    pub modules: Modules<'gc>,
    /// Output-capture redirect for the DAP adapter ([`OutputCapture`]).
    #[collect(require_static)]
    pub output: OutputCapture,
    /// Memoized method resolution ([`DispatchCache`]).
    pub dispatch_cache: DispatchCache<'gc>,
    /// Shared inline-cache arrays keyed by block-literal template id: every closure
    /// materialized from the same literal gets the same cell, so its call sites stay
    /// warm across re-materialization. Rooted here for the VM's lifetime and ids are
    /// never reused, so `(template_id, ip)` is a stable call-site identity (no ABA);
    /// stale entries self-evict via `dispatch_epoch`.
    pub ic_registry: FxHashMap<u32, Gc<'gc, RefLock<Option<Box<[ICSlot<'gc>]>>>>>,
    /// D2 site-cache cells, indexed by translation-minted outcall-site id.
    pub aot_sites: Vec<AotSiteCell<'gc>>,
    /// D3a retranslation queue: caller tids whose sites crossed
    /// `QN_DIRECT_WARM`, drained at the driver boundary (never inside a VM
    /// step). The queued set dedups for the process lifetime — one
    /// retranslation per tid until an epoch-driven refill re-queues (D3b).
    pub aot_retranslate_queue: Vec<u32>,
    /// A3: values PINNED by baked direct edges (a native identity guard
    /// compares a slot's 16 bytes against a baked Gc pointer — the pointee
    /// must stay alive for the code's lifetime or a recycled address could
    /// false-positive the guard). Append-only, bounded by baked edges.
    pub aot_baked_roots: Vec<Value<'gc>>,
    /// Constant-closure promotion: CLOSED templates (no captures, no self,
    /// no `^^`) materialize ONE closure per VM, shared by the interpreter
    /// and compiled make_closure alike. Gives hot loops allocation-free
    /// block arguments and makes the baked identity guards durable across
    /// calls. Observable: two evaluations of a closed literal are `==`.
    pub aot_closure_cache: rustc_hash::FxHashMap<u32, Value<'gc>>,
    pub aot_retranslate_queued: rustc_hash::FxHashSet<u32>,
    /// Bumped on any method-table change; a stored `epoch` mismatch self-evicts every
    /// per-`Block` [`ICSlot`] at once, giving O(1) inline-cache invalidation.
    #[collect(require_static)]
    pub dispatch_epoch: u64,
    /// The session's async-I/O backend + the deferred resource-reap queues ([`Io`]).
    #[collect(require_static)]
    pub io: Io,
    /// Set at boot on a WORKER's VM (docs/CONCURRENCY_ARCH.md §5): the
    /// channel ends back to the parent. `None` on the main VM.
    #[collect(require_static)]
    pub worker_link: Option<crate::worker::WorkerLink>,
    /// The ENTRY unit this VM was booted to run (canonicalized), `None` for
    /// REPL/eval. "What program is this?" — `Worker.spawn:(VM.unit)` runs
    /// another copy of the current program (the same-unit provisioning
    /// model, docs/WEB_ARCH.md workers). Deliberately NOT `__FILE__`: a
    /// `use`d library sees the app's unit, not its own file.
    pub unit_path: Option<String>,
    /// Workers this VM spawned (`VM.ps` observability; see `WorkerReg`).
    #[collect(require_static)]
    pub worker_registry: Vec<crate::worker::WorkerReg>,
    /// Per-instruction instrumentation hooks — debugger + coverage ([`Instrumentation`]).
    #[collect(require_static)]
    pub instrumentation: Instrumentation,

    /// Opaque `u64` handles for host values held by out-of-process extensions (Tier 1).
    /// Inline by value so `#[derive(Collect)]` traces it: the table **is a GC root set**,
    /// keeping a handle's `Value` alive as long as the extension holds it. Empty (and a
    /// single bool load on the hot path) unless extensions are in use. See
    /// `src/handle_table.rs` / `docs/FUTURE_EXT_ARCH.md` §2.
    pub handle_table: crate::handle_table::HandleTable<'gc>,
}

pub enum VmStatus<'gc> {
    Running,
    Finished(Value<'gc>),
    Yeeted(Value<'gc>), // Uncaught exception
}

impl<'gc> VmState<'gc> {
    pub unsafe fn get_yielder(&self) -> Option<&VMYielder<'gc>> {
        self.sched
            .yielder
            .map(|ptr| unsafe { &*(ptr as *const VMYielder<'gc>) })
    }

    /// Record the running coroutine's yielder into the current fiber's slot (or
    /// the main slot) and make it live. Called once at the top of `run_vm_loop`.
    pub fn register_yielder(&mut self, mc: &Mutation<'gc>, ptr: *const ()) {
        match self.sched.current_fiber {
            None => {
                if let Some(task) = self
                    .sched
                    .tasks
                    .get_mut(self.sched.current_task.0)
                    .and_then(|t| t.as_mut())
                {
                    task.root_yielder = Some(ptr);
                }
            }
            Some(f) => {
                let _ =
                    f.with_native_state_mut::<NativeFiberState, _, _>(mc, |s| s.set_yielder(ptr));
            }
        }
        self.sched.yielder = Some(ptr);
    }

    /// The stored yielder for whichever fiber is current (main if `None`). The
    /// driver loads this into `self.sched.yielder` before resuming, guaranteeing it
    /// always points at the live, GC-rooted coroutine being run.
    pub fn current_fiber_yielder(&self) -> Option<*const ()> {
        match self.sched.current_fiber {
            None => self
                .sched
                .tasks
                .get(self.sched.current_task.0)
                .and_then(|t| t.as_ref())
                .and_then(|t| t.root_yielder),
            Some(f) => f
                .with_native_state::<NativeFiberState, _, _>(|s| s.yielder())
                .ok()
                .flatten(),
        }
    }

    pub fn new(mc: &Mutation<'gc>, options: VmOptions) -> Self {
        Self {
            stack: crate::value::SlotStack::new(),
            frames: Vec::new(),
            globals: gcl!(mc, FxHashMap::default()),
            symbol_table: gcl!(mc, HashMap::new()),
            string_literal_buffers: FxHashMap::default(),
            pending_class_def: None,
            next_frame_id: 1,
            native_reentry_depth: 0,
            stdin_stream: None,
            class_meta: FxHashMap::default(),
            open_write_streams: Vec::new(),
            stack_limit: 0,
            requested_exit: None,
            aot: AotTaskState::default(),
            aot_pending_error: None,
            aot_pending_blocks: rustc_hash::FxHashMap::default(),
            aot_refused_blocks: rustc_hash::FxHashSet::default(),
            aot_pending_spec: crate::codegen::spec::SpecPendingMap::default(),
            aot_spec_promoted: 0,
            aot_spec_obs_left: crate::codegen::spec::OBSERVE_BUDGET,
            builtin_cache: gcl!(mc, BuiltinCache::new()),
            active_native_args: Vec::new(),
            active_init_plans: Vec::new(),
            last_popped_env: None,
            repl_env: None,
            sched: Scheduler {
                yielder: None,
                tasks: Vec::new(),
                ready: VecDeque::new(),
                read_scratch: Vec::new(),
                current_task: TaskId(0),
                active_fiber: None,
                current_fiber: None,
                resume_stack: Vec::new(),
                fiber_transfer: None,
                main_saved_stack: Vec::new(),
                main_saved_frames: Vec::new(),
                main_saved_native_args: Vec::new(),
                main_saved_aot: AotTaskState::default(),
                fiber_error: None,
                wake: None,
                cancel_current: false,
                park_seq: 0,
            },
            exceptions: Exceptions {
                active: None,
                last_send_args: Vec::new(),
                handler_stack: Vec::new(),
                reraised: false,
            },
            modules: Modules {
                resolver: Box::new(FsResolver::new(options.self_root.clone())),
                loaded: Vec::new(),
                packages: gcl!(mc, HashMap::new()),
            },
            output: OutputCapture {
                capture: false,
                chunks: Vec::new(),
            },
            ic_registry: FxHashMap::default(),
            aot_sites: Vec::new(),
            aot_retranslate_queue: Vec::new(),
            aot_baked_roots: Vec::new(),
            aot_closure_cache: rustc_hash::FxHashMap::default(),
            aot_retranslate_queued: rustc_hash::FxHashSet::default(),
            dispatch_cache: DispatchCache {
                entries: FxHashMap::default(),
                uncacheable: false,
            },
            // Epoch starts at 1 so the epoch-0 empty slots never spuriously match.
            dispatch_epoch: 1,
            worker_link: None,
            unit_path: None,
            worker_registry: Vec::new(),
            io: Io {
                backend: crate::io_backend::SmolBackend::new(),
                socket_reap: std::rc::Rc::new(std::cell::RefCell::new(Vec::new())),
                ext_handle_reap: std::rc::Rc::new(std::cell::RefCell::new(Vec::new())),
            },
            instrumentation: Instrumentation {
                debug: None,
                coverage: None,
            },
            options,
            handle_table: crate::handle_table::HandleTable::new(),
        }
    }

    /// Write `bytes` to a standard stream, honoring the output-capture redirect: when
    /// `capture_output` is on (set by the DAP adapter), buffer the chunk for the driver to emit
    /// as an `output` event; otherwise write straight to the real fd. The single chokepoint for
    /// `[IO]Handle`'s stdout/stderr writes (see `src/runtime/io.rs`).
    pub fn write_std(&mut self, stream: StdStream, bytes: &[u8]) -> std::io::Result<()> {
        if self.output.capture {
            self.output.chunks.push(OutputChunk {
                stream,
                bytes: bytes.to_vec(),
            });
            Ok(())
        } else {
            use std::io::Write;
            match stream {
                StdStream::Out => std::io::stdout().write_all(bytes),
                StdStream::Err => std::io::stderr().write_all(bytes),
            }
        }
    }

    /// `write_std` for *guest* output (`.print`, `[IO]Stdout`/`[IO]Stderr`): a broken pipe on
    /// a standard stream is the reader hanging up (`qn test | head`), not a program error —
    /// convert it to a quiet `Runtime.exit:`-style unwind with the conventional SIGPIPE status
    /// (128+13), uncatchable, running `finally` blocks and normal teardown on the way out.
    /// Sockets and files are unaffected: their broken pipes stay catchable `IoError`s.
    pub fn write_std_guest(&mut self, stream: StdStream, bytes: &[u8]) -> Result<(), QuoinError> {
        self.write_std(stream, bytes).map_err(|e| {
            if e.kind() == std::io::ErrorKind::BrokenPipe {
                self.requested_exit = Some(141);
                QuoinError::ExitRequested(141)
            } else {
                QuoinError::Other(e.to_string())
            }
        })
    }

    /// `file:line:col: <level>: <message>` header, the level keyword colored (yellow warning,
    /// red error, gray note) like uncaught errors. `indent` shifts a provenance note under its
    /// parent. Shared by `report_type_warnings` and `report_compile_error`.
    fn diag_header(
        level: &str,
        color: &str,
        message: &str,
        span: Option<&crate::value::SourceInfo>,
        colorize: bool,
        indent: bool,
    ) -> String {
        let pad = if indent { "  " } else { "" };
        let label = if colorize {
            ansi_colorizer::colorize(&format!("${color}[{level}$]"))
        } else {
            level.to_string()
        };
        match span {
            Some(s) => {
                // The `file:line:col` form colored exactly like a stack
                // trace's location (gray separators, cyan numbers).
                let loc = if colorize {
                    ansi_colorizer::colorize(&format!(
                        "{}$#808080[:$]$#00bfff[{}$]$#808080[:$]$#00bfff[{}$]",
                        s.filename,
                        s.line,
                        s.column + 1
                    ))
                } else {
                    format!("{}:{}:{}", s.filename, s.line, s.column + 1)
                };
                format!("{pad}{loc}: {label}: {message}\n")
            }
            None => format!("{pad}{label}: {message}\n"),
        }
    }

    /// The offending line under a gray `|` gutter with a caret beneath the span — the same
    /// visual language as an uncaught error's source block. `None` if the file can't be read.
    fn diag_source_block(span: &crate::value::SourceInfo, colorize: bool) -> Option<String> {
        let content = fs::read_to_string(&span.filename).ok()?;
        let line_text = content.lines().nth(span.line.saturating_sub(1))?;
        let width = content
            .get(span.start..span.end)
            .map(|s| s.chars().count())
            .unwrap_or(1)
            .max(1);
        let gutter = span.line.to_string();
        let pad = " ".repeat(gutter.len());
        let pipe = if colorize {
            ansi_colorizer::colorize("$#808080[|$]")
        } else {
            "|".to_string()
        };
        let line_hl = if colorize {
            highlight_to_ansi(line_text)
        } else {
            line_text.to_string()
        };
        let carets = format!("{}{}", " ".repeat(span.column), "^".repeat(width));
        let carets = if colorize {
            ansi_colorizer::colorize(&format!("$#ffcc00[{carets}$]"))
        } else {
            carets
        };
        Some(format!(
            "  {pad} {pipe}\n  {gutter} {pipe} {line_hl}\n  {pad} {pipe} {carets}\n"
        ))
    }

    /// Emit collected compile-time type diagnostics through the stderr sink (so under the DAP
    /// adapter, with `capture` on, they become `output` events rather than leaking to raw stderr).
    /// Each is rendered `file:line:col: warning: message` (the standard, editor-jumpable form) when
    /// a span is known, else bare `warning: message`. Best-effort; never fatal. (Phase 4.)
    pub fn report_type_warnings(&mut self, diagnostics: &[crate::compiler::Diagnostic]) {
        let colorize = self.options.supports_color;
        let mut out = String::new();
        for d in diagnostics {
            out.push_str(&Self::diag_header(
                "warning",
                "#ffcc00",
                &d.message,
                d.span.as_ref(),
                colorize,
                false,
            ));
            if let Some(s) = &d.span
                && let Some(block) = Self::diag_source_block(s, colorize)
            {
                out.push_str(&block);
            }
            // Why-chain notes (Phase 4 provenance): each under its own span, indented.
            for note in &d.notes {
                out.push_str(&Self::diag_header(
                    "note",
                    "#808080",
                    &note.message,
                    note.span.as_ref(),
                    colorize,
                    true,
                ));
                if let Some(s) = &note.span
                    && let Some(block) = Self::diag_source_block(s, colorize)
                {
                    out.push_str(&block);
                }
            }
        }
        let _ = self.write_std(StdStream::Err, out.as_bytes());
    }

    /// Report a fatal compile error through the stderr sink, `file:line:col: error: message`
    /// with the offending line and caret — the same visual language as `report_type_warnings`,
    /// at `error` level. Used by the file-based entry points (run, check, debug, benchmark);
    /// string-based modes (`-e`, REPL, `Runtime.eval:`, workers) use the error's `Display`,
    /// which embeds `(line …, column …)` the way those modes render parse errors.
    pub fn report_compile_error(&mut self, err: &crate::compiler::CompileError) {
        let colorize = self.options.supports_color;
        let mut out = Self::diag_header(
            "error",
            "#ff6961",
            &err.message,
            err.span.as_ref(),
            colorize,
            false,
        );
        if let Some(s) = &err.span
            && let Some(block) = Self::diag_source_block(s, colorize)
        {
            out.push_str(&block);
        }
        let _ = self.write_std(StdStream::Err, out.as_bytes());
    }

    /// Drain the captured program output (the DAP driver calls this between resumes to emit
    /// `output` events). Empty when nothing was captured.
    pub fn take_program_output(&mut self) -> Vec<OutputChunk> {
        std::mem::take(&mut self.output.chunks)
    }

    pub fn new_object(
        &self,
        mc: &Mutation<'gc>,
        class_obj: Gc<'gc, RefLock<Class<'gc>>>,
    ) -> Gc<'gc, RefLock<Object<'gc>>> {
        let count = self.ensure_field_layout(mc, class_obj);
        let nil_val = self.new_nil(mc);
        let fields = Fields::new(count, nil_val);
        gcl!(
            mc,
            Object {
                class: class_obj,
                fields,
                payload: ObjectPayload::Instance,
            }
        )
    }

    /// Ensure `class.field_slots` covers the full current hierarchy (own + mixins +
    /// parent) and return the field count. Append-only: a newly-seen ivar gets a
    /// fresh trailing slot, so existing slots stay stable across runtime mixins.
    fn ensure_field_layout(
        &self,
        mc: &Mutation<'gc>,
        class: Gc<'gc, RefLock<Class<'gc>>>,
    ) -> usize {
        let all = self.get_all_instance_vars(class);
        let mut c = class.borrow_mut(mc);
        for name in all {
            if !c.field_slots.contains_key(&name) {
                let slot = c.field_slots.len();
                c.field_slots.insert(name, slot);
            }
        }
        c.field_slots.len()
    }

    /// The absolute slot of instance variable `name` for instances of `class`
    /// (the layout is populated at instantiation), or `None` if it's not a declared
    /// ivar of the class.
    fn field_slot(&self, class: Gc<'gc, RefLock<Class<'gc>>>, name: &str) -> Option<usize> {
        class.borrow().field_slots.get(name).copied()
    }

    pub fn new_native_state<T: AnyCollect + 'static>(
        &self,
        mc: &Mutation<'gc>,
        class_obj: Gc<'gc, RefLock<Class<'gc>>>,
        state: T,
    ) -> Value<'gc> {
        self.new_native_state_boxed(mc, class_obj, Box::new(state))
    }

    /// The dyn-safe core of [`new_native_state`](Self::new_native_state): takes an
    /// already-boxed payload, so it can sit on the `ext_sdk::Host` trait (which can't
    /// carry the generic form). The generic wrapper lives on `ext_sdk::HostExt`.
    pub fn new_native_state_boxed(
        &self,
        mc: &Mutation<'gc>,
        class_obj: Gc<'gc, RefLock<Class<'gc>>>,
        state: Box<dyn AnyCollect>,
    ) -> Value<'gc> {
        let payload = ObjectPayload::NativeState(gcl!(mc, state));
        let obj = gcl!(
            mc,
            Object {
                class: class_obj,
                fields: Fields::default(),
                payload,
            }
        );
        Value::Object(obj)
    }

    /// Start flushing this buffered write stream at program exit.
    pub fn track_write_stream(&mut self, stream: Value<'gc>) {
        self.open_write_streams.push(stream);
    }

    /// Stop tracking `stream` — it was closed (and so already flushed), or consumed by a
    /// `stringStream` that took over its buffer.
    pub fn untrack_write_stream(&mut self, mc: &Mutation<'gc>, stream: Value<'gc>) {
        let Ok(id) = stream.with_native_state::<NativeStream, _, _>(|s| s.stream_id()) else {
            return;
        };
        let _ = mc;
        self.open_write_streams.retain(|v| {
            v.with_native_state::<NativeStream, _, _>(|s| s.stream_id())
                .map(|other| other != id)
                .unwrap_or(true)
        });
    }

    /// Take every still-buffered byte from the tracked write streams. The driver writes these
    /// out when the program ends. Returns `(id, bytes)` pairs in the order the streams were
    /// opened; a stream with nothing pending contributes nothing.
    ///
    /// A stream that is still *open* stays tracked: the REPL drives — and so flushes — once per
    /// line, and a stream opened on one line is written on the next. Emptying the registry here
    /// would leave that stream untracked and lose its bytes. Closed streams are dropped; they
    /// were flushed on the way out.
    pub fn take_pending_writes(&mut self, mc: &Mutation<'gc>) -> Vec<(StreamId, Vec<u8>)> {
        let mut pending = Vec::new();
        self.open_write_streams.retain(|v| {
            match v.with_native_state_mut::<NativeStream, _, _>(mc, |s| {
                (!s.is_stream_closed(), s.take_pending())
            }) {
                Ok((open, bytes)) => {
                    if let Some(b) = bytes {
                        pending.push(b);
                    }
                    open
                }
                Err(_) => false, // no longer a stream: stop tracking it
            }
        });
        pending
    }

    // Scalar value types are immediate `Value` variants — no GC allocation. `mc`
    // is kept in the signatures so the many call sites stay unchanged.
    pub fn new_nil(&self, _mc: &Mutation<'gc>) -> Value<'gc> {
        Value::Nil
    }

    pub fn new_bool(&self, _mc: &Mutation<'gc>, b: bool) -> Value<'gc> {
        Value::Bool(b)
    }

    pub fn new_int(&self, _mc: &Mutation<'gc>, i: i64) -> Value<'gc> {
        Value::Int(i)
    }

    pub fn new_double(&self, _mc: &Mutation<'gc>, f: f64) -> Value<'gc> {
        Value::Double(f)
    }

    pub fn new_string(&self, mc: &Mutation<'gc>, s: String) -> Value<'gc> {
        let class = self.builtin_cache.borrow().string_class;
        let class = class.unwrap_or_else(|| self.get_or_create_builtin_class(mc, "String"));
        Value::Object(gcl!(
            mc,
            Object {
                class,
                fields: Fields::default(),
                payload: ObjectPayload::String(gc!(mc, s)),
            }
        ))
    }

    /// A fresh string VALUE over an already-GC'd shared buffer — the
    /// literal-materialization fast path (see `string_literal_buffers`).
    pub fn new_string_shared(&self, mc: &Mutation<'gc>, buf: Gc<'gc, String>) -> Value<'gc> {
        let class = self.builtin_cache.borrow().string_class;
        let class = class.unwrap_or_else(|| self.get_or_create_builtin_class(mc, "String"));
        Value::Object(gcl!(
            mc,
            Object {
                class,
                fields: Fields::default(),
                payload: ObjectPayload::String(buf),
            }
        ))
    }

    /// The shared buffer for literal content `s`, minting it on first use.
    pub fn literal_string_buffer(&mut self, mc: &Mutation<'gc>, s: &str) -> Gc<'gc, String> {
        if let Some(g) = self.string_literal_buffers.get(s) {
            return *g;
        }
        let g = gc!(mc, s.to_string());
        self.string_literal_buffers.insert(s.to_string(), g);
        g
    }

    /// Build an immutable `Bytes` value from raw bytes (mirrors `new_string`). One
    /// copy at the native boundary; the inner `Vec<u8>` is a GC leaf.
    pub fn new_bytes(&self, mc: &Mutation<'gc>, bytes: Vec<u8>) -> Value<'gc> {
        let class = self.get_or_create_builtin_class(mc, "Bytes");
        Value::Object(gcl!(
            mc,
            Object {
                class,
                fields: Fields::default(),
                payload: ObjectPayload::Bytes(gc!(mc, bytes)),
            }
        ))
    }

    /// Return the interned `Symbol` value for `name`, creating it on first use.
    /// All occurrences of the same name share one value, so symbols compare by
    /// identity.
    pub fn new_symbol(&self, mc: &Mutation<'gc>, name: String) -> Value<'gc> {
        let existing = self.symbol_table.borrow().get(&name).copied();
        if let Some(sym) = existing {
            return sym;
        }
        let class = self.get_or_create_builtin_class(mc, "Symbol");
        let sym = Value::Object(gcl!(
            mc,
            Object {
                class,
                fields: Fields::default(),
                payload: ObjectPayload::Symbol(gc!(mc, name.clone())),
            }
        ));
        self.symbol_table.borrow_mut(mc).insert(name, sym);
        sym
    }

    #[allow(clippy::wrong_self_convention)]
    pub fn to_s(
        &mut self,
        mc: &Mutation<'gc>,
        value: Value<'gc>,
    ) -> Result<Value<'gc>, QuoinError> {
        match value {
            Value::Class(_) | Value::ClassMeta(_) => {
                let display = value.to_string();
                Ok(self.new_string(mc, display))
            }
            // Object + immediate value types dispatch their `s` method.
            _ => self.call_method(mc, value, "s", vec![]),
        }
    }

    /// Verify every element of a FRESH collection literal against `tag`, then
    /// stamp the tag (`TagCollection` — annotation-driven tagged literals,
    /// docs/GENERICS_ARCH.md §4.2). Safe to stamp in place: the literal has no
    /// aliases yet.
    pub(crate) fn tag_fresh_collection(
        &self,
        mc: &Mutation<'gc>,
        v: Value<'gc>,
        tag: elem_tag::ElemTag,
    ) -> Result<(), QuoinError> {
        use crate::runtime::map::NativeMapState;
        use crate::runtime::set::NativeSetState;
        if let Ok(vec) = v.with_native_state::<NativeListState, _, _>(|l| l.get_vec().to_vec()) {
            for (i, e) in vec.iter().enumerate() {
                elem_tag::check_insert(Some(tag), "List", e, Some(i as i64), |val, n| {
                    self.value_matches_type(*val, n)
                })?;
            }
            let _ = v.with_native_state_mut::<NativeListState, _, _>(mc, |l| {
                l.elem = Some(tag);
            });
            return Ok(());
        }
        if let Ok(vals) = v.with_native_state::<NativeMapState, _, _>(|m| {
            m.entries().iter().map(|(_, _, v)| *v).collect::<Vec<_>>()
        }) {
            for e in &vals {
                elem_tag::check_insert(Some(tag), "Map String", e, None, |val, n| {
                    self.value_matches_type(*val, n)
                })?;
            }
            let _ = v.with_native_state_mut::<NativeMapState, _, _>(mc, |m| {
                m.elem = Some(tag);
            });
            return Ok(());
        }
        if let Ok(vec) = v.with_native_state::<NativeSetState, _, _>(|s| s.values()) {
            for (i, e) in vec.iter().enumerate() {
                elem_tag::check_insert(Some(tag), "Set", e, Some(i as i64), |val, n| {
                    self.value_matches_type(*val, n)
                })?;
            }
            let _ = v.with_native_state_mut::<NativeSetState, _, _>(mc, |s| {
                s.elem = Some(tag);
            });
            return Ok(());
        }
        Err(QuoinError::Other(
            "TagCollection on a non-collection value".to_string(),
        ))
    }

    /// Checked write into a TAGGED list (the cold side of the ListPush arm).
    #[inline(never)]
    pub(crate) fn tagged_list_push(
        &self,
        mc: &Mutation<'gc>,
        receiver: Value<'gc>,
        value: Value<'gc>,
    ) -> Result<(), QuoinError> {
        let tag = receiver
            .with_native_state::<NativeListState, _, _>(|l| l.elem)
            .map_err(QuoinError::Other)?;
        elem_tag::check_insert(tag, "List", &value, None, |v, n| {
            self.value_matches_type(*v, n)
        })?;
        let _ = receiver
            .with_native_state_mut::<NativeListState, _, _>(mc, |l| l.get_vec_mut().push(value));
        Ok(())
    }

    /// Checked write into a TAGGED list (the cold side of the ListSet arm).
    /// The tag check precedes the bounds check — the VALUE is illegal
    /// regardless of index.
    #[inline(never)]
    pub(crate) fn tagged_list_set(
        &self,
        mc: &Mutation<'gc>,
        receiver: Value<'gc>,
        i: i64,
        value: Value<'gc>,
    ) -> Result<(), QuoinError> {
        let tag = receiver
            .with_native_state::<NativeListState, _, _>(|l| l.elem)
            .map_err(QuoinError::Other)?;
        elem_tag::check_insert(tag, "List", &value, Some(i), |v, n| {
            self.value_matches_type(*v, n)
        })?;
        receiver
            .with_native_state_mut::<NativeListState, _, _>(mc, |l| {
                devirt_ops::list_set(l.get_vec_mut(), i, value)
            })
            .map_err(QuoinError::Other)?
    }

    pub fn new_list(&self, mc: &Mutation<'gc>, list: Vec<Value<'gc>>) -> Value<'gc> {
        let class = self.builtin_cache.borrow().list_class;
        let class = class.unwrap_or_else(|| self.get_or_create_builtin_class(mc, "List"));
        let state = NativeListState::new(list);
        let boxed_state: Box<dyn AnyCollect> = Box::new(state);
        Value::Object(gcl!(
            mc,
            Object {
                class,
                fields: Fields::default(),
                payload: ObjectPayload::NativeState(gc!(mc, RefLock::new(boxed_state))),
            }
        ))
    }

    pub fn new_map(&self, mc: &Mutation<'gc>, pairs: Vec<(String, Value<'gc>)>) -> Value<'gc> {
        let class = self.builtin_cache.borrow().map_class;
        let class = class.unwrap_or_else(|| self.get_or_create_builtin_class(mc, "Map"));
        // Constructor for the string-shaped native callers (JSON/wire/CSV/
        // stats): ordered pairs straight into the any-key storage, duplicate
        // keys last-wins (what the IndexMap intermediary this replaced did —
        // it cost a second hash of every key, plus SipHash and a table build
        // per map, measured on the json bench).
        let mut state = NativeMapState::new_empty();
        for (k, v) in pairs {
            let k = self.new_string(mc, k);
            state
                .insert_scalar(k, v)
                .expect("String keys are native-exact");
        }
        let boxed_state: Box<dyn AnyCollect> = Box::new(state);
        Value::Object(gcl!(
            mc,
            Object {
                class,
                fields: Fields::default(),
                payload: ObjectPayload::NativeState(gc!(mc, RefLock::new(boxed_state))),
            }
        ))
    }

    pub fn new_set(&self, mc: &Mutation<'gc>, set: Vec<Value<'gc>>) -> Value<'gc> {
        let class = self.get_or_create_builtin_class(mc, "Set");
        // Sole caller passes an empty vec (the NewSet literal dedups via
        // set_add); accept scalar-hashable elements defensively.
        let mut state = NativeSetState::new_empty();
        for v in set {
            let h = crate::value::value_hash_scalar(&v)
                .expect("new_set elements must be scalar-hashable; use set_add for instances");
            state.append(h, v);
        }
        let boxed_state: Box<dyn AnyCollect> = Box::new(state);
        Value::Object(gcl!(
            mc,
            Object {
                class,
                fields: Fields::default(),
                payload: ObjectPayload::NativeState(gc!(mc, RefLock::new(boxed_state))),
            }
        ))
    }

    /// True if `set_val` already contains a value equal (by Quoin `==:`) to `value`.
    pub fn set_contains(
        &mut self,
        mc: &Mutation<'gc>,
        set_val: Value<'gc>,
        value: Value<'gc>,
    ) -> Result<bool, QuoinError> {
        let (_, found) = crate::runtime::set::set_find(self, mc, set_val, value)?;
        Ok(found.is_some())
    }

    /// Insert `value` into `set_val` unless an equal element is already present.
    /// Returns whether a new element was added.
    pub fn set_add(
        &mut self,
        mc: &Mutation<'gc>,
        set_val: Value<'gc>,
        value: Value<'gc>,
    ) -> Result<bool, QuoinError> {
        let (h, found) = crate::runtime::set::set_find(self, mc, set_val, value)?;
        if found.is_some() {
            Ok(false)
        } else {
            set_val
                .with_native_state_mut::<NativeSetState, _, _>(mc, |s| s.append(h, value))
                .map_err(QuoinError::Other)?;
            Ok(true)
        }
    }

    /// Remove the first element of `set_val` equal (by `==:`) to `value`.
    /// Returns whether an element was removed.
    pub fn set_remove(
        &mut self,
        mc: &Mutation<'gc>,
        set_val: Value<'gc>,
        value: Value<'gc>,
    ) -> Result<bool, QuoinError> {
        let (_, found) = crate::runtime::set::set_find(self, mc, set_val, value)?;
        match found {
            Some(idx) => {
                set_val
                    .with_native_state_mut::<NativeSetState, _, _>(mc, |s| s.remove_at(idx))
                    .map_err(QuoinError::Other)?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    pub fn new_regex(&self, mc: &Mutation<'gc>, regex: Regex) -> Value<'gc> {
        let class = self.builtin_cache.borrow().regex_class;
        let class = class.unwrap_or_else(|| self.get_or_create_builtin_class(mc, "Regex"));
        let boxed_state: Box<dyn AnyCollect> = Box::new(NativeRegexState::new(regex));
        Value::Object(gcl!(
            mc,
            Object {
                class,
                fields: Fields::default(),
                payload: ObjectPayload::NativeState(gc!(mc, RefLock::new(boxed_state))),
            }
        ))
    }

    pub fn new_block(&self, mc: &Mutation<'gc>, block: Block<'gc>) -> Value<'gc> {
        let class = self.builtin_cache.borrow().block_class;
        let class = class.unwrap_or_else(|| self.get_or_create_builtin_class(mc, "Block"));
        Value::Object(gcl!(
            mc,
            Object {
                class,
                fields: Fields::default(),
                payload: ObjectPayload::Block(gc!(mc, block)),
            }
        ))
    }

    pub fn new_method(
        &self,
        mc: &Mutation<'gc>,
        selector: String,
        block: Value<'gc>,
        is_extension: bool,
    ) -> Value<'gc> {
        let class = self.get_or_create_builtin_class(mc, "Method");
        let state = NativeMethodState::new(selector, block, is_extension);
        let boxed_state: Box<dyn AnyCollect> = Box::new(state);
        Value::Object(gcl!(
            mc,
            Object {
                class,
                fields: Fields::default(),
                payload: ObjectPayload::NativeState(gc!(mc, RefLock::new(boxed_state))),
            }
        ))
    }

    /// Wrap a native fn as a `Method` chain node, so native methods are chainable,
    /// scored, and override-able just like user methods.
    pub fn new_native_method(
        &self,
        mc: &Mutation<'gc>,
        selector: String,
        func: NativeFunc,
        param_types: Option<Vec<String>>,
        ret_type: Option<String>,
        doc: Option<String>,
    ) -> Value<'gc> {
        let class = self.get_or_create_builtin_class(mc, "Method");
        let state = NativeMethodState::new_native(selector, func, param_types, ret_type, doc);
        let boxed_state: Box<dyn AnyCollect> = Box::new(state);
        Value::Object(gcl!(
            mc,
            Object {
                class,
                fields: Fields::default(),
                payload: ObjectPayload::NativeState(gc!(mc, RefLock::new(boxed_state))),
            }
        ))
    }

    /// Wrap an extension-backed selector as a `Method` chain node (Phase 3): the method dispatches
    /// over the socket to `ext` (the owning `Extension` value, kept GC-rooted via the method table).
    pub fn new_ext_method(
        &self,
        mc: &Mutation<'gc>,
        selector: String,
        ext: Value<'gc>,
    ) -> Value<'gc> {
        let class = self.get_or_create_builtin_class(mc, "Method");
        let state = NativeMethodState::new_ext(selector, ext);
        let boxed_state: Box<dyn AnyCollect> = Box::new(state);
        Value::Object(gcl!(
            mc,
            Object {
                class,
                fields: Fields::default(),
                payload: ObjectPayload::NativeState(gc!(mc, RefLock::new(boxed_state))),
            }
        ))
    }

    /// Install an extension-provided class (Phase 3) as a host global: a real Quoin class whose
    /// selectors dispatch over the socket to `ext`. Class-side selectors become class methods,
    /// instance-side become instance methods; each is an `ExtDispatch` node carrying `ext`. A
    /// re-declared name is overwritten (last spawn wins).
    pub fn install_ext_class(
        &mut self,
        mc: &Mutation<'gc>,
        ext: Value<'gc>,
        name: &str,
        instance_selectors: &[String],
        class_selectors: &[String],
    ) {
        let mut instance_methods: FxHashMap<Symbol, Value<'gc>> = FxHashMap::default();
        for sel in instance_selectors {
            let node = self.new_ext_method(mc, sel.clone(), ext);
            instance_methods.insert(Symbol::intern(sel), node);
        }
        let mut class_methods: FxHashMap<Symbol, Value<'gc>> = FxHashMap::default();
        for sel in class_selectors {
            let node = self.new_ext_method(mc, sel.clone(), ext);
            class_methods.insert(Symbol::intern(sel), node);
        }
        let parent = self.get_or_create_builtin_class(mc, "Object");
        let ns_name = NamespacedName::parse(name);
        let class_obj = gcl!(
            mc,
            Class {
                name: ns_name.clone(),
                parent: Some(parent),
                instance_vars: Vec::new(),
                instance_methods,
                class_methods,
                mixin_classes: Vec::new(),
                field_slots: FxHashMap::default(),
                init_plan: None,
                is_eigenclass: false,
                is_sealed: false,
                is_abstract: false,
                native_new_refusal: None,
            }
        );
        self.globals
            .borrow_mut(mc)
            .insert(ns_name, Value::Class(class_obj));
        self.invalidate_method_cache();
        // An ext class can shadow/extend a name already baked into a compiled
        // entry's direct self-calls — the redefinition epoch is what Bails
        // those stale entries (the contract codegen's epoch doc promises for
        // extension installs, matching the DefineMethod arms).
        crate::codegen::bump_redef_epoch();
    }

    /// The memoized instantiation recipe for `class` (see [`InitPlan`]),
    /// rebuilt whenever the dispatch epoch has moved — every method-table,
    /// mixin, or extension mutation bumps it (including `mix:`, fixed
    /// alongside this cache), so a stale plan cannot survive a hierarchy
    /// change. Field layout is append-only (`field_slots`), so resolved
    /// slots never go stale within an epoch.
    fn instantiation_plan(
        &mut self,
        mc: &Mutation<'gc>,
        class: Gc<'gc, RefLock<Class<'gc>>>,
    ) -> Gc<'gc, InitPlan<'gc>> {
        if let Some((epoch, plan)) = class.borrow().init_plan
            && epoch == self.dispatch_epoch
        {
            return plan;
        }
        let vars = self.get_all_instance_vars(class);
        let ivar_slots: Vec<(String, usize)> = vars
            .into_iter()
            .filter_map(|v| self.field_slot(class, &v).map(|slot| (v, slot)))
            .collect();
        let mut classes = Vec::new();
        let mut visited = Vec::new();
        self.collect_classes_for_init(class, &mut classes, &mut visited);
        let mut inits = Vec::new();
        for clz in classes {
            let init_colon = clz
                .borrow()
                .instance_methods
                .get(&init_colon_symbol())
                .copied()
                .map(|m| (m, self.init_param_names(m).unwrap_or_default()));
            let init_plain = clz.borrow().instance_methods.get(&init_symbol()).copied();
            if init_colon.is_some() || init_plain.is_some() {
                inits.push(InitEntry {
                    init_colon,
                    init_plain,
                });
            }
        }
        let plan = gc!(mc, InitPlan { ivar_slots, inits });
        class.borrow_mut(mc).init_plan = Some((self.dispatch_epoch, plan));
        plan
    }

    /// NO borrow may be held while an initializer runs: `call_method_value` executes
    /// arbitrary Quoin that can cooperatively yield (an `init` that resumes a fiber or
    /// does I/O parks the whole task mid-call), and a Class/env borrow living on this
    /// suspended stack collides with any other task touching the same cell — e.g.
    /// `ensure_field_layout`'s `borrow_mut` when instantiating the same class
    /// ("RefCell already borrowed"). So the env rides in as a `Gc` and is borrowed
    /// transiently per lookup, and method lookups are hoisted OUT of `if let`
    /// scrutinees (a scrutinee temporary lives through the success branch — even in
    /// edition 2024, whose rescope only shortened the `else` path).
    fn finalize_instantiation(
        &mut self,
        mc: &Mutation<'gc>,
        obj: Gc<'gc, RefLock<Object<'gc>>>,
        env: Gc<'gc, RefLock<EnvFrame<'gc>>>,
    ) -> Result<(), QuoinError> {
        let class = obj.borrow().class;
        let plan = self.instantiation_plan(mc, class);
        for (name, slot) in &plan.ivar_slots {
            let val = env.borrow().lookup_str(name);
            if let Some(val) = val {
                obj.borrow_mut(mc).fields[*slot] = val;
            }
        }

        // Run each class's initializer base->derived (parents, then mixins,
        // then self). A class that defines `init:` receives the block fields
        // it names (matched by param name); otherwise its zero-arg `init`
        // runs. Running the whole chain means an ancestor or mixin
        // initializer is never skipped just because a more derived class
        // happens to define `init:`. The plan is rooted for the chain's
        // duration (a user init can park AND replace the cached plan).
        let receiver = Value::Object(obj);
        self.active_init_plans.push(plan);
        let result = self.run_init_chain_planned(mc, receiver, plan, Some(env));
        self.active_init_plans.pop();
        result
    }

    /// The init-chain body shared by [`Self::finalize_instantiation`]
    /// (`with_env` = the `new:{}` block env feeding `init:` params) and
    /// [`Self::run_all_inits`] (`None`: the plain-`new` path runs `init`
    /// ONLY, exactly as before the plan existed).
    // The caller has pushed `plan` onto `active_init_plans` for this whole
    // call; the method Values read from it are rooted by that contract
    // across the user init calls (which can park).
    #[allow(no_gc_across_yield)]
    fn run_init_chain_planned(
        &mut self,
        mc: &Mutation<'gc>,
        receiver: Value<'gc>,
        plan: Gc<'gc, InitPlan<'gc>>,
        with_env: Option<Gc<'gc, RefLock<EnvFrame<'gc>>>>,
    ) -> Result<(), QuoinError> {
        for idx in 0..plan.inits.len() {
            let entry = &plan.inits[idx];
            match (with_env, &entry.init_colon) {
                (Some(env), Some((method_val, param_names))) => {
                    let method_val = *method_val;
                    let mut init_args = Vec::with_capacity(param_names.len());
                    for param in param_names {
                        let val = env
                            .borrow()
                            .lookup_str(param)
                            .unwrap_or_else(|| self.new_nil(mc));
                        init_args.push(val);
                    }
                    self.call_method_value(mc, receiver, method_val, "init:", init_args)?;
                }
                _ => {
                    if let Some(method_val) = entry.init_plain {
                        self.call_method_value(mc, receiver, method_val, "init", Vec::new())?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Parameter names of a method's underlying block, used so `init:` can be fed
    /// the `new:{}` block fields it declares by name. Handles both plain block
    /// methods and native-wrapped method state.
    fn init_param_names(&self, method_val: Value<'gc>) -> Option<Vec<String>> {
        let Value::Object(io) = method_val else {
            return None;
        };
        let io_ref = io.borrow();
        match &io_ref.payload {
            ObjectPayload::Block(b) => Some(
                b.template
                    .param_syms
                    .iter()
                    .map(|s| s.as_str().to_string())
                    .collect(),
            ),
            ObjectPayload::NativeState(state_cell) => {
                let state_ref = state_cell.borrow();
                let any_ref = (**state_ref).as_any();
                let method_state = any_ref.downcast_ref::<NativeMethodState>()?;
                if let Some(Value::Object(block_obj)) = method_state.get_block()
                    && let ObjectPayload::Block(b) = &block_obj.borrow().payload
                {
                    Some(
                        b.template
                            .param_syms
                            .iter()
                            .map(|s| s.as_str().to_string())
                            .collect(),
                    )
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    pub fn get_or_create_builtin_class(
        &self,
        mc: &Mutation<'gc>,
        name: &str,
    ) -> Gc<'gc, RefLock<Class<'gc>>> {
        let ns_name = NamespacedName::parse(name);
        let existing = self.globals.borrow().get(&ns_name).copied();
        if let Some(Value::Class(c)) = existing {
            c
        } else {
            let parent = if name == "Object" {
                None
            } else {
                Some(self.get_or_create_builtin_class(mc, "Object"))
            };
            let class_obj = gcl!(
                mc,
                Class {
                    name: ns_name.clone(),
                    parent,
                    instance_vars: Vec::new(),
                    instance_methods: FxHashMap::default(),
                    class_methods: FxHashMap::default(),
                    mixin_classes: Vec::new(),
                    field_slots: FxHashMap::default(),
                    init_plan: None,
                    is_eigenclass: false,
                    is_sealed: false,
                    is_abstract: false,
                    native_new_refusal: None,
                }
            );
            self.globals
                .borrow_mut(mc)
                .insert(ns_name, Value::Class(class_obj));

            let mut cache = self.builtin_cache.borrow_mut(mc);
            match name {
                "Nil" => cache.nil_class = Some(class_obj),
                "Boolean" => cache.boolean_class = Some(class_obj),
                "Integer" => cache.integer_class = Some(class_obj),
                "Double" => cache.double_class = Some(class_obj),
                "String" => cache.string_class = Some(class_obj),
                "List" => cache.list_class = Some(class_obj),
                "Map" => cache.map_class = Some(class_obj),
                "Regex" => cache.regex_class = Some(class_obj),
                "Block" => cache.block_class = Some(class_obj),
                _ => {}
            }
            class_obj
        }
    }

    pub fn get_builtin_class(&self, name: &str) -> Gc<'gc, RefLock<Class<'gc>>> {
        let ns_name = NamespacedName::parse(name);
        let existing = self.globals.borrow().get(&ns_name).copied();
        if let Some(Value::Class(c)) = existing {
            c
        } else {
            panic!("Builtin class {} not found in globals!", name);
        }
    }

    pub fn register_native_class<T: NativeClass>(&mut self, mc: &Mutation<'gc>, native_class: T) {
        if let Some(doc) = native_class.class_doc() {
            self.class_meta
                .entry(NamespacedName::parse(native_class.name()))
                .or_default()
                .doc = Some(doc.to_string());
        }
        let parent_class = if let Some(parent_name) = native_class.parent_name() {
            Some(self.get_or_create_builtin_class(mc, parent_name))
        } else {
            None
        };

        // Several defs may share a selector (typed multimethod variants); chain
        // them in declaration order so the scorer routes by argument type and ties
        // resolve to the first-declared.
        let mut inst_methods: FxHashMap<Symbol, Value<'gc>> = FxHashMap::default();
        for def in native_class.instance_methods() {
            let sym = Symbol::intern(&def.selector);
            let node = self.new_native_method(
                mc,
                def.selector.clone(),
                def.func,
                def.param_types,
                def.ret_type,
                def.doc,
            );
            if let Some(head) = inst_methods.get(&sym).copied() {
                let _ = Self::append_method_to_chain(mc, head, node);
            } else {
                inst_methods.insert(sym, node);
            }
        }

        let mut cls_methods: FxHashMap<Symbol, Value<'gc>> = FxHashMap::default();
        for def in native_class.class_methods() {
            let sym = Symbol::intern(&def.selector);
            let node = self.new_native_method(
                mc,
                def.selector.clone(),
                def.func,
                def.param_types,
                def.ret_type,
                def.doc,
            );
            if let Some(head) = cls_methods.get(&sym).copied() {
                let _ = Self::append_method_to_chain(mc, head, node);
            } else {
                cls_methods.insert(sym, node);
            }
        }

        let (is_abstract, native_new_refusal) = match native_class.new_policy() {
            NativeNewPolicy::Abstract => (true, None),
            NativeNewPolicy::Refuse(hint) => (false, Some(hint.unwrap_or(NATIVE_NEW_GENERIC_HINT))),
        };

        let name = native_class.name();
        let ns_name = NamespacedName::parse(name);
        let existing = self.globals.borrow().get(&ns_name).copied();
        if let Some(Value::Class(existing_class)) = existing {
            let mut borrowed = existing_class.borrow_mut(mc);
            borrowed.parent = parent_class;
            borrowed.instance_methods = inst_methods;
            borrowed.class_methods = cls_methods;
            borrowed.instance_vars = Vec::new();
            borrowed.is_abstract = is_abstract;
            borrowed.native_new_refusal = native_new_refusal;
        } else {
            let class_obj = gcl!(
                mc,
                Class {
                    name: ns_name.clone(),
                    parent: parent_class,
                    instance_vars: Vec::new(),
                    instance_methods: inst_methods,
                    class_methods: cls_methods,
                    mixin_classes: Vec::new(),
                    field_slots: FxHashMap::default(),
                    init_plan: None,
                    is_eigenclass: false,
                    is_sealed: false,
                    is_abstract,
                    native_new_refusal,
                }
            );

            self.globals
                .borrow_mut(mc)
                .insert(ns_name, Value::Class(class_obj));

            let mut cache = self.builtin_cache.borrow_mut(mc);
            match name {
                "Nil" => cache.nil_class = Some(class_obj),
                "Boolean" => cache.boolean_class = Some(class_obj),
                "Integer" => cache.integer_class = Some(class_obj),
                "Double" => cache.double_class = Some(class_obj),
                "String" => cache.string_class = Some(class_obj),
                "List" => cache.list_class = Some(class_obj),
                "Map" => cache.map_class = Some(class_obj),
                "Regex" => cache.regex_class = Some(class_obj),
                "Block" => cache.block_class = Some(class_obj),
                _ => {}
            }
        }
        // A class's method tables just changed — drop any memoized resolutions.
        self.invalidate_method_cache();
    }
    pub fn start_method_call(
        &mut self,
        mc: &Mutation<'gc>,
        receiver: Value<'gc>,
        selector: &str,
        args: Vec<Value<'gc>>,
    ) -> Result<usize, QuoinError> {
        let sel = Symbol::intern(selector);
        let method = self.lookup_method(mc, receiver, sel, &args)?;
        if let Some(method) = method {
            let initial_frame_count = self.frames.len();
            method.call(self, mc, Some(receiver), args, Some(sel), None)?;
            Ok(initial_frame_count)
        } else {
            Err(QuoinError::Other(format!(
                "Method {} not found on receiver",
                selector
            )))
        }
    }

    pub fn call_method(
        &mut self,
        mc: &Mutation<'gc>,
        receiver: Value<'gc>,
        selector: &str,
        args: Vec<Value<'gc>>,
    ) -> Result<Value<'gc>, QuoinError> {
        // Bound native → Quoin re-entry so a self-referential hook (a `==:` that re-adds
        // to the set it's a key of, a comparator that re-sorts, …) fails catchably rather
        // than overflowing the machine stack. The `?` returns before incrementing on the
        // over-limit case; otherwise the guard decrements on every exit path.
        self.enter_native_reentry()?;
        let result = self.call_method_inner(mc, receiver, selector, args);
        self.native_reentry_depth = self.native_reentry_depth.saturating_sub(1);
        result
    }

    /// The catchable ceiling on native → Quoin re-entry depth (see `native_reentry_depth`).
    /// Well above any legitimate nesting of custom hooks, low enough to fault before the
    /// coroutine stack overflows (each re-entry frame drives a nested `step` loop).
    const MAX_NATIVE_REENTRY: usize = 12;

    /// Headroom `execute_block` insists on before re-entering the VM: refuse once fewer than
    /// this many bytes of the 16 MiB coroutine stack remain.
    ///
    /// A *depth* cap is the wrong instrument here (and is why `execute_block` was left
    /// unguarded): lazy generator pipelines legitimately compose blocks deeper than any
    /// machine-stack-safe fixed count, so a counter cannot tell them from a block that
    /// re-enters itself. Measuring the stack itself separates the two — deep-but-finite
    /// pipelines keep their real ceiling, minus this margin.
    ///
    /// 2 MiB is sized to cover the deepest single frame we can add after the check passes:
    /// `dispatch_one` + a compiled outcall + a native method, several times over.
    const STACK_MARGIN: usize = 2 * 1024 * 1024;

    /// Claim one level of native re-entry, or return a catchable error at the ceiling.
    fn enter_native_reentry(&mut self) -> Result<(), QuoinError> {
        if self.native_reentry_depth >= Self::MAX_NATIVE_REENTRY {
            return Err(QuoinError::StackExhausted(format!(
                "native call recursion too deep (> {}): a custom ==:/hash/comparator/render \
                 hook is re-entering a native operation without bound",
                Self::MAX_NATIVE_REENTRY
            )));
        }
        self.native_reentry_depth += 1;
        Ok(())
    }

    /// Refuse to re-enter the VM when this coroutine's stack is nearly spent.
    ///
    /// Each `execute_block` level stacks *real Rust frames* (the `valueWithSelfOrArg:`
    /// combinator seam, the `catch:` family), so an `each:` body that re-iterates its own
    /// receiver — or a `catch:` whose protected block re-enters itself — walks off the end of
    /// the 16 MiB coroutine stack and aborts the process with SIGBUS, uncatchably. The check
    /// is a load, a subtract and a compare against the address of a stack local.
    ///
    /// `stack_limit == 0` disables it: the benchmark harness steps the VM on the OS thread
    /// stack, where we have no extent to measure and no re-entry to bound.
    #[inline]
    fn ensure_stack_headroom(&self) -> Result<(), QuoinError> {
        if self.stack_limit == 0 {
            return Ok(());
        }
        let probe = 0u8;
        let sp = &probe as *const u8 as usize;
        if sp.saturating_sub(self.stack_limit) >= Self::STACK_MARGIN {
            return Ok(());
        }
        Err(QuoinError::StackExhausted(
            "block re-entry exhausted the task stack: a block is re-entering itself without \
             bound (an each:/collect: body that re-iterates its own receiver, or a catch: \
             whose protected block re-enters it)"
                .to_string(),
        ))
    }

    fn call_method_inner(
        &mut self,
        mc: &Mutation<'gc>,
        receiver: Value<'gc>,
        selector: &str,
        args: Vec<Value<'gc>>,
    ) -> Result<Value<'gc>, QuoinError> {
        let sel = Symbol::intern(selector);
        let method = self.lookup_method(mc, receiver, sel, &args)?;
        if let Some(method) = method {
            let initial_frame_count = self.frames.len();
            method.call(self, mc, Some(receiver), args, Some(sel), None)?;

            // let the VM catch up (batched — B0)
            self.run_nested(mc, initial_frame_count, "method call")?;

            Ok(self.pop()?)
        } else {
            Ok(self.new_nil(mc))
        }
    }

    pub fn call_method_value(
        &mut self,
        mc: &Mutation<'gc>,
        receiver: Value<'gc>,
        method_val: Value<'gc>,
        selector: &str,
        args: Vec<Value<'gc>>,
    ) -> Result<Value<'gc>, QuoinError> {
        self.enter_native_reentry()?;
        let result = self.call_method_value_inner(mc, receiver, method_val, selector, args);
        self.native_reentry_depth = self.native_reentry_depth.saturating_sub(1);
        result
    }

    fn call_method_value_inner(
        &mut self,
        mc: &Mutation<'gc>,
        receiver: Value<'gc>,
        method_val: Value<'gc>,
        selector: &str,
        args: Vec<Value<'gc>>,
    ) -> Result<Value<'gc>, QuoinError> {
        let method: Option<Callable<'gc>> = match method_val {
            Value::Object(obj) => match &obj.borrow().payload {
                ObjectPayload::Block(block) => Some(Callable::Block(*block)),
                ObjectPayload::NativeState(state_cell) => {
                    let state_ref = state_cell.borrow();
                    let any_ref = (**state_ref).as_any();
                    if let Some(method_state) = any_ref.downcast_ref::<NativeMethodState>() {
                        if let Some(ext) = method_state.ext_dispatch() {
                            Some(Callable::ExtMethod {
                                ext,
                                selector: Symbol::intern(selector),
                            })
                        } else if let Some(func) = method_state.native_func() {
                            Some(Callable::Native(func))
                        } else if let Some(Value::Object(block_obj)) = method_state.get_block()
                            && let ObjectPayload::Block(block) = &block_obj.borrow().payload
                        {
                            Some(Callable::Block(*block))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
                _ => None,
            },
            _ => None,
        };

        if let Some(method) = method {
            let initial_frame_count = self.frames.len();
            method.call(
                self,
                mc,
                Some(receiver),
                args,
                Some(Symbol::intern(selector)),
                None,
            )?;

            // let the VM catch up (batched — B0)
            self.run_nested(mc, initial_frame_count, "method call")?;

            Ok(self.pop()?)
        } else {
            Ok(self.new_nil(mc))
        }
    }

    fn collect_classes_for_init(
        &self,
        class_ref: Gc<'gc, RefLock<Class<'gc>>>,
        classes: &mut Vec<Gc<'gc, RefLock<Class<'gc>>>>,
        visited: &mut Vec<Gc<'gc, RefLock<Class<'gc>>>>,
    ) {
        if visited.iter().any(|c| Gc::ptr_eq(*c, class_ref)) {
            return;
        }
        visited.push(class_ref);

        let class_borrow = class_ref.borrow();
        if let Some(parent) = class_borrow.parent {
            self.collect_classes_for_init(parent, classes, visited);
        }
        for mixin in &class_borrow.mixin_classes {
            self.collect_classes_for_init(*mixin, classes, visited);
        }

        if !classes.iter().any(|c| Gc::ptr_eq(*c, class_ref)) {
            classes.push(class_ref);
        }
    }

    /// Run a frame's deferred calls in order. Each is a plain method send; the
    /// first one that errors aborts and returns the error.
    fn run_defers(
        &mut self,
        mc: &Mutation<'gc>,
        defers: &[DeferredCall<'gc>],
    ) -> Result<(), QuoinError> {
        for d in defers {
            self.call_method(mc, d.receiver, &d.selector, d.args.clone())?;
        }
        Ok(())
    }

    pub fn run_all_inits(
        &mut self,
        mc: &Mutation<'gc>,
        obj: Gc<'gc, RefLock<Object<'gc>>>,
    ) -> Result<(), QuoinError> {
        let class = obj.borrow().class;
        let plan = self.instantiation_plan(mc, class);
        let receiver = Value::Object(obj);
        self.active_init_plans.push(plan);
        let result = self.run_init_chain_planned(mc, receiver, plan, None);
        self.active_init_plans.pop();
        result
    }

    /// Drive nested execution (a native-initiated block or method call) until the frame
    /// stack returns to `initial_frame_count` — the BATCHED form (B0,
    /// docs/BLOCK_AOT_ARCH.md §3). One flat loop with the current frame's bytecode `Rc`
    /// hoisted exactly like `run_dispatch` (re-cloned only when the frame stack changes),
    /// yielding to the driver every `step_batch()` instructions instead of after every
    /// one. This gives nested block bodies — every `each:`-family combinator element —
    /// the same observable scheduling granularity as top-level code; before B0 they paid
    /// a full coroutine suspend→driver→resume round-trip plus a bytecode-`Rc` clone per
    /// instruction. Under the stress modes `step_batch()` is 1, so their per-instruction
    /// coverage is unchanged. Errors are returned raw (un-annotated), exactly as the
    /// per-step loops returned them; `context` names the caller in the uncaught-throw
    /// message, byte-identical to the old per-site strings.
    /// An in-flight `^^` MUST keep unwinding past this loop — either its
    /// target frame is strictly below the loop's baseline, or its home is a
    /// live COMPILED frame (`aot.nlr_target` set): a compiled frame owns no
    /// interpreter frame of its own to pop, so its delivery stops the unwind
    /// EXACTLY AT nested baselines, where "all callee frames gone" must read
    /// as delivery, not completion — only the owning `codegen::invoke` may
    /// consume it (the S5 absorb-at-baseline abort). Every loop that absorbs
    /// `NonLocalReturn` decides through this ONE predicate.
    #[inline(always)]
    pub(crate) fn nlr_must_propagate(&self, baseline: usize) -> bool {
        self.frames.len() < baseline || self.aot.nlr_target.is_some()
    }

    fn run_nested(
        &mut self,
        mc: &Mutation<'gc>,
        initial_frame_count: usize,
        context: &str,
    ) -> Result<(), QuoinError> {
        let budget = crate::tuning::step_batch();
        let mut steps: u32 = 0;
        let mut cached_len = usize::MAX;
        let mut bytecode: Option<SharedBytecode> = None;
        while self.frames.len() > initial_frame_count {
            // The cancellation check `step_internal` performed per step — including
            // immediately after a resume from the suspend below.
            if self.sched.cancel_current {
                return Err(self.take_cancellation());
            }
            let flen = self.frames.len();
            if flen != cached_len {
                cached_len = flen;
                bytecode = Some(self.frames[flen - 1].block.template.bytecode.clone());
            }
            match self.dispatch_one(mc, bytecode.as_ref().unwrap()) {
                Ok(VmStatus::Running) => {}
                // A `^`/`^^` unwound frames: below the baseline it belongs to an
                // enclosing loop; at/above it, the loop head re-evaluates. Counted
                // as a step, like `run_dispatch`.
                Err(QuoinError::NonLocalReturn) => {
                    if self.nlr_must_propagate(initial_frame_count) {
                        return Err(QuoinError::NonLocalReturn);
                    }
                }
                Ok(VmStatus::Finished(_)) => break,
                Ok(VmStatus::Yeeted(val)) => {
                    return Err(QuoinError::Other(format!(
                        "Uncaught exception during {}: {}",
                        context, val
                    )));
                }
                Err(e) => return Err(e),
            }
            steps += 1;
            if steps >= budget {
                steps = 0;
                if let Some(yielder) = unsafe { self.get_yielder() } {
                    yielder.suspend(YieldReason::CooperativeYield);
                }
            }
        }
        Ok(())
    }

    pub fn execute_block(
        &mut self,
        mc: &Mutation<'gc>,
        block: Gc<'gc, Block<'gc>>,
        args: Vec<Value<'gc>>,
        self_val: Option<Value<'gc>>,
    ) -> Result<Value<'gc>, QuoinError> {
        // NOTE: deliberately *not* guarded by `enter_native_reentry`. Lazy generator
        // pipelines legitimately compose blocks many levels deep on the native stack
        // (each stage's `execute_block` nests inside the next), so a low machine-stack
        // cap here would break real programs. The native-recursion guard lives on the
        // method-dispatch paths (`call_method`/`call_method_value`), where the
        // pathological self-referential hooks (a `==:` that re-adds to its own set)
        // actually recurse. What bounds *this* path is the remaining stack itself, which
        // costs those pipelines nothing while still refusing unbounded self-re-entry.
        self.ensure_stack_headroom()?;
        let initial_frame_count = self.frames.len();
        if let Some(receiver) = self_val {
            self.start_block_as_method(mc, block, receiver, args, None, false);
        } else {
            self.start_block(mc, block, args, None, None);
        }

        self.run_nested(mc, initial_frame_count, "block execution")?;

        Ok(self.pop()?)
    }

    /// Start a REPL line's top-level `block` in the persistent `repl_env`, returning the
    /// `(frame, stack)` depths to restore once the line finishes. The frame's env *is* the
    /// reused `repl_env` (not a fresh child), so top-level `x = 5` binds there and persists
    /// across lines. Transient scheduler state is reset first so a line that errored mid-fiber
    /// can't corrupt this one. The caller installs this as scheduler task #0 and drives it
    /// (via the shared `drive_main_task`), so the line gets async I/O, sleep, tasks, and
    /// fibers — which the old synchronous path could not. `repl_env` must be `Some`.
    pub fn begin_repl_line(&mut self, block: Gc<'gc, Block<'gc>>) -> (usize, usize) {
        let env = self
            .repl_env
            .expect("begin_repl_line called without a repl_env");
        let base_frames = self.frames.len();
        let base_stack = self.stack.len();
        self.reset_scheduler();

        let frame_id = self.next_frame_id;
        self.next_frame_id += 1;
        self.frames.push(Frame {
            id: frame_id,
            is_nested_block: false,
            enclosing_method_id: Some(frame_id),
            block,
            ic: block.inline_cache,
            ip: 0,
            env,
            instantiating_obj: None,
            receiver: None,
            selector: None,
            args: Vec::new(),
            stack_base: base_stack,
            spec_tid: 0,
            return_receiver: false,
            defers: Vec::new(),
            unregister_on_defer_failure: None,
        });
        (base_frames, base_stack)
    }

    /// Finish a REPL line driven by the scheduler: take its result off the stack (or `nil` on
    /// error / an empty stack), then restore the `(frame, stack)` baseline and clear any
    /// pending exception so the next line starts clean. `succeeded` reflects whether the drive
    /// finished without a runtime error; the error itself is already source-annotated by `step`
    /// and surfaced by the caller. The returned value is meaningful only when `succeeded`.
    pub fn end_repl_line(
        &mut self,
        mc: &Mutation<'gc>,
        base_frames: usize,
        base_stack: usize,
        succeeded: bool,
    ) -> Value<'gc> {
        let result = if succeeded {
            self.pop().unwrap_or_else(|_| self.new_nil(mc))
        } else {
            self.new_nil(mc)
        };
        self.frames.truncate(base_frames);
        self.stack.truncate(base_stack);
        self.exceptions.active = None;
        result
    }

    pub fn execute_validation_block(
        &mut self,
        mc: &Mutation<'gc>,
        block: Gc<'gc, Block<'gc>>,
        receiver: Value<'gc>,
        outer_param_syms: &[Symbol],
        args: &[Value<'gc>],
    ) -> Result<Value<'gc>, QuoinError> {
        let initial_frame_count = self.frames.len();

        let frame_id = self.next_frame_id;
        self.next_frame_id += 1;

        let mut env_frame = EnvFrame::new(block.parent_env);

        // A guard is a predicate over the method's arguments: every argument is bound
        // by its (method) parameter name, so the guard references them directly
        // (`|x:Integer { x > 5 }|`) without re-declaring them. `self` is the method's
        // receiver (the subject of the call), so a guard can also use the rest of the
        // class's functionality — other methods, instance variables, etc.
        env_frame.bind(self_symbol(), receiver);

        for (sym, val) in outer_param_syms.iter().zip(args.iter().copied()) {
            env_frame.bind(*sym, val);
        }

        let env_ref = gcl!(mc, env_frame);

        self.frames.push(Frame {
            id: frame_id,
            is_nested_block: block.template.is_nested_block,
            enclosing_method_id: Some(frame_id),
            block,
            ic: block.inline_cache,
            ip: 0,
            env: env_ref,
            instantiating_obj: None,
            receiver: Some(receiver),
            selector: None,
            args: args.to_vec(),
            stack_base: self.stack.len(),
            spec_tid: 0,
            return_receiver: false,
            defers: Vec::new(),
            unregister_on_defer_failure: None,
        });

        self.run_nested(mc, initial_frame_count, "validation block execution")?;

        Ok(self.pop()?)
    }

    pub fn is_subclass_of_clz(
        &self,
        sub: Gc<'gc, RefLock<Class<'gc>>>,
        sup: Gc<'gc, RefLock<Class<'gc>>>,
    ) -> bool {
        let mut curr = Some(sub);
        while let Some(clz) = curr {
            if Gc::ptr_eq(clz, sup) {
                return true;
            }
            for mixin in &clz.borrow().mixin_classes {
                if Gc::ptr_eq(*mixin, sup) {
                    return true;
                }
            }
            curr = clz.borrow().parent;
        }
        false
    }

    pub fn is_instance_of(&self, val: Value<'gc>, class_obj: Gc<'gc, RefLock<Class<'gc>>>) -> bool {
        if let Some(val_class) = self.get_class_for_lookup(val) {
            self.is_subclass_of_clz(val_class, class_obj)
        } else {
            false
        }
    }

    pub fn append_method_to_chain(
        mc: &Mutation<'gc>,
        chain_start: Value<'gc>,
        new_method: Value<'gc>,
    ) -> Result<(), QuoinError> {
        let mut curr = chain_start;
        loop {
            if let Value::Object(obj) = curr {
                let payload = &obj.borrow().payload;
                if let ObjectPayload::NativeState(state_cell) = payload {
                    let mut state_ref = state_cell.borrow_mut(mc);
                    let any_mut = state_ref.as_any_mut();
                    if let Some(method_state) = any_mut.downcast_mut::<NativeMethodState>() {
                        if let Some(next_val) = method_state.next {
                            let next_val_gc: Value<'gc> = unsafe { transmute(next_val) };
                            drop(state_ref);
                            curr = next_val_gc;
                            continue;
                        } else {
                            let new_method_static: Value<'static> =
                                unsafe { transmute(new_method) };
                            method_state.next = Some(new_method_static);
                            return Ok(());
                        }
                    }
                }
            }
            return Err(QuoinError::Other(
                "Invalid method object in chain".to_string(),
            ));
        }
    }

    /// Add `new_method` to a selector's method chain. A plain *unguarded* variant
    /// (no `decl_block`) whose parameter types match an existing unguarded variant
    /// *replaces* that variant's block in place — a true redefinition, so a later
    /// `-->` (or a repeated `->`) overrides instead of silently shadowing. Guarded
    /// and type-differentiated variants are appended, preserving definition order
    /// for multimethod dispatch.
    fn replace_or_append_method_in_chain(
        &self,
        mc: &Mutation<'gc>,
        chain_start: Value<'gc>,
        new_method: Value<'gc>,
    ) -> Result<(), QuoinError> {
        let new_block = self.get_block_from_method(new_method);
        if let Some(nb) = new_block
            && nb.decl_block.is_none()
            && let Some(new_block_val) =
                new_method.with_native_state::<NativeMethodState, _, _>(|m| m.get_block())?
        {
            let new_param_types = nb.template.param_types.clone();
            // Element-tag requirements are part of a variant's identity:
            // `|l: List(Integer)|` and `|l: List(String)|` share the erased
            // base signature ["List"] but are distinct multimethod variants
            // (GENERICS_ARCH.md §5), not a redefinition.
            let new_elem_tags = nb.template.param_elem_tags.clone();
            let mut curr = Some(chain_start);
            while let Some(node) = curr {
                let is_match = self
                    .get_block_from_method(node)
                    .map(|eb| {
                        eb.decl_block.is_none()
                            && eb.template.param_types == new_param_types
                            && eb.template.param_elem_tags == new_elem_tags
                    })
                    .unwrap_or(false);
                if is_match {
                    if let Value::Object(obj) = node {
                        let obj_ref = obj.borrow();
                        if let ObjectPayload::NativeState(state_cell) = &obj_ref.payload {
                            let mut state_ref = state_cell.borrow_mut(mc);
                            if let Some(ms) =
                                state_ref.as_any_mut().downcast_mut::<NativeMethodState>()
                            {
                                ms.body =
                                    MethodBody::UserBlock(unsafe { transmute(new_block_val) });
                            }
                        }
                    }
                    return Ok(());
                }
                curr = self.get_next_method_in_chain(node);
            }
        }
        Self::append_method_to_chain(mc, chain_start, new_method)
    }

    pub fn lookup_in_class_hierarchy(
        &self,
        class_ref: Gc<'gc, RefLock<Class<'gc>>>,
        selector: &str,
        class_side: bool,
    ) -> Option<Value<'gc>> {
        // Intern once at the boundary; the recursive walk probes by Symbol.
        let selector = Symbol::intern(selector);
        let mut visited = Vec::new();
        self.lookup_in_class_hierarchy_rec(class_ref, selector, class_side, &mut visited)
    }

    fn lookup_in_class_hierarchy_rec(
        &self,
        class_ref: Gc<'gc, RefLock<Class<'gc>>>,
        selector: Symbol,
        class_side: bool,
        visited: &mut Vec<Gc<'gc, RefLock<Class<'gc>>>>,
    ) -> Option<Value<'gc>> {
        if visited.iter().any(|c| Gc::ptr_eq(*c, class_ref)) {
            return None;
        }
        visited.push(class_ref);

        let class_borrow = class_ref.borrow();
        let methods = if class_side {
            &class_borrow.class_methods
        } else {
            &class_borrow.instance_methods
        };
        if let Some(method) = methods.get(&selector).copied() {
            return Some(method);
        }
        for mixin in &class_borrow.mixin_classes {
            if let Some(method) =
                self.lookup_in_class_hierarchy_rec(*mixin, selector, class_side, visited)
            {
                return Some(method);
            }
        }
        if let Some(parent) = class_borrow.parent {
            if let Some(method) =
                self.lookup_in_class_hierarchy_rec(parent, selector, class_side, visited)
            {
                return Some(method);
            }
        }
        None
    }

    pub fn get_all_instance_vars(&self, class_ref: Gc<'gc, RefLock<Class<'gc>>>) -> Vec<String> {
        let mut vars = Vec::new();
        let mut visited = Vec::new();
        self.collect_instance_vars(class_ref, &mut vars, &mut visited);
        vars
    }

    fn collect_instance_vars(
        &self,
        class_ref: Gc<'gc, RefLock<Class<'gc>>>,
        vars: &mut Vec<String>,
        visited: &mut Vec<Gc<'gc, RefLock<Class<'gc>>>>,
    ) {
        if visited.iter().any(|c| Gc::ptr_eq(*c, class_ref)) {
            return;
        }
        visited.push(class_ref);

        let class_borrow = class_ref.borrow();
        for var in &class_borrow.instance_vars {
            if !vars.contains(var) {
                vars.push(var.clone());
            }
        }
        for mixin in &class_borrow.mixin_classes {
            self.collect_instance_vars(*mixin, vars, visited);
        }
        if let Some(parent) = class_borrow.parent {
            self.collect_instance_vars(parent, vars, visited);
        }
    }

    pub fn push(&mut self, val: Value<'gc>) {
        self.stack.push(val);
    }

    pub fn pop(&mut self) -> Result<Value<'gc>, String> {
        self.stack
            .pop()
            .ok_or_else(|| "Stack underflow".to_string())
    }

    pub fn peek(&self) -> Result<Value<'gc>, String> {
        self.stack
            .last()
            .copied()
            .ok_or_else(|| "Stack is empty".to_string())
    }

    pub fn start_block(
        &mut self,
        mc: &Mutation<'gc>,
        block: Gc<'gc, Block<'gc>>,
        args: Vec<Value<'gc>>,
        receiver: Option<Value<'gc>>,
        selector: Option<Symbol>,
    ) {
        let frame_id = self.next_frame_id;
        self.next_frame_id += 1;

        let mut env_frame = EnvFrame::new(block.parent_env);
        // Bind parameters
        for (sym, val) in block.template.param_syms.iter().zip(args.iter().copied()) {
            env_frame.bind(*sym, val);
        }
        let env_ref = gcl!(mc, env_frame);

        let is_nested_block = block.template.is_nested_block;
        let enclosing_method_id = if is_nested_block {
            block.enclosing_method_id
        } else {
            Some(frame_id)
        };

        self.frames.push(Frame {
            id: frame_id,
            is_nested_block,
            enclosing_method_id,
            block,
            ic: block.inline_cache,
            ip: 0,
            env: env_ref,
            instantiating_obj: None,
            receiver,
            selector,
            args,
            stack_base: self.stack.len(),
            spec_tid: 0,
            return_receiver: false,
            defers: Vec::new(),
            unregister_on_defer_failure: None,
        });
    }

    pub fn start_block_as_method(
        &mut self,
        mc: &Mutation<'gc>,
        block: Gc<'gc, Block<'gc>>,
        receiver: Value<'gc>,
        args: Vec<Value<'gc>>,
        selector: Option<Symbol>,
        is_method_call: bool,
    ) {
        let frame_id = self.next_frame_id;
        self.next_frame_id += 1;

        let spec_tid = if is_method_call
            && self.aot_spec_obs_left != 0
            && block.template.spec_state.get() == crate::codegen::spec::OBSERVING
        {
            self.spec_observe_entry(&block.template, &args)
        } else {
            0
        };

        let mut env_frame = EnvFrame::new(block.parent_env);
        // Bind self
        env_frame.bind(self_symbol(), receiver);
        // Bind parameters
        for (sym, val) in block.template.param_syms.iter().zip(args.iter().copied()) {
            env_frame.bind(*sym, val);
        }
        let env_ref = gcl!(mc, env_frame);

        let is_nested_block = block.template.is_nested_block;
        let enclosing_method_id = if is_method_call {
            Some(frame_id)
        } else if is_nested_block {
            block.enclosing_method_id
        } else {
            Some(frame_id)
        };

        self.frames.push(Frame {
            id: frame_id,
            is_nested_block,
            enclosing_method_id,
            block,
            ic: block.inline_cache,
            ip: 0,
            env: env_ref,
            instantiating_obj: None,
            receiver: Some(receiver),
            selector,
            args,
            stack_base: self.stack.len(),
            spec_tid,
            return_receiver: false,
            defers: Vec::new(),
            unregister_on_defer_failure: None,
        });
    }

    pub fn start_block_for_instantiation(
        &mut self,
        mc: &Mutation<'gc>,
        block: Gc<'gc, Block<'gc>>,
        obj: Gc<'gc, RefLock<Object<'gc>>>,
        selector: Option<Symbol>,
    ) {
        let frame_id = self.next_frame_id;
        self.next_frame_id += 1;

        // The block runs in a fresh frame over its lexical parent only. Instance
        // variables are deliberately NOT pre-bound here: an empty `new:{}` block
        // must leave fields at their default (nil) rather than silently capturing
        // a same-named variable from the surrounding scope. A bare instance-var
        // name therefore reads up the lexical chain, and an explicit assignment is
        // what binds the field (see StoreLocal's instantiation-frame handling).
        let env_frame = EnvFrame::new(block.parent_env);
        let env_ref = gcl!(mc, env_frame);

        let is_nested_block = block.template.is_nested_block;
        let enclosing_method_id = if is_nested_block {
            block.enclosing_method_id
        } else {
            Some(frame_id)
        };

        self.frames.push(Frame {
            id: frame_id,
            is_nested_block,
            enclosing_method_id,
            block,
            ic: block.inline_cache,
            ip: 0,
            env: env_ref,
            instantiating_obj: Some(obj),
            receiver: Some(Value::Object(obj)),
            selector,
            args: Vec::new(),
            stack_base: self.stack.len(),
            spec_tid: 0,
            return_receiver: false,
            defers: Vec::new(),
            unregister_on_defer_failure: None,
        });
    }

    pub fn get_class_for_lookup(
        &self,
        receiver: Value<'gc>,
    ) -> Option<Gc<'gc, RefLock<Class<'gc>>>> {
        match receiver {
            Value::Int(_) | Value::Double(_) | Value::Bool(_) | Value::Nil => {
                self.immediate_class(receiver)
            }
            Value::Object(obj) => Some(obj.borrow().class),
            Value::Class(c) => Some(c),
            Value::ClassMeta(c) => Some(c),
        }
    }

    /// The dispatch class for an immediate value type, read from `builtin_cache`
    /// (populated at native-class registration) with a globals fallback. The
    /// booleans use their per-value singleton class once `true`/`false` have been
    /// extended, otherwise the shared `Boolean` class.
    fn immediate_class(&self, receiver: Value<'gc>) -> Option<Gc<'gc, RefLock<Class<'gc>>>> {
        let (cached, name) = {
            let c = self.builtin_cache.borrow();
            match receiver {
                Value::Int(_) => (c.integer_class, "Integer"),
                Value::Double(_) => (c.double_class, "Double"),
                Value::Bool(true) => (c.true_class.or(c.boolean_class), "Boolean"),
                Value::Bool(false) => (c.false_class.or(c.boolean_class), "Boolean"),
                Value::Nil => (c.nil_class, "Nil"),
                _ => return None,
            }
        };
        cached.or_else(|| {
            match self
                .globals
                .borrow()
                .get(&NamespacedName::parse(name))
                .copied()
            {
                Some(Value::Class(c)) => Some(c),
                _ => None,
            }
        })
    }

    /// Error if `class` is `sealed!` — refuses extension (`<--` / `->` / `-->` /
    /// `.mix:`) and subclassing of a sealed class (or an instance's sealed eigenclass).
    pub(crate) fn ensure_not_sealed(
        &self,
        class: Gc<'gc, RefLock<Class<'gc>>>,
    ) -> Result<(), QuoinError> {
        let c = class.borrow();
        if c.is_sealed {
            return Err(QuoinError::ClassError(if c.is_eigenclass {
                "Cannot extend a sealed instance".to_string()
            } else {
                format!("Cannot extend sealed class {}", c.name.to_explicit_string())
            }));
        }
        Ok(())
    }

    /// Error if `class` refuses `new` / `new:` on the class itself — either
    /// `abstract!`, or a native class whose generic instantiation fallback would
    /// mint a payload-less shell (`Class::native_new_refusal`). Concrete
    /// subclasses are unaffected, since neither flag is inherited.
    pub(crate) fn ensure_instantiable(
        &self,
        class: Gc<'gc, RefLock<Class<'gc>>>,
    ) -> Result<(), QuoinError> {
        let c = class.borrow();
        if c.is_abstract {
            return Err(QuoinError::ClassError(format!(
                "Cannot instantiate abstract class {}",
                c.name.to_explicit_string()
            )));
        }
        if let Some(hint) = c.native_new_refusal {
            return Err(QuoinError::ClassError(format!(
                "Cannot construct {} with new — {}",
                c.name.to_explicit_string(),
                hint
            )));
        }
        Ok(())
    }

    pub fn get_target_class_for_def(
        &mut self,
        mc: &Mutation<'gc>,
        receiver: Value<'gc>,
    ) -> Result<Gc<'gc, RefLock<Class<'gc>>>, String> {
        match receiver {
            Value::Class(c) => Ok(c),
            Value::ClassMeta(c) => Ok(c),
            // Extending a value type (`5 <-- {…}`, `Integer <-- {…}`) extends the
            // type itself — value types have no per-instance eigenclass.
            Value::Int(_) => Ok(self.get_or_create_builtin_class(mc, "Integer")),
            Value::Double(_) => Ok(self.get_or_create_builtin_class(mc, "Double")),
            Value::Nil => Ok(self.get_or_create_builtin_class(mc, "Nil")),
            // `true` and `false` carry distinct methods, so each gets its own
            // singleton class (parent `Boolean`), synthesized once and cached.
            Value::Bool(b) => {
                let existing = if b {
                    self.builtin_cache.borrow().true_class
                } else {
                    self.builtin_cache.borrow().false_class
                };
                if let Some(c) = existing {
                    return Ok(c);
                }
                let boolean = self.get_or_create_builtin_class(mc, "Boolean");
                let name = if b { "$TrueClass" } else { "$FalseClass" };
                let ns = NamespacedName::new(Vec::new(), name.to_string());
                let s = gcl!(
                    mc,
                    Class {
                        name: ns.clone(),
                        parent: Some(boolean),
                        instance_vars: Vec::new(),
                        instance_methods: FxHashMap::default(),
                        class_methods: FxHashMap::default(),
                        mixin_classes: Vec::new(),
                        field_slots: FxHashMap::default(),
                        init_plan: None,
                        is_eigenclass: false,
                        is_sealed: false,
                        is_abstract: false,
                        native_new_refusal: None,
                    }
                );
                self.globals.borrow_mut(mc).insert(ns, Value::Class(s));
                if b {
                    self.builtin_cache.borrow_mut(mc).true_class = Some(s);
                } else {
                    self.builtin_cache.borrow_mut(mc).false_class = Some(s);
                }
                Ok(s)
            }
            Value::Object(obj) => {
                let class_ref = obj.borrow().class;
                if class_ref.borrow().name.name.starts_with('$') {
                    Ok(class_ref)
                } else {
                    let mut singleton_name = class_ref.borrow().name.clone();
                    singleton_name.name = format!("${}", singleton_name.name);
                    // The eigenclass declares no new ivars, so it shares its base
                    // class's instance layout: it must carry the same field-slot map,
                    // or `@ivar` access on the instance (now of the eigenclass) can't
                    // resolve the inherited slots and reads them as nil.
                    let field_slots = class_ref.borrow().field_slots.clone();
                    let s = gcl!(
                        mc,
                        Class {
                            name: singleton_name,
                            parent: Some(class_ref),
                            instance_vars: Vec::new(),
                            instance_methods: FxHashMap::default(),
                            class_methods: FxHashMap::default(),
                            mixin_classes: Vec::new(),
                            field_slots,
                            init_plan: None,
                            is_eigenclass: true,
                            is_sealed: false,
                            is_abstract: false,
                            native_new_refusal: None,
                        }
                    );
                    obj.borrow_mut(mc).class = s;
                    Ok(s)
                }
            }
        }
    }

    pub fn annotate_error(&self, error: QuoinError) -> QuoinError {
        // An uncaught Quoin throw reaches here as `Thrown`; surface the actual
        // thrown value (which lives in `active_exception`) for display.
        let error = if matches!(error, QuoinError::Thrown) {
            let msg = match self.exceptions.active {
                Some(v) => format!("{}", v),
                None => "uncaught exception".to_string(),
            };
            QuoinError::Other(msg)
        } else {
            error
        };
        if matches!(error, QuoinError::WithSourceInfo { .. }) {
            return error;
        }
        if let Some(frame) = self.frames.last() {
            let active_ip = if frame.ip > 0 { frame.ip - 1 } else { 0 };
            let active_source_info = frame
                .block
                .template
                .source_map
                .get(active_ip)
                .and_then(|opt| opt.as_ref())
                .or(frame.block.template.source_info.as_ref())
                .cloned();
            if let Some(source_info) = active_source_info {
                let supports_color = self.options.supports_color;

                let colorize_selector = |sel: &str, cls: &str| -> String {
                    if supports_color {
                        format!("$#ab82ff[{}$]$#808080[:$]$#5fd7af[{}$]", sel, cls)
                    } else {
                        format!("{}:{}", sel, cls)
                    }
                };
                let colorize_simple = |sel: &str| -> String {
                    if supports_color {
                        format!("$#ab82ff[{}$]", sel)
                    } else {
                        sel.to_string()
                    }
                };

                let mut frames_info = Vec::new();
                let n = self.frames.len();
                for (i, f) in self.frames.iter().enumerate().rev() {
                    if i == n - 1 {
                        continue;
                    }
                    let frame_ip = if f.ip > 0 { f.ip - 1 } else { 0 };

                    let si_opt = f
                        .block
                        .template
                        .source_map
                        .get(frame_ip)
                        .and_then(|opt| opt.as_ref())
                        .or(f.block.template.source_info.as_ref())
                        .cloned();

                    // The failing instruction is a send — plain or a fused superinstruction;
                    // pull `(selector, num_args)` from whichever form it is.
                    let send_at_ip = match f.block.template.bytecode.get(frame_ip) {
                        Some(Instruction::Send(s, n))
                        | Some(Instruction::SendLocal(_, s, n))
                        | Some(Instruction::SendConst(_, s, n))
                        | Some(Instruction::SendField(_, s, n))
                        | Some(Instruction::SendLocalLocal(_, _, s, n))
                        | Some(Instruction::SendLocalConst(_, _, s, n)) => Some((*s, *n)),
                        _ => None,
                    };
                    let formatted_selector = if let Some((selector, num_args)) = send_at_ip {
                        let selector = selector.as_str();
                        let args_vec = if num_args > 0 {
                            if i == n - 1 {
                                self.exceptions.last_send_args.clone()
                            } else {
                                self.frames[i + 1].args.clone()
                            }
                        } else {
                            Vec::new()
                        };

                        if !args_vec.is_empty() {
                            let mut parts = Vec::new();
                            let mut current = String::new();
                            for c in selector.chars() {
                                current.push(c);
                                if c == ':' {
                                    parts.push(current);
                                    current = String::new();
                                }
                            }
                            if !current.is_empty() {
                                parts.push(current);
                            }

                            let mut formatted_parts = Vec::new();
                            for (idx, part) in parts.iter().enumerate() {
                                if let Some(arg) = args_vec.get(idx) {
                                    let mut p = part.clone();
                                    if p.ends_with(':') {
                                        p.pop();
                                    }
                                    formatted_parts.push(colorize_selector(&p, &arg.class_name()));
                                } else {
                                    formatted_parts.push(colorize_simple(part));
                                }
                            }
                            formatted_parts.join(" ")
                        } else {
                            colorize_simple(selector)
                        }
                    } else if i == 0 {
                        colorize_simple("(top)")
                    } else {
                        let sel_str = f
                            .selector
                            .map(|s| s.as_str().to_string())
                            .unwrap_or_else(|| "value".to_string());
                        colorize_simple(&sel_str)
                    };

                    let formatted_loc = if let Some(si) = &si_opt {
                        let display_filename = Path::new(&si.filename)
                            .file_name()
                            .and_then(|s| s.to_str())
                            .unwrap_or(&si.filename)
                            .to_string();
                        if supports_color {
                            format!(
                                " $#808080[in$] {}$#808080[:$]$#00bfff[{}$]$#808080[:$]$#00bfff[{}$]",
                                display_filename, si.line, si.column
                            )
                        } else {
                            format!(" in {}:{}:{}", display_filename, si.line, si.column)
                        }
                    } else {
                        "".to_string()
                    };

                    let at_str = if supports_color {
                        "$#808080[at$]"
                    } else {
                        "at"
                    };
                    let prefix_colored =
                        format!("{} {}{}", at_str, formatted_selector, formatted_loc);
                    let prefix_plain = if supports_color {
                        ansi_colorizer::decolorize(&ansi_colorizer::colorize(&prefix_colored))
                    } else {
                        prefix_colored.clone()
                    };
                    let plain_len = prefix_plain.chars().count();

                    frames_info.push((prefix_colored, plain_len, si_opt));
                }

                // Always append the (top) frame at the bottom if it was not already the only frame formatted as (top)
                if n > 0 {
                    let first_frame = &self.frames[0];
                    let first_ip = if first_frame.ip > 0 {
                        first_frame.ip - 1
                    } else {
                        0
                    };
                    let si_opt = first_frame
                        .block
                        .template
                        .source_map
                        .get(first_ip)
                        .and_then(|opt| opt.as_ref())
                        .or(first_frame.block.template.source_info.as_ref())
                        .cloned();

                    let formatted_selector = colorize_simple("(top)");

                    let formatted_loc = if let Some(si) = &si_opt {
                        let display_filename = Path::new(&si.filename)
                            .file_name()
                            .and_then(|s| s.to_str())
                            .unwrap_or(&si.filename)
                            .to_string();
                        if supports_color {
                            format!(
                                " $#808080[in$] {}$#808080[:$]$#00bfff[{}$]$#808080[:$]$#00bfff[{}$]",
                                display_filename, si.line, si.column
                            )
                        } else {
                            format!(" in {}:{}:{}", display_filename, si.line, si.column)
                        }
                    } else {
                        "".to_string()
                    };

                    let at_str = if supports_color {
                        "$#808080[at$]"
                    } else {
                        "at"
                    };
                    let prefix_colored =
                        format!("{} {}{}", at_str, formatted_selector, formatted_loc);
                    let prefix_plain = if supports_color {
                        ansi_colorizer::decolorize(&ansi_colorizer::colorize(&prefix_colored))
                    } else {
                        prefix_colored.clone()
                    };
                    let plain_len = prefix_plain.chars().count();

                    // Only push if the last trace element is not already representing (top) at the same location
                    let is_dup = if let Some(last_info) = frames_info.last() {
                        last_info.0 == prefix_colored
                    } else {
                        false
                    };

                    if !is_dup {
                        frames_info.push((prefix_colored, plain_len, si_opt));
                    }
                }

                let max_l = frames_info.iter().map(|info| info.1).max().unwrap_or(0);
                let target_alignment = cmp::max(54, max_l + 2);

                let console_width = self.options.console_width.unwrap_or(80) as usize;
                let available_width = console_width.saturating_sub(target_alignment + 4);
                let show_snippet = available_width >= 15;
                let w = available_width;

                let mut trace = Vec::new();
                for (prefix_colored, plain_len, si_opt) in frames_info {
                    let mut line = if supports_color {
                        ansi_colorizer::colorize(&prefix_colored)
                    } else {
                        prefix_colored
                    };

                    if let Some(si) = si_opt {
                        if show_snippet {
                            if let Some(snippet) = self.get_highlighted_snippet(
                                &si.filename,
                                si.line.saturating_sub(1),
                                si.column,
                                si.start,
                                si.end,
                                si.source_text.as_ref(),
                                w,
                            ) {
                                let padding_len = target_alignment.saturating_sub(plain_len);
                                let padding: String = " ".repeat(padding_len);
                                let separator = if supports_color {
                                    ansi_colorizer::colorize("$#808080[<$]")
                                } else {
                                    "<".to_string()
                                };
                                line = format!("{}{}{} {}", line, padding, separator, snippet);
                            }
                        }
                    }
                    trace.push(line);
                }

                return QuoinError::WithSourceInfo {
                    error: Box::new(error),
                    source_info: source_info.clone(),
                    trace,
                    supports_color,
                };
            }
        }
        error
    }

    /// Build a typed Quoin error object: an instance of `class_name` with its `message`
    /// field set, plus any `extra` (name, value) fields. Falls back to a plain string if
    /// the class isn't registered yet (e.g. an error fired during bootstrap before the
    /// Error hierarchy is defined). The typed `make_*` helpers below are thin wrappers —
    /// each domain error sets `message` plus its own structured fields.
    fn build_error_object(
        &self,
        mc: &Mutation<'gc>,
        class_name: &str,
        message: &str,
        extra: &[(&str, Value<'gc>)],
    ) -> Value<'gc> {
        let key = NamespacedName::new(Vec::new(), class_name.to_string());
        let class_opt = self.globals.borrow().get(&key).copied();
        if let Some(Value::Class(cls)) = class_opt {
            let obj = self.new_object(mc, cls);
            let msg_val = self.new_string(mc, message.to_string());
            if let Some(slot) = self.field_slot(cls, "message") {
                obj.borrow_mut(mc).fields[slot] = msg_val;
            }
            for (name, val) in extra {
                if let Some(slot) = self.field_slot(cls, name) {
                    obj.borrow_mut(mc).fields[slot] = *val;
                }
            }
            Value::Object(obj)
        } else {
            self.new_string(mc, message.to_string())
        }
    }

    /// Build a Quoin `Error` instance of the named class with `message`/`payload`.
    pub fn make_error(
        &self,
        mc: &Mutation<'gc>,
        class_name: &str,
        message: &str,
        payload: Option<Value<'gc>>,
    ) -> Value<'gc> {
        match payload {
            Some(p) => self.build_error_object(mc, class_name, message, &[("payload", p)]),
            None => self.build_error_object(mc, class_name, message, &[]),
        }
    }

    /// Build a Quoin `IoError` carrying `message` and a `kind` symbol (e.g.
    /// `#connectionRefused`).
    pub fn make_io_error(&self, mc: &Mutation<'gc>, kind: &str, message: &str) -> Value<'gc> {
        let kind_val = self.new_symbol(mc, kind.to_string());
        self.build_error_object(mc, "IoError", message, &[("kind", kind_val)])
    }

    /// Build a Quoin `IndexError` carrying `message` and the offending `index`/`length`.
    pub fn make_index_error(
        &self,
        mc: &Mutation<'gc>,
        index: i64,
        len: i64,
        message: &str,
    ) -> Value<'gc> {
        let index_val = self.new_int(mc, index);
        let length_val = self.new_int(mc, len);
        self.build_error_object(
            mc,
            "IndexError",
            message,
            &[("index", index_val), ("length", length_val)],
        )
    }

    /// Convert an internal `QuoinError` into the Quoin value a `catch:` handler should
    /// receive. Domain variants become typed `Error` objects so guest code can dispatch
    /// on them; control-flow signals and internal errors stay a descriptive string. The
    /// match is exhaustive over domain variants on purpose — a new typed error that
    /// forgets its arm here is then a compile error, not a silent fall-through to string.
    pub fn quoinerror_to_value(&self, mc: &Mutation<'gc>, error: &QuoinError) -> Value<'gc> {
        match error {
            QuoinError::TypeError { msg, .. } => self.make_error(mc, "TypeError", msg, None),
            QuoinError::ArgumentCountMismatch { msg, .. } => {
                self.make_error(mc, "ArgumentError", msg, None)
            }
            QuoinError::ArithmeticError(msg) => self.make_error(mc, "ArithmeticError", msg, None),
            QuoinError::MessageNotUnderstood {
                receiver, selector, ..
            } => {
                let msg = format!("no method '{}' for {}", selector, receiver);
                self.make_error(mc, "MessageNotUnderstood", &msg, None)
            }
            QuoinError::AmbiguousMethod { msg, .. } => {
                self.make_error(mc, "AmbiguousMethodError", msg, None)
            }
            QuoinError::Io { kind, message } => self.make_io_error(mc, kind.symbol(), message),
            QuoinError::IndexError { index, len, msg } => {
                self.make_index_error(mc, *index, *len, msg)
            }
            QuoinError::Timeout { ms } => {
                let ms_val = self.new_int(mc, *ms);
                self.build_error_object(mc, "TimeoutError", &error.to_string(), &[("ms", ms_val)])
            }
            QuoinError::ValueError(msg) => self.make_error(mc, "ValueError", msg, None),
            QuoinError::ParseError(msg) => self.make_error(mc, "ParseError", msg, None),
            QuoinError::ClassError(msg) => self.make_error(mc, "ClassError", msg, None),
            QuoinError::NameError(msg) => self.make_error(mc, "NameError", msg, None),
            QuoinError::StackExhausted(msg) => self.make_error(mc, "StackError", msg, None),
            QuoinError::ExtensionError(msg) => self.make_error(mc, "Error", msg, None),
            QuoinError::WithSourceInfo { error, .. } => self.quoinerror_to_value(mc, error),
            QuoinError::NotCallable(_)
            | QuoinError::StackUnderflow(_)
            | QuoinError::Other(_)
            | QuoinError::Thrown
            | QuoinError::NonLocalReturn
            | QuoinError::Cancelled
            | QuoinError::ExitRequested(_) => {
                let s = format!("{}", error);
                self.new_string(mc, s)
            }
        }
    }

    fn get_highlighted_snippet(
        &self,
        filename: &str,
        line_idx: usize,
        column: usize,
        node_start_offset: usize,
        node_end_offset: usize,
        source_text: Option<&String>,
        w: usize,
    ) -> Option<String> {
        let supports_color = self.options.supports_color;
        let content = match fs::read_to_string(filename) {
            Ok(s) => s,
            Err(_) => {
                if let Some(text) = source_text {
                    let snippet_text = if text.chars().count() > w {
                        let sliced: String = text.chars().take(w).collect();
                        sliced
                    } else {
                        text.clone()
                    };
                    if supports_color {
                        // Resilient: `snippet_text` is `source_text` truncated to `w`, so it can
                        // end mid-expression — `highlight_to_ansi` predictively completes it and
                        // never panics (and returns the text verbatim when it can't parse).
                        return Some(highlight_to_ansi(&snippet_text));
                    }
                    return Some(snippet_text);
                }
                return None;
            }
        };

        let mut current_line = 0;
        let mut line_start_byte = 0;
        let mut line_end_byte = content.len();
        for (i, c) in content.char_indices() {
            if c == '\n' {
                if current_line == line_idx {
                    line_end_byte = i;
                    break;
                }
                current_line += 1;
                line_start_byte = i + 1;
            }
        }
        if current_line != line_idx {
            if current_line == line_idx && line_start_byte <= content.len() {
                line_end_byte = content.len();
            } else {
                return None;
            }
        }

        if line_end_byte > line_start_byte && content.as_bytes()[line_end_byte - 1] == b'\r' {
            line_end_byte -= 1;
        }

        let line_str = &content[line_start_byte..line_end_byte];
        let line_chars: Vec<(usize, char)> = line_str.char_indices().collect();
        let line_char_count = line_chars.len();

        let node_text = content
            .get(node_start_offset..node_end_offset)
            .unwrap_or("");
        let node_char_count = node_text.chars().count();

        let start_col = cmp::min(column, line_char_count);
        let end_col = cmp::min(start_col + node_char_count, line_char_count);

        let node_center = start_col + (end_col - start_col) / 2;
        let mut win_start = node_center.saturating_sub(w / 2);
        let mut win_end = win_start + w;
        if win_end > line_char_count {
            let overflow = win_end - line_char_count;
            win_start = win_start.saturating_sub(overflow);
            win_end = line_char_count;
        }

        let get_char_byte_offset = |char_idx: usize| -> usize {
            if char_idx >= line_char_count {
                line_end_byte
            } else {
                line_start_byte + line_chars[char_idx].0
            }
        };

        let win_start_byte = get_char_byte_offset(win_start);
        let win_end_byte = get_char_byte_offset(win_end);
        let snippet_text = &content[win_start_byte..win_end_byte];

        if supports_color {
            // Resilient highlight of the full file, then crop spans to the window. Guarded on
            // non-empty so a file that can't be parsed/completed falls through to plain text
            // rather than panicking (old behavior) or emitting an empty snippet.
            let spans = highlight_resilient(&content);
            if !spans.is_empty() {
                let mut snippet_spans = Vec::new();
                for span in spans {
                    let overlap_start = cmp::max(span.start, win_start_byte);
                    let overlap_end = cmp::min(span.end, win_end_byte);
                    if overlap_start < overlap_end {
                        snippet_spans.push(HighlightSpan {
                            start: overlap_start - win_start_byte,
                            end: overlap_end - win_start_byte,
                            htype: span.htype,
                            counter: span.counter,
                        });
                    }
                }
                if !snippet_spans.is_empty() {
                    return Some(format_ansi(snippet_text, snippet_spans));
                }
            }
        }

        Some(snippet_text.to_string())
    }

    /// Materialize a `Constant` into a runtime `Value`. The body of the `Push` handler,
    /// shared with the fused `SendConst` superinstruction.
    fn materialize_constant(&mut self, mc: &Mutation<'gc>, constant: &Constant) -> Value<'gc> {
        match constant {
            Constant::Nil => self.new_nil(mc),
            Constant::Bool(b) => self.new_bool(mc, *b),
            Constant::Int(i) => self.new_int(mc, *i),
            Constant::Double(f) => self.new_double(mc, *f),
            Constant::String(s) => {
                let buf = self.literal_string_buffer(mc, s);
                self.new_string_shared(mc, buf)
            }
            Constant::Symbol(s) => self.new_symbol(mc, s.clone()),
            Constant::Block(sb) => {
                // Constant-closure promotion: a CLOSED template (no captures,
                // no self, no ^^) has one behavioral identity — reuse the
                // per-VM cached closure (shared with compiled make_closure,
                // so baked identity guards stay durable across calls).
                if let Some(tid) = sb.template_id
                    && crate::instruction::template_is_closed(sb)
                    && let Some(&v) = self.aot_closure_cache.get(&tid)
                {
                    return v;
                }
                // A closure is its shared template (Rc bump) plus the captured
                // runtime state — no deep clone of the param vectors.
                let parent_env = self.frames.last().map(|f| f.env);
                let enclosing_method_id = self.frames.last().and_then(|f| f.enclosing_method_id);
                let decl_block = sb.decl_block.as_ref().map(|db| {
                    let inline_cache = self.ic_cell_for(mc, db);
                    gc!(
                        mc,
                        Block {
                            template: db.clone(),
                            parent_env,
                            enclosing_method_id,
                            decl_block: None,
                            inline_cache,
                        }
                    )
                });
                let inline_cache = self.ic_cell_for(mc, sb);
                let block = Block {
                    template: sb.clone(),
                    parent_env,
                    enclosing_method_id,
                    decl_block,
                    inline_cache,
                };
                let v = self.new_block(mc, block);
                if let Some(tid) = sb.template_id
                    && crate::instruction::template_is_closed(sb)
                {
                    self.aot_closure_cache.insert(tid, v);
                }
                v
            }
        }
    }

    /// Read instance field `name` off `self` in the current frame. The body of the
    /// `LoadField` handler, shared with the fused `SendField` superinstruction.
    /// Missing/undeclared field (or a non-object `self`) reads as nil.
    /// `cache_ip`: the call site for the field-slot cache, or `None` to skip caching —
    /// `SendField` must pass `None`, because its *send* entry lives at the same `ip`
    /// (one fused instruction) and a field entry there would thrash the slot.
    /// `load_field` for compiled frames (S3): the receiver comes from the
    /// frame's slot window and the slot cache is the SHARED `(template_id,
    /// ip)` cell — both tiers warm one cache, the B3a outcall lesson applied
    /// to fields. Missing/undeclared/non-object reads are nil, exactly as
    /// interpreted.
    pub(crate) fn field_load_cached(
        &mut self,
        mc: &Mutation<'gc>,
        tid: u32,
        ip: usize,
        bc_len: usize,
        self_val: Value<'gc>,
        name: &str,
    ) -> Value<'gc> {
        let ic = self.ic_cell_by_id(mc, tid);
        if let Value::Object(obj) = self_val {
            let borrowed = obj.borrow();
            let class = borrowed.class;
            if let Some(slot) = self.field_probe(ic, ip, Gc::as_ptr(class) as usize) {
                let val = borrowed.fields.get(slot).copied();
                drop(borrowed);
                return val.unwrap_or_else(|| self.new_nil(mc));
            }
            drop(borrowed);
            match self.field_slot(class, name) {
                Some(slot) => {
                    self.field_fill_cell(mc, ic, bc_len, ip, class, slot);
                    obj.borrow()
                        .fields
                        .get(slot)
                        .copied()
                        .unwrap_or_else(|| self.new_nil(mc))
                }
                None => self.new_nil(mc),
            }
        } else {
            self.new_nil(mc)
        }
    }

    /// `store_field_value` for compiled frames (S3) — same shared-cell cache,
    /// same declared-field errors as interpreted.
    pub(crate) fn field_store_cached(
        &mut self,
        mc: &Mutation<'gc>,
        tid: u32,
        ip: usize,
        bc_len: usize,
        self_val: Value<'gc>,
        name: &str,
        val: Value<'gc>,
    ) -> Result<(), QuoinError> {
        let ic = self.ic_cell_by_id(mc, tid);
        if let Value::Object(obj) = self_val {
            let class = obj.borrow().class;
            if let Some(slot) = self.field_probe(ic, ip, Gc::as_ptr(class) as usize)
                && slot < obj.borrow().fields.len()
            {
                obj.borrow_mut(mc).fields[slot] = val;
                return Ok(());
            }
            match self.field_slot(class, name) {
                Some(slot) if slot < obj.borrow().fields.len() => {
                    self.field_fill_cell(mc, ic, bc_len, ip, class, slot);
                    obj.borrow_mut(mc).fields[slot] = val;
                    Ok(())
                }
                Some(_) => Err(QuoinError::Other(format!(
                    "Instance of '{}' has no '@{}' (it was added after this instance was created)",
                    class.borrow().name,
                    name
                ))),
                None => Err(QuoinError::Other(format!(
                    "No instance variable '@{}' declared on '{}'",
                    name,
                    class.borrow().name
                ))),
            }
        } else {
            Err(QuoinError::Other(format!(
                "Cannot set instance variable '@{}' on a value type ({})",
                name,
                self_val.type_name()
            )))
        }
    }

    fn load_field(
        &mut self,
        mc: &Mutation<'gc>,
        frame_idx: usize,
        cache_ip: Option<usize>,
        name: &str,
    ) -> Value<'gc> {
        let frame = &self.frames[frame_idx];
        let block = frame.block;
        let ic = frame.ic;
        let self_val = EnvFrame::get(frame.env, self_symbol()).unwrap_or_else(|| self.new_nil(mc));
        self.field_of(mc, block, ic, cache_ip, self_val, name)
    }

    /// Read instance field `name` off an arbitrary object value (the body of `LoadFieldOf`, and
    /// shared by `load_field` with `self`). Missing/undeclared field, or a non-object value => nil.
    /// `(block, ip)` is the executing call site, for the field-slot cache.
    fn field_of(
        &mut self,
        mc: &Mutation<'gc>,
        block: Gc<'gc, Block<'gc>>,
        ic: InlineCacheCell<'gc>,
        cache_ip: Option<usize>,
        obj_val: Value<'gc>,
        name: &str,
    ) -> Value<'gc> {
        if let Value::Object(obj) = obj_val {
            // Fast path: one object borrow, one cache probe, direct index — no class
            // borrow, no field-name hash.
            let borrowed = obj.borrow();
            let class = borrowed.class;
            if let Some(ip) = cache_ip
                && let Some(slot) = self.field_probe(ic, ip, Gc::as_ptr(class) as usize)
            {
                let val = borrowed.fields.get(slot).copied();
                drop(borrowed);
                return val.unwrap_or_else(|| self.new_nil(mc));
            }
            drop(borrowed);
            // No slot (undeclared) or a slot past this instance's array (declared on the
            // class after this object was created) => nil.
            match self.field_slot(class, name) {
                Some(slot) => {
                    if let Some(ip) = cache_ip {
                        self.field_fill(mc, block, ip, class, slot);
                    }
                    obj.borrow()
                        .fields
                        .get(slot)
                        .copied()
                        .unwrap_or_else(|| self.new_nil(mc))
                }
                None => self.new_nil(mc),
            }
        } else {
            self.new_nil(mc)
        }
    }

    /// Execute a send: pop `num_args` then the receiver off the stack and dispatch
    /// `selector`. Shared by the `Send` handler and the fused `Send*` superinstructions
    /// (which push the send's last operand first). Advances the caller frame's ip by one
    /// slot, then either tail-starts a block, invokes the resolved callable, or raises MNU.
    /// Fast path for a devirtualized Integer op (Slice 2a/2f): if the top two stack values
    /// are both `Int`, pop them and return `Some((a, b))`; otherwise leave them in place and
    /// return `None` so the caller falls back to the real send. This optimistic fallback is
    /// what lets `Int` be *inferred* for a mutable `var` (a stale-typed var is handled by the
    /// send) instead of only trusted for annotated params.
    fn take_two_ints(&mut self) -> Option<(i64, i64)> {
        let n = self.stack.len();
        if n < 2 {
            return None;
        }
        if let (Value::Int(a), Value::Int(b)) = (self.stack[n - 2], self.stack[n - 1]) {
            self.stack.truncate(n - 2);
            Some((a, b))
        } else {
            None
        }
    }

    /// Like `take_two_ints`, but for the `Double` devirt arms: pops the top two values iff both
    /// are `Value::Double`, else leaves the stack untouched so the caller can fall back to a send.
    fn take_two_doubles(&mut self) -> Option<(f64, f64)> {
        let n = self.stack.len();
        if n < 2 {
            return None;
        }
        if let (Value::Double(a), Value::Double(b)) = (self.stack[n - 2], self.stack[n - 1]) {
            self.stack.truncate(n - 2);
            Some((a, b))
        } else {
            None
        }
    }

    /// The fused-`Int`-op computation (Slice a1), shared by `IntBinLL`/`IntBinLC`. Matches the
    /// standalone `IntAdd`..`IntNe` arms exactly (arith wraps in release; `/`/`%` raise on a
    /// zero divisor; compares yield a Bool).
    fn int_bin_compute(kind: IntBinKind, a: i64, b: i64) -> Result<Value<'gc>, QuoinError> {
        Ok(match devirt_ops::int_bin(kind, a, b)? {
            devirt_ops::IntBinOut::Int(i) => Value::Int(i),
            devirt_ops::IntBinOut::Bool(b) => Value::Bool(b),
        })
    }

    /// The fused-`Double`-op computation, shared by `DoubleBinLL`/`DoubleBinLC`. Plain IEEE-754
    /// f64 — `/`/`%` yield inf/NaN on a zero divisor (never raise, unlike `int_bin_compute`), so
    /// it returns a `Value` directly rather than a `Result`.
    fn double_bin_compute(kind: IntBinKind, a: f64, b: f64) -> Value<'gc> {
        match devirt_ops::double_bin(kind, a, b) {
            devirt_ops::DoubleBinOut::Double(d) => Value::Double(d),
            devirt_ops::DoubleBinOut::Bool(b) => Value::Bool(b),
        }
    }

    /// Register unit-load AOT candidates (S0): classic annotated methods
    /// compile eagerly, block templates and speculative methods go pending
    /// (blocks tier by invocation count at the vWSOA seams; speculative
    /// methods first OBSERVE their param/return kinds here).
    pub fn register_aot_candidates(&mut self, cands: Vec<crate::codegen::AotCandidate>) {
        use crate::codegen::{AotRole, spec};
        let mut immediate = Vec::new();
        for cand in cands {
            let Some(tid) = cand.block.template_id else {
                continue;
            };
            if cand.role == AotRole::BlockTemplate {
                self.aot_pending_blocks
                    .insert(tid, (0, spec::K_UNKNOWN, cand));
            } else if cand.speculative() {
                cand.block.spec_state.set(spec::OBSERVING);
                let n_params = cand.params.len();
                self.aot_pending_spec.insert(
                    tid,
                    spec::SpecPending {
                        count: 0,
                        param_kinds: vec![spec::K_UNKNOWN; n_params],
                        ret_kind: spec::K_UNKNOWN,
                        cand,
                    },
                );
            } else {
                immediate.push(cand);
            }
        }
        if !immediate.is_empty() {
            crate::codegen::compile_candidates(immediate);
        }
    }

    /// The kind lattice value of a runtime value (spec-AOT observation).
    fn spec_kind(v: Value<'gc>) -> u8 {
        crate::codegen::spec::kind_of(v)
    }

    /// Merge a method entry's arg kinds into its speculative profile and
    /// return the tid for the frame to stash (`Frame.spec_tid`) — so the
    /// pop-side return observation never re-chases the template. Called on
    /// every method-frame push; the common case (template not OBSERVING) is
    /// one bounds-checked byte load.
    /// Cold path: the caller has already checked the template's `spec_state`
    /// Cell (the hot-path gate is inline at the push site). Returns the tid
    /// for `Frame.spec_tid`, or 0.
    #[cold]
    fn spec_observe_entry(&mut self, template: &Arc<StaticBlock>, args: &[Value<'gc>]) -> u32 {
        use crate::codegen::spec;
        let Some(tid) = template.template_id else {
            return 0;
        };
        let Some(p) = self.aot_pending_spec.get_mut(&tid) else {
            return 0;
        };
        for (lat, arg) in p.param_kinds.iter_mut().zip(args.iter()) {
            *lat = spec::merge(*lat, Self::spec_kind(*arg));
        }
        p.count += 1;
        self.aot_spec_obs_left -= 1;
        // A speculated RETURN needs at least one observed return before
        // promotion — a recursive method reaches warmth by ENTRIES alone
        // (fib descends past the threshold before its first base case), and
        // promoting with an unknown ret would compile Obj forever. Cap the
        // wait so a genuinely non-returning-yet method still promotes.
        let ret_pending = p.cand.spec_ret && p.ret_kind == spec::K_UNKNOWN;
        if p.count >= crate::codegen::warm_threshold()
            && (!ret_pending || p.count >= spec::OBSERVE_CAP)
        {
            self.spec_promote(tid);
            return 0; // promoted (or refused): no frame stash needed
        }
        tid
    }

    /// S1 promotion: compile a warm speculative method with its OBSERVED
    /// kinds. Scalar observations become the compiled params AND entry
    /// preconditions (checked by the dispatch arm; mismatch Bails to the
    /// interpreted body); `Obj`/unknown observations ride as Obj with no
    /// check. Annotated params were never speculated — dispatch guarantees
    /// them, exactly as before. The method-cache epoch bumps so call sites
    /// whose inline caches hold the interpreted callable re-fill with the
    /// compiled entry.
    fn spec_promote(&mut self, tid: u32) {
        use crate::codegen::spec;
        // Bisection debug hooks (they found every S1 seam bug):
        // QN_AOT_SPEC_MAX=<n> promotes only tids <= n;
        // QN_AOT_SPEC_ONLY=<csv> promotes only the listed tids.
        if let Ok(max) = std::env::var("QN_AOT_SPEC_MAX")
            && max.parse::<u32>().map(|m| tid > m).unwrap_or(false)
        {
            return;
        }
        if let Ok(only) = std::env::var("QN_AOT_SPEC_ONLY")
            && !only.split(',').any(|t| t.trim() == tid.to_string())
        {
            return;
        }
        let Some(pending) = self.aot_pending_spec.remove(&tid) else {
            return;
        };
        let mut cand = pending.cand;
        cand.block.spec_state.set(spec::RESOLVED);
        let mut preconds = vec![None; cand.params.len()];
        for i in 0..cand.params.len() {
            if cand.spec_params[i]
                && let Some(kind) = spec::scalar_kind(*pending.param_kinds.get(i).unwrap_or(&0))
            {
                cand.params[i] = crate::codegen::AotParam::Scalar(kind);
                preconds[i] = Some(kind);
            }
        }
        // S2: an observed-scalar RETURN compiles as a scalar too — statically
        // verified (a return path the translator can't prove demotes the ret
        // back to Obj and retries; no runtime narrowing, no wrong-type error
        // the interpreter wouldn't raise).
        if cand.spec_ret
            && let Some(kind) = spec::scalar_kind(pending.ret_kind)
        {
            cand.ret = crate::codegen::AotRet::Scalar(kind);
        }
        if preconds.iter().any(|p| p.is_some()) {
            cand.spec_preconditions = preconds;
        }
        let sel = cand.selector.clone();
        crate::codegen::compile_candidates(vec![cand]);
        if crate::codegen::block_registered(tid) {
            if std::env::var("QN_AOT_VERBOSE").is_ok_and(|v| v == "1") {
                eprintln!("qn aot: promoted {sel} (tid {tid})");
            }
            self.aot_spec_promoted += 1;
            self.invalidate_method_cache();
        }
    }

    /// Merge a method's return kind into its speculative profile. `tid` comes
    /// from the popped frame's `spec_tid` (set at push), so this only runs
    /// for frames that were observing; the state re-check tolerates
    /// saturation between push and pop.
    #[cold]
    fn spec_observe_return(&mut self, tid: u32, ret: Value<'gc>) {
        use crate::codegen::spec;
        if let Some(p) = self.aot_pending_spec.get_mut(&tid) {
            p.ret_kind = spec::merge(p.ret_kind, Self::spec_kind(ret));
        }
    }

    /// One-line profile summary for `QN_AOT_STATS=1`.
    pub fn aot_spec_stats(&self) -> String {
        use crate::codegen::spec;
        let observing = self
            .aot_pending_spec
            .values()
            .filter(|p| p.cand.block.spec_state.get() == spec::OBSERVING)
            .count();
        let (compiled, refused) = crate::codegen::compile_totals();
        let mut lines = vec![format!(
            "spec-aot: {} pending ({} observing), {} promoted; {} compiled, {} refused (QN_AOT_VERBOSE=1 for reasons)",
            self.aot_pending_spec.len(),
            observing,
            self.aot_spec_promoted,
            compiled,
            refused
        )];
        let mut profiled: Vec<_> = self
            .aot_pending_spec
            .values()
            .filter(|p| p.count > 0)
            .collect();
        profiled.sort_by_key(|p| std::cmp::Reverse(p.count));
        for p in profiled.iter().take(12) {
            let kinds: Vec<&str> = p.param_kinds.iter().map(|&k| spec::kind_name(k)).collect();
            lines.push(format!(
                "  {} x{}: ({}) -> {}",
                p.cand.selector,
                p.count,
                kinds.join(", "),
                spec::kind_name(p.ret_kind)
            ));
        }
        lines.join("\n")
    }

    /// Materialize a runtime closure from a compiled template: the thin
    /// `{template, captured state}` pair plus the (possibly registry-shared)
    /// inline-cache cell. Shared by the runner entry points, eval, and string
    /// interpolation; `materialize_constant` inlines the same shape.
    pub fn block_from_template(
        &mut self,
        mc: &Mutation<'gc>,
        template: Arc<StaticBlock>,
        parent_env: Option<Gc<'gc, RefLock<EnvFrame<'gc>>>>,
        enclosing_method_id: Option<usize>,
    ) -> Gc<'gc, Block<'gc>> {
        let decl_block = template.decl_block.as_ref().map(|db| {
            let inline_cache = self.ic_cell_for(mc, db);
            gc!(
                mc,
                Block {
                    template: db.clone(),
                    parent_env,
                    enclosing_method_id,
                    decl_block: None,
                    inline_cache,
                }
            )
        });
        let inline_cache = self.ic_cell_for(mc, &template);
        gc!(
            mc,
            Block {
                template,
                parent_env,
                enclosing_method_id,
                decl_block,
                inline_cache,
            }
        )
    }

    /// The inline-cache cell for a closure materialized from `template`: the shared
    /// per-template cell from `ic_registry` when the template has an id (so every
    /// closure of one literal warms the same call sites), or a fresh private cell
    /// for id-less runtime-built blocks.
    pub(crate) fn ic_cell_for(
        &mut self,
        mc: &Mutation<'gc>,
        template: &Arc<StaticBlock>,
    ) -> Gc<'gc, RefLock<Option<Box<[ICSlot<'gc>]>>>> {
        match template.template_id {
            Some(id) => {
                if let Some(cell) = self.ic_registry.get(&id) {
                    *cell
                } else {
                    let cell = gcl!(mc, None);
                    self.ic_registry.insert(id, cell);
                    cell
                }
            }
            None => gcl!(mc, None),
        }
    }

    /// The shared IC cell for a template id directly — the compiled-code twin of
    /// `ic_cell_for` (outcall sites pass their `(template_id, ip)`, which is the
    /// same call-site identity the interpreted send at that instruction uses, so
    /// compiled and interpreted execution warm ONE cache).
    pub(crate) fn ic_cell_by_id(
        &mut self,
        mc: &Mutation<'gc>,
        id: u32,
    ) -> Gc<'gc, RefLock<Option<Box<[ICSlot<'gc>]>>>> {
        if let Some(cell) = self.ic_registry.get(&id) {
            *cell
        } else {
            let cell = gcl!(mc, None);
            self.ic_registry.insert(id, cell);
            cell
        }
    }

    /// Receiver-phase probe of a D2 site cell: live epoch + receiver guard,
    /// checked BEFORE the caller decodes any argument lanes, so a site whose
    /// target is not compiled (a native, a polymorphic receiver) pays a few
    /// loads and nothing else. Returns the cell BY COPY (it is `Copy`; the
    /// `parent_env` Gc stays rooted in the traced `aot_sites` vec) for the
    /// argument-phase check.
    #[inline]
    /// Block-call site peek (D2-for-blocks): the identity is the block's
    /// TEMPLATE id (every closure shares the `Block` class, so the method
    /// cells' receiver-class guard would alias all of them). Returns the
    /// cached entry when live.
    #[inline]
    pub(crate) fn aot_block_site_peek(
        &self,
        site: usize,
        template_id: u32,
    ) -> Option<(&'static crate::codegen::AotEntry, u32)> {
        let cell = self.aot_sites.get(site)?;
        let entry = cell.entry?;
        if cell.epoch != self.dispatch_epoch || entry.template_id != template_id {
            return None;
        }
        Some((entry, cell.hits))
    }

    /// Fill a block-call site cell (entry + epoch only — the template-id
    /// guard lives on the entry itself).
    pub(crate) fn aot_block_site_fill(
        &mut self,
        site: usize,
        entry: &'static crate::codegen::AotEntry,
        recv: Value<'gc>,
    ) {
        if site >= self.aot_sites.len() {
            self.aot_sites.resize(site + 1, AotSiteCell::default());
        }
        let cell = &mut self.aot_sites[site];
        *cell = AotSiteCell::default();
        cell.epoch = self.dispatch_epoch;
        cell.entry = Some(entry);
        cell.recv_val = Some(recv);
    }

    pub(crate) fn aot_site_peek(
        &self,
        site: usize,
        receiver: Value<'gc>,
        n_args: usize,
    ) -> Option<AotSiteCell<'gc>> {
        let cell = self.aot_sites.get(site)?;
        cell.entry?;
        if cell.epoch != self.dispatch_epoch || cell.n_args as usize != n_args {
            return None;
        }
        let (rk, rp) = value_type_guard(receiver);
        if cell.recv_kind != rk || cell.recv_ptr != rp {
            return None;
        }
        Some(*cell)
    }

    /// Argument-phase check for a peeked D2 cell (see [`Self::aot_site_peek`]).
    #[inline]
    /// One lane of [`aot_site_args_match`] — the D2.5b helper fast path
    /// guards verbatim scalar lanes by lane-kind compare and only routes
    /// GENERAL lanes (Obj / precondition-narrowed) through this shape guard.
    pub(crate) fn aot_site_arg_match_one(cell: &AotSiteCell<'gc>, i: usize, a: Value<'gc>) -> bool {
        let (ak, ap) = value_type_guard(a);
        cell.arg_kinds[i] == ak && cell.arg_ptrs[i] == ap
    }

    /// D3a: count a fast-path hit; crossing the `QN_DIRECT_WARM` threshold
    /// queues the CALLER tid for retranslation (deduped, process-lifetime).
    #[inline(always)]
    pub(crate) fn aot_site_note_hit(&mut self, site: usize, caller_tid: u32) {
        let Some(threshold) = crate::codegen::direct_warm_threshold() else {
            return;
        };
        let Some(cell) = self.aot_sites.get_mut(site) else {
            return;
        };
        // Saturate at the threshold: a warm site's hits become a read-only
        // compare — the unconditional per-hit WRITE dirtied the cell's cache
        // line millions of times on call-heavy programs (measured ~2% on
        // richards even with the counter inlined).
        if cell.hits >= threshold {
            return;
        }
        cell.hits += 1;
        if cell.hits == threshold && self.aot_retranslate_queued.insert(caller_tid) {
            self.aot_retranslate_queue.push(caller_tid);
        }
    }

    /// Drain the retranslation queue (driver-boundary caller).
    pub(crate) fn take_retranslations(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.aot_retranslate_queue)
    }

    /// D3b activation, the TARGETED form: clear exactly the caches holding
    /// `tid`'s (now replaced) entry — its D2 site cells and interpreted IC
    /// slots — so the next resolution refills from the registry and picks
    /// up the retranslated code. Everything else stays warm, and earlier
    /// batches' baked guards stay LIVE (the wholesale dispatch-epoch bump
    /// this replaces stranded every prior batch's edges and re-warmed the
    /// world per batch — measured btrees +3.2%/richards +3.7%). Runs at the
    /// driver boundary; O(total cached slots), rare.
    pub(crate) fn invalidate_caches_for_template(&mut self, mc: &Mutation<'gc>, tid: u32) {
        for cell in &mut self.aot_sites {
            if cell.entry.is_some_and(|e| e.template_id == tid) {
                *cell = AotSiteCell::default();
            }
        }
        for ic in self.ic_registry.values() {
            let mut slots = ic.borrow_mut(mc);
            if let Some(slots) = slots.as_mut() {
                for slot in slots.iter_mut() {
                    let stale = matches!(
                        &slot.callable,
                        Some(crate::dispatch::Callable::AotCall { entry, .. })
                            if entry.0.template_id == tid
                    );
                    if stale {
                        *slot = ICSlot::empty();
                    }
                }
            }
        }
    }

    /// D3b: capture baked W0 facts for a caller's retained sites — warm,
    /// current-epoch, monomorphic cells whose entry meets the W0 tier
    /// criteria. Runs in the driver's drain (the translator has no VM).
    pub(crate) fn bake_w0_sites(
        &self,
        sites: &rustc_hash::FxHashMap<usize, u32>,
        threshold: u32,
    ) -> (
        rustc_hash::FxHashMap<usize, crate::codegen::BakedW0>,
        Vec<Value<'gc>>,
    ) {
        let mut out = rustc_hash::FxHashMap::default();
        let mut roots = Vec::new();
        for (&ip, &site) in sites {
            let Some(cell) = self.aot_sites.get(site as usize) else {
                continue;
            };
            let Some(entry) = cell.entry else { continue };
            let is_block = crate::codegen::block_w0_eligible(entry);
            let eligible = crate::codegen::w0_eligible(entry) || is_block;
            if cell.epoch != self.dispatch_epoch || cell.hits < threshold || !eligible {
                continue;
            }
            if is_block {
                // Identity bake: the guard compares the receiver slot's 16
                // bytes against this exact closure NATIVELY (fixed Value
                // layout). Pin the closure for the code's lifetime — a
                // recycled address must never false-positive the guard.
                let Some(rv) = cell.recv_val else { continue };
                let Value::Object(obj) = rv else { continue };
                roots.push(rv);
                out.insert(
                    ip,
                    crate::codegen::BakedW0 {
                        entry,
                        epoch: self.dispatch_epoch,
                        recv_kind: 4, // Value tag: Object
                        recv_ptr: Gc::as_ptr(obj) as usize,
                    },
                );
                continue;
            }
            out.insert(
                ip,
                crate::codegen::BakedW0 {
                    entry,
                    epoch: self.dispatch_epoch,
                    recv_kind: cell.recv_kind,
                    recv_ptr: cell.recv_ptr,
                },
            );
        }
        (out, roots)
    }

    /// Fill a D2 site cell. The caller gates this on the interpreted IC
    /// having filled for the same resolution (probe-after-fill), which
    /// carries over every cacheability rule (guard-free, non-eigenclass,
    /// arg-count bound) without restating them.
    pub(crate) fn aot_site_fill(
        &mut self,
        site: usize,
        receiver: Value<'gc>,
        args: &[Value<'gc>],
        entry: &'static crate::codegen::AotEntry,
        parent_env: Option<Gc<'gc, RefLock<EnvFrame<'gc>>>>,
    ) {
        if args.len() > IC_MAX_ARGS {
            return;
        }
        if site >= self.aot_sites.len() {
            self.aot_sites.resize(site + 1, AotSiteCell::default());
        }
        let (recv_kind, recv_ptr) = value_type_guard(receiver);
        let mut arg_kinds = [0u8; IC_MAX_ARGS];
        let mut arg_ptrs = [0usize; IC_MAX_ARGS];
        for (i, a) in args.iter().enumerate() {
            let (ak, ap) = value_type_guard(*a);
            arg_kinds[i] = ak;
            arg_ptrs[i] = ap;
        }
        self.aot_sites[site] = AotSiteCell {
            epoch: self.dispatch_epoch,
            hits: 0,
            recv_kind,
            recv_ptr,
            n_args: args.len() as u8,
            arg_kinds,
            arg_ptrs,
            entry: Some(entry),
            parent_env,
            recv_val: None,
        };
    }

    /// `call_method`, with the caller's inline cache consulted and filled — the
    /// compiled-code outcall path (B3a): without it every compiled operator send
    /// paid an uncached `lookup_method` while the interpreted body it replaced
    /// had warm ICs, which measurably REGRESSED arithmetic-heavy blocks.
    pub fn call_method_cached(
        &mut self,
        mc: &Mutation<'gc>,
        tid: u32,
        ip: usize,
        bc_len: usize,
        receiver: Value<'gc>,
        selector: Symbol,
        args: Vec<Value<'gc>>,
        site: Option<u32>,
    ) -> Result<Value<'gc>, QuoinError> {
        // No `enter_native_reentry` here (unlike `call_method`): charging the
        // 12-deep hook-recursion budget per outcall made a 12-deep chain of
        // PROMOTED methods (S1: everything unannotated compiles) a spurious
        // "recursion too deep". Instead, `outcall_nesting` counts the REAL
        // hazard — Rust-stack frames per compiled<->interpreted alternation —
        // and dispatch degrades to interpreted bodies past the cap.
        self.aot.outcall_nesting += 1;
        let result =
            self.call_method_cached_inner(mc, tid, ip, bc_len, receiver, selector, args, site);
        self.aot.outcall_nesting = self.aot.outcall_nesting.saturating_sub(1);
        result
    }

    // The IC cell local is a copy of a `Gc` rooted in the traced `ic_registry`
    // for the VM's whole life — safe across `lookup_method`'s guard-predicate
    // yields by that rooting, which the span heuristic can't see.
    #[allow(no_gc_across_yield)]
    fn call_method_cached_inner(
        &mut self,
        mc: &Mutation<'gc>,
        tid: u32,
        ip: usize,
        bc_len: usize,
        receiver: Value<'gc>,
        selector: Symbol,
        args: Vec<Value<'gc>>,
        site: Option<u32>,
    ) -> Result<Value<'gc>, QuoinError> {
        let ic = self.ic_cell_by_id(mc, tid);
        let method = match self.ic_probe(ic, ip, receiver, &args) {
            Some(c) => {
                // D2 gap (found by D3b): a caller that TIERS UP mid-run has
                // warm interpreted ICs, so the cold-arm fill below never
                // runs and its site cells stay cold forever — the D2 fast
                // path was inert for every spec-promoted caller. Fill on a
                // probe-hit too, under the same once-per-epoch gate (the
                // polymorphic-flip tax the cold-arm comment guards against
                // stays impossible: a warm cell short-circuits here).
                if let (Some(site), crate::dispatch::Callable::AotCall { block, entry }) =
                    (site, &c)
                    && self.aot_sites.get(site as usize).is_none_or(|cell| {
                        cell.entry.is_none() || cell.epoch != self.dispatch_epoch
                    })
                {
                    self.aot_site_fill(site as usize, receiver, &args, entry.0, block.parent_env);
                }
                Some(c)
            }
            None => {
                let m = self.lookup_method(mc, receiver, selector, &args)?;
                if let Some(c) = &m {
                    self.ic_fill_cell(mc, ic, bc_len, ip, receiver, selector, &args, c.clone());
                    // D2: mirror the resolution into the site cell — but only
                    // when the IC actually filled (probe-after-fill), so the
                    // site cache inherits ic_fill_cell's cacheability rules;
                    // and only ONCE PER EPOCH per site — a polymorphic site
                    // re-resolves cold on every receiver flip, and re-running
                    // the probe + rewriting the cell each time taxed exactly
                    // the sites that can never benefit.
                    if let (Some(site), crate::dispatch::Callable::AotCall { block, entry }) =
                        (site, c)
                        && self.aot_sites.get(site as usize).is_none_or(|cell| {
                            cell.entry.is_none() || cell.epoch != self.dispatch_epoch
                        })
                        && self.ic_probe(ic, ip, receiver, &args).is_some()
                    {
                        self.aot_site_fill(
                            site as usize,
                            receiver,
                            &args,
                            entry.0,
                            block.parent_env,
                        );
                    }
                }
                m
            }
        };
        if let Some(method) = method {
            let initial_frame_count = self.frames.len();
            if matches!(
                method,
                crate::dispatch::Callable::Native(_) | crate::dispatch::Callable::AotCall { .. }
            ) {
                // Same stack-window rooting as `exec_send` (A2c): outcall
                // args arrive in an owned Vec (decoded from compiled lanes,
                // never on the value stack), so push them once — two stack
                // writes beat the rooting clone. The frame-count
                // discriminator below is exact: a synchronous call pushes no
                // frame; the AotCall interpreter fallbacks consume the
                // window themselves before pushing theirs.
                let recv_start = self.stack.len();
                self.push(receiver);
                for &a in &args {
                    self.push(a);
                }
                let res = method.call(
                    self,
                    mc,
                    Some(receiver),
                    args,
                    Some(selector),
                    Some(recv_start + 1),
                );
                if let Err(e) = res {
                    // The S1/finish_frame rule, as in `dispatch_send_rooted`:
                    // an escaping `^^` already delivered at (possibly) the
                    // window start — only non-NLR errors tear down here.
                    if !matches!(e, QuoinError::NonLocalReturn) {
                        self.stack.truncate(recv_start.min(self.stack.len()));
                    }
                    return Err(e);
                }
                if self.frames.len() > initial_frame_count {
                    // An interpreter fallback started a frame (window
                    // already consumed by the dispatch arm): drive it.
                    self.run_nested(mc, initial_frame_count, "method call")?;
                    Ok(self.pop()?)
                } else {
                    let result = self.pop()?;
                    self.stack.truncate(recv_start);
                    Ok(result)
                }
            } else {
                method.call(self, mc, Some(receiver), args, Some(selector), None)?;
                self.run_nested(mc, initial_frame_count, "method call")?;
                Ok(self.pop()?)
            }
        } else {
            // Same service-proxy forwarding as exec_send's miss branch —
            // compiled callers reach proxies through this outcall arm.
            if let Some(res) = crate::runtime::worker_service::try_service_call(
                self, mc, receiver, selector, &args,
            ) {
                if res.is_err() {
                    self.exceptions.last_send_args = args;
                }
                return res;
            }
            // No method: raise EXACTLY what the interpreted send raises
            // (candidates included). This arm returned nil since the first
            // outcall shell, which made a warm compiled outcall silently
            // "succeed" where the same interpreted send raised
            // MessageNotUnderstood — a parity hole that hid real errors the
            // moment a block template or promoted method warmed up.
            // (`call_method_inner` — the native `call_method` helper — still
            // has the legacy nil arm: its callers are host-ops and hooks
            // with their own absent-method conventions, out of scope here.)
            let candidates = self
                .collect_method_candidates(receiver, selector)
                .iter()
                .map(|&mv| self.format_candidate_signature(mv, selector))
                .collect();
            let receiver_name = receiver.class_name();
            let arg_names = args.iter().map(|a| a.class_name()).collect();
            self.exceptions.last_send_args = args;
            Err(QuoinError::MessageNotUnderstood {
                receiver: receiver_name,
                selector: selector.as_str().to_string(),
                args: arg_names,
                candidates,
            })
        }
    }

    /// Probe the executing `block`'s inline cache at `ip` for a *field-slot* entry
    /// (see [`IC_FIELD_KIND`]): a hit returns the receiver-class's slot index for the
    /// field named at this instruction, skipping the `field_slots` hash lookup and
    /// the class borrow. Guarded on the exact class pointer — inherited methods run
    /// the same `Gc<Block>` for every subclass, and the same field name maps to a
    /// *different* slot per class, so the guard is load-bearing.
    #[inline]
    fn field_probe(&self, ic: InlineCacheCell<'gc>, ip: usize, class_ptr: usize) -> Option<usize> {
        let cache = ic.borrow();
        let slot = cache.as_ref()?.get(ip)?;
        if slot.epoch != self.dispatch_epoch
            || slot.recv_kind != IC_FIELD_KIND
            || slot.recv_ptr != class_ptr
        {
            return None;
        }
        Some(slot.arg_ptrs[0])
    }

    /// Memoize `class`'s slot index for the field read/written at `(block, ip)`.
    /// Eigenclasses are never cached (transient pointers — same ABA rule as the
    /// dispatch cache); their accesses just re-run the hash lookup. Slot indices are
    /// append-only per class (see `Class::field_slots`), so a cached entry can't go
    /// stale; the epoch guard is belt-and-braces and gives O(1) invalidation anyway.
    /// `field_fill` for a cell reached by template id (compiled field access,
    /// S3) — same slot-cache protocol, shared with the interpreted site.
    fn field_fill_cell(
        &mut self,
        mc: &Mutation<'gc>,
        cell: Gc<'gc, RefLock<Option<Box<[ICSlot<'gc>]>>>>,
        bc_len: usize,
        ip: usize,
        class: Gc<'gc, RefLock<Class<'gc>>>,
        slot_idx: usize,
    ) {
        if class.borrow().is_eigenclass {
            return;
        }
        Self::ic_write_slot(
            mc,
            cell,
            bc_len,
            ip,
            ICSlot {
                epoch: self.dispatch_epoch,
                recv_kind: IC_FIELD_KIND,
                recv_ptr: Gc::as_ptr(class) as usize,
                n_args: 0,
                arg_kinds: [0; IC_MAX_ARGS],
                arg_ptrs: [slot_idx, 0],
                callable: None,
            },
        );
    }

    fn field_fill(
        &mut self,
        mc: &Mutation<'gc>,
        block: Gc<'gc, Block<'gc>>,
        ip: usize,
        class: Gc<'gc, RefLock<Class<'gc>>>,
        slot_idx: usize,
    ) {
        if class.borrow().is_eigenclass {
            return;
        }
        let epoch = self.dispatch_epoch;
        let mut cache = block.inline_cache.borrow_mut(mc);
        if cache.is_none() {
            *cache = Some(vec![ICSlot::empty(); block.template.bytecode.len()].into_boxed_slice());
        }
        if let Some(slot) = cache.as_mut().and_then(|slots| slots.get_mut(ip)) {
            *slot = ICSlot {
                epoch,
                recv_kind: IC_FIELD_KIND,
                recv_ptr: Gc::as_ptr(class) as usize,
                n_args: 0,
                arg_kinds: [0; IC_MAX_ARGS],
                arg_ptrs: [slot_idx, 0],
                callable: None,
            };
        }
    }

    /// Probe a site's cache for a fused-instantiation verdict (`IC_PLAINNEW_KIND`):
    /// a hit is (class-ptr, epoch)-guarded, same protocol as `field_probe`.
    #[inline]
    fn plain_new_probe(
        &self,
        ic: InlineCacheCell<'gc>,
        ip: usize,
        class_ptr: usize,
    ) -> Option<bool> {
        let cache = ic.borrow();
        let slot = cache.as_ref()?.get(ip)?;
        if slot.epoch != self.dispatch_epoch
            || slot.recv_kind != IC_PLAINNEW_KIND
            || slot.recv_ptr != class_ptr
        {
            return None;
        }
        Some(slot.arg_ptrs[0] != 0)
    }

    /// Does any class in the class-side chain (own, ancestors, mixins —
    /// transitively) define a `new:` method? Over-approximates dispatch on
    /// purpose: a typed user variant that would NOT match a Block argument
    /// still answers true here, which only sends the site to the cold path
    /// (the real send then falls through to `Callable::New` exactly as today).
    fn hierarchy_defines_class_new(
        &self,
        class: Gc<'gc, RefLock<Class<'gc>>>,
        visited: &mut Vec<Gc<'gc, RefLock<Class<'gc>>>>,
    ) -> bool {
        if visited.iter().any(|c| Gc::ptr_eq(*c, class)) {
            return false;
        }
        visited.push(class);
        let c = class.borrow();
        if c.class_methods.contains_key(&new_colon_symbol()) {
            return true;
        }
        if let Some(parent) = c.parent
            && self.hierarchy_defines_class_new(parent, visited)
        {
            return true;
        }
        c.mixin_classes
            .iter()
            .any(|m| self.hierarchy_defines_class_new(*m, visited))
    }

    /// The fused-instantiation verdict (M2, `BranchIfNotPlainNew`): does `new:`
    /// on this receiver resolve to the BUILT-IN `Callable::New` — the fallback
    /// `lookup_method` returns only when NO user `new:` exists anywhere in the
    /// class-side chain — with an instantiable class? False sends the site to
    /// the cold path (the real send), so a conservative false is never wrong.
    fn plain_new_verdict(&self, receiver: Value<'gc>) -> bool {
        let Value::Class(class) = receiver else {
            return false;
        };
        let mut visited = Vec::new();
        if self.hierarchy_defines_class_new(class, &mut visited) {
            return false;
        }
        self.ensure_instantiable(class).is_ok()
    }

    /// Cached `plain_new_verdict`: probe/fill `cell` at `ip` when the receiver
    /// is a (non-eigenclass) class; other receivers recompute (always false).
    pub(crate) fn plain_new_check_cached(
        &mut self,
        mc: &Mutation<'gc>,
        cell: Option<(InlineCacheCell<'gc>, usize)>,
        ip: usize,
        receiver: Value<'gc>,
    ) -> bool {
        if let (Some((cell, _)), Value::Class(class)) = (cell, receiver)
            && let Some(v) = self.plain_new_probe(cell, ip, Gc::as_ptr(class) as usize)
        {
            return v;
        }
        let verdict = self.plain_new_verdict(receiver);
        if let (Some((cell, bc_len)), Value::Class(class)) = (cell, receiver)
            && !class.borrow().is_eigenclass
        {
            Self::ic_write_slot(
                mc,
                cell,
                bc_len,
                ip,
                ICSlot {
                    epoch: self.dispatch_epoch,
                    recv_kind: IC_PLAINNEW_KIND,
                    recv_ptr: Gc::as_ptr(class) as usize,
                    n_args: 0,
                    arg_kinds: [0; IC_MAX_ARGS],
                    arg_ptrs: [usize::from(verdict), 0],
                    callable: None,
                },
            );
        }
        verdict
    }

    /// The fused-instantiation body (M2, `NewWithFields`): the stack holds
    /// `[class, v1..vn]` with the class at `base - 1` and `names[i]` naming
    /// `v(i+1)`'s field; the window is replaced by the finished object.
    /// Reached only through a true `BranchIfNotPlainNew` verdict, so the
    /// receiver was a plain-instantiable class when the field expressions
    /// started evaluating — exactly the point `Callable::New` commits today.
    /// `instantiation_plan` re-derives per epoch, so a field expression that
    /// mutated the class mid-evaluation (adding an `init`) is still honored:
    /// the non-empty-plan path below IS `finalize_instantiation`, fed an env
    /// holding exactly the named bindings (`lookup_str` is local-only, so a
    /// parentless env is indistinguishable from the config frame's).
    pub(crate) fn exec_new_with_fields(
        &mut self,
        mc: &Mutation<'gc>,
        base: usize,
        names: &[Symbol],
    ) -> Result<(), QuoinError> {
        let recv_at = base
            .checked_sub(1)
            .ok_or_else(|| QuoinError::Other("Stack underflow".to_string()))?;
        let Value::Class(class) = self.stack[recv_at] else {
            return Err(QuoinError::Other(
                "NewWithFields: receiver is not a class".to_string(),
            ));
        };
        let obj = self.new_object(mc, class);
        let plan = self.instantiation_plan(mc, class);
        if plan.inits.is_empty() {
            // Direct field binds: `finalize_instantiation` with an empty chain
            // reduces to exactly this (unknown names silently dropped there
            // too — it iterates ivar_slots and looks each up in the env).
            for (i, sym) in names.iter().enumerate() {
                let val = self.stack[base + i];
                if let Some((_, slot)) = plan
                    .ivar_slots
                    .iter()
                    .find(|(n, _)| n.as_str() == sym.as_str())
                {
                    obj.borrow_mut(mc).fields[*slot] = val;
                }
            }
            self.stack.truncate(recv_at);
            self.push(Value::Object(obj));
        } else {
            // Root the object in the receiver slot across the init chain (an
            // init can park); the values stay rooted in the window and env.
            self.stack[recv_at] = Value::Object(obj);
            let mut env = EnvFrame::new(None);
            for (i, sym) in names.iter().enumerate() {
                env.bind(*sym, self.stack[base + i]);
            }
            let env = gcl!(mc, env);
            self.finalize_instantiation(mc, obj, env)?;
            let out = self.stack[recv_at];
            self.stack.truncate(recv_at);
            self.push(out);
        }
        Ok(())
    }

    /// Probe the executing `block`'s inline cache at `ip`: a hit requires a live epoch (method
    /// tables unchanged) and matching receiver + argument type-shape guards. Immediates match on
    /// their cheap `Value` discriminant with no class derivation — the whole point. Sound with no
    /// ABA guard: the cache cell is shared per *template*, rooted in `ic_registry` for the VM's
    /// lifetime, and template ids are never reused — `(template, ip)` is a stable call-site
    /// identity. Entries are guard-free resolutions keyed only on receiver/arg type-shape +
    /// epoch, so sharing one array across every closure (and concurrent activation) of the same
    /// literal is sound.
    #[inline]
    fn ic_probe(
        &self,
        ic: InlineCacheCell<'gc>,
        ip: usize,
        receiver: Value<'gc>,
        args: &[Value<'gc>],
    ) -> Option<Callable<'gc>> {
        if args.len() > IC_MAX_ARGS {
            return None;
        }
        let cache = ic.borrow();
        let slot = cache.as_ref()?.get(ip)?;
        if slot.epoch != self.dispatch_epoch || slot.n_args as usize != args.len() {
            return None;
        }
        let (rk, rp) = value_type_guard(receiver);
        if slot.recv_kind != rk || slot.recv_ptr != rp {
            return None;
        }
        for (i, a) in args.iter().enumerate() {
            let (ak, ap) = value_type_guard(*a);
            if slot.arg_kinds[i] != ak || slot.arg_ptrs[i] != ap {
                return None;
            }
        }
        slot.callable
    }

    /// Fill the executing `block`'s inline-cache slot at `ip` — but only for a **guard-free**
    /// resolution, i.e. one the global cache also memoized (a guarded dispatch depends on
    /// argument *values*, not just types, so it must never be inline-cached). The global-cache
    /// lookup here is cold: it runs only on an IC miss, which for a monomorphic site happens
    /// once. The block's per-`ip` array is allocated lazily (sized to its bytecode) on first fill.
    fn ic_fill(
        &mut self,
        mc: &Mutation<'gc>,
        block: Gc<'gc, Block<'gc>>,
        ip: usize,
        receiver: Value<'gc>,
        selector: Symbol,
        args: &[Value<'gc>],
        callable: Callable<'gc>,
    ) {
        if args.len() > IC_MAX_ARGS {
            return;
        }
        let class_side = matches!(receiver, Value::Class(_));
        let Some(class_ref) = self.get_class_for_lookup(receiver) else {
            return;
        };
        let Some(key) = self.method_cache_key(class_ref, selector, class_side, args) else {
            return;
        };
        if !matches!(self.dispatch_cache.entries.get(&key), Some(Some(_))) {
            return; // uncacheable (guarded) or not a hierarchy method — don't inline-cache
        }
        let epoch = self.dispatch_epoch;
        let (recv_kind, recv_ptr) = value_type_guard(receiver);
        let mut arg_kinds = [0u8; IC_MAX_ARGS];
        let mut arg_ptrs = [0usize; IC_MAX_ARGS];
        for (i, a) in args.iter().enumerate() {
            let (ak, ap) = value_type_guard(*a);
            arg_kinds[i] = ak;
            arg_ptrs[i] = ap;
        }
        // The cache cell is its own `Gc<RefLock<…>>` (shared across every closure of
        // the same template via `ic_registry`), so mutate it directly through the
        // write barrier, same idiom as `globals`.
        Self::ic_write_slot(
            mc,
            block.inline_cache,
            block.template.bytecode.len(),
            ip,
            ICSlot {
                epoch,
                recv_kind,
                recv_ptr,
                n_args: args.len() as u8,
                arg_kinds,
                arg_ptrs,
                callable: Some(callable),
            },
        );
    }

    fn ic_write_slot(
        mc: &Mutation<'gc>,
        cell: Gc<'gc, RefLock<Option<Box<[ICSlot<'gc>]>>>>,
        bc_len: usize,
        ip: usize,
        new_slot: ICSlot<'gc>,
    ) {
        let mut cache = cell.borrow_mut(mc);
        if cache.is_none() {
            *cache = Some(vec![ICSlot::empty(); bc_len].into_boxed_slice());
        }
        if let Some(slot) = cache.as_mut().and_then(|slots| slots.get_mut(ip)) {
            *slot = new_slot;
        }
    }

    /// `ic_fill` for a cell reached by template id (the compiled outcall path) —
    /// the same guards and cacheability rules, no `Gc<Block>` needed.
    #[allow(clippy::too_many_arguments)]
    fn ic_fill_cell(
        &mut self,
        mc: &Mutation<'gc>,
        cell: Gc<'gc, RefLock<Option<Box<[ICSlot<'gc>]>>>>,
        bc_len: usize,
        ip: usize,
        receiver: Value<'gc>,
        selector: Symbol,
        args: &[Value<'gc>],
        callable: Callable<'gc>,
    ) {
        if args.len() > IC_MAX_ARGS {
            return;
        }
        let class_side = matches!(receiver, Value::Class(_));
        let Some(class_ref) = self.get_class_for_lookup(receiver) else {
            return;
        };
        let Some(key) = self.method_cache_key(class_ref, selector, class_side, args) else {
            return;
        };
        if !matches!(self.dispatch_cache.entries.get(&key), Some(Some(_))) {
            return; // uncacheable (guarded/tag-requiring) — never inline-cache
        }
        let epoch = self.dispatch_epoch;
        let (recv_kind, recv_ptr) = value_type_guard(receiver);
        let mut arg_kinds = [0u8; IC_MAX_ARGS];
        let mut arg_ptrs = [0usize; IC_MAX_ARGS];
        for (i, a) in args.iter().enumerate() {
            let (ak, ap) = value_type_guard(*a);
            arg_kinds[i] = ak;
            arg_ptrs[i] = ap;
        }
        Self::ic_write_slot(
            mc,
            cell,
            bc_len,
            ip,
            ICSlot {
                epoch,
                recv_kind,
                recv_ptr,
                n_args: args.len() as u8,
                arg_kinds,
                arg_ptrs,
                callable: Some(callable),
            },
        );
    }

    // GC-rooting: the only yield reachable from here is a *guarded* method's guard
    // predicate (`lookup_method` -> `match_score` -> `execute_validation_block`), and
    // that binds `receiver` as the guard's `self` and each `args` element as a guard
    // parameter into the guard env frame before it steps — so both are rooted through
    // any yield. `caller_block` is a copy of `self.frames[frame_idx].block`, rooted by
    // the live frame stack. Nothing here is held across a yield unrooted.
    fn exec_send(
        &mut self,
        mc: &Mutation<'gc>,
        frame_idx: usize,
        selector: Symbol,
        num_args: usize,
    ) -> Result<VmStatus<'gc>, QuoinError> {
        // The operands sit in ORDER at the stack top. Copy the args in one
        // exact-size allocation, but leave `[receiver, args..]` LIVE on the
        // stack: for Native/AotCall callables that window IS the GC root for
        // the whole call (no rooting clone — see `NativeArgs::StackWindow`),
        // torn down in `dispatch_send_rooted` after the call returns. Frame-
        // pushing callables consume the window before their frame instead.
        let args_start = self
            .stack
            .len()
            .checked_sub(num_args)
            .ok_or("Stack underflow")?;
        let recv_start = args_start.checked_sub(1).ok_or("Stack underflow")?;
        let args: Vec<Value<'gc>> = self.stack[args_start..].to_vec();
        let receiver = self.stack[recv_start];
        // Call-site identity for the inline cache: the executing frame's cache cell + the
        // Send's own `ip`, captured before we advance it (the block itself is re-read at
        // fill time — see the note at `ic_fill` below).
        let caller_ic = self.frames[frame_idx].ic;
        let site_ip = self.frames[frame_idx].ip;
        self.frames[frame_idx].ip += 1; // Advance caller frame IP

        if let Value::Object(obj) = receiver
            && let ObjectPayload::Block(block) = &obj.borrow().payload
        {
            if selector.as_str() == "value" || selector.as_str() == "value:" {
                let block = *block;
                self.stack.truncate(recv_start);
                self.start_block(mc, block, args, Some(receiver), Some(selector));
                return Ok(VmStatus::Running);
            }
        }

        // Inline-cache fast path: a hit skips `lookup_method`'s key-build + hash + hashmap.
        if let Some(callable) = self.ic_probe(caller_ic, site_ip, receiver, &args) {
            return self.dispatch_send_rooted(mc, callable, receiver, args, selector, recv_start);
        }

        // `last_send_args` is read only by the stack-trace formatter, and only for an
        // innermost send that fails *in place* (no callee frame of its own): a failed
        // lookup, a `MessageNotUnderstood`, or a native-method error (the last captured
        // inside `Callable::call`). On success the args move into the callee frame
        // (`Frame.args`), which the formatter reads instead — so we snapshot only on
        // these error branches, not every send.
        let method_opt = match self.lookup_method(mc, receiver, selector, &args) {
            Ok(m) => m,
            Err(e) => {
                self.stack.truncate(recv_start);
                self.exceptions.last_send_args = args;
                return Err(e);
            }
        };
        if let Some(callable) = method_opt {
            // Re-read rather than reuse `caller_block`: `lookup_method` above
            // can run guard blocks (yield-capable); the frame itself stays in
            // the traced `self.frames`, so the fresh read is always rooted.
            self.ic_fill(
                mc,
                self.frames[frame_idx].block,
                site_ip,
                receiver,
                selector,
                &args,
                callable,
            );
            self.dispatch_send_rooted(mc, callable, receiver, args, selector, recv_start)
        } else {
            // A WorkerService proxy forwards any selector its class doesn't
            // define (docs/CONCURRENCY_ARCH.md §10 L4) — the hook sits on
            // this lookup-miss branch, so the hot path never pays for it.
            if let Some(res) = crate::runtime::worker_service::try_service_call(
                self, mc, receiver, selector, &args,
            ) {
                self.stack.truncate(recv_start);
                return match res {
                    Ok(v) => {
                        self.push(v);
                        Ok(VmStatus::Running)
                    }
                    Err(e) => {
                        self.exceptions.last_send_args = args;
                        Err(e)
                    }
                };
            }
            // The selector may still exist with non-matching signatures; surface those
            // filtered-out variants as a hint.
            let candidates = self
                .collect_method_candidates(receiver, selector)
                .iter()
                .map(|&mv| self.format_candidate_signature(mv, selector))
                .collect();
            let receiver_name = receiver.class_name();
            let arg_names = args.iter().map(|a| a.class_name()).collect();
            self.stack.truncate(recv_start);
            self.exceptions.last_send_args = args;
            Err(QuoinError::MessageNotUnderstood {
                receiver: receiver_name,
                selector: selector.as_str().to_string(),
                args: arg_names,
                candidates,
            })
        }
    }

    /// Dispatch a send whose `[receiver, args..]` window is still LIVE on the
    /// value stack at `stack[recv_start..]` (see `exec_send`). Native and
    /// AotCall callables run with the window as their GC root — no rooting
    /// clone — and their pushed result is re-seated over the window
    /// afterwards. Everything else (interpreted methods, guarded variants,
    /// ext methods) consumes the window up front, exactly as before. The
    /// AotCall arm's interpreter fallbacks truncate the window themselves
    /// before pushing their frame, so after an `Ok` the discriminator is the
    /// stack height: above `recv_start` = a synchronous result to re-seat;
    /// at it = a frame was started and there is nothing to move.
    fn dispatch_send_rooted(
        &mut self,
        mc: &Mutation<'gc>,
        callable: crate::dispatch::Callable<'gc>,
        receiver: Value<'gc>,
        args: Vec<Value<'gc>>,
        selector: Symbol,
        recv_start: usize,
    ) -> Result<VmStatus<'gc>, QuoinError> {
        use crate::dispatch::Callable;
        match callable {
            Callable::Native(_) | Callable::AotCall { .. } => {
                let res = callable.call(
                    self,
                    mc,
                    Some(receiver),
                    args,
                    Some(selector),
                    Some(recv_start + 1),
                );
                match res {
                    Ok(()) => {
                        if self.stack.len() > recv_start {
                            let result = self.pop()?;
                            self.stack.truncate(recv_start);
                            self.push(result);
                        }
                        Ok(VmStatus::Running)
                    }
                    Err(e) => {
                        // NLR-aware teardown — the S1/finish_frame rule: a
                        // `^^` that escaped through this send has already
                        // truncated to its target's base and pushed the
                        // delivered value there, and that base can sit AT or
                        // ABOVE this window's start (a caller whose operand
                        // stack was empty at the send). Touching the stack
                        // then chops the delivery; every OTHER error tears
                        // the window down here.
                        if !matches!(e, QuoinError::NonLocalReturn) {
                            self.stack.truncate(recv_start.min(self.stack.len()));
                        }
                        Err(e)
                    }
                }
            }
            _ => {
                self.stack.truncate(recv_start);
                callable.call(self, mc, Some(receiver), args, Some(selector), None)?;
                Ok(VmStatus::Running)
            }
        }
    }

    /// Bind `name` in the current frame to an already-obtained `val`. Shared by the
    /// `DefineLocal` handler (pops) and `DefineLocalKeep` (peeks).
    fn store_define_local(
        &mut self,
        mc: &Mutation<'gc>,
        frame_idx: usize,
        name: Symbol,
        val: Value<'gc>,
    ) -> Result<(), QuoinError> {
        if matches!(name.as_str(), "true" | "false" | "nil") {
            let err_msg = format!("Can't modify reserved identifier {}", name);
            self.exceptions.active = Some(self.new_string(mc, err_msg.clone()));
            return Err(QuoinError::Other(err_msg));
        }
        self.frames[frame_idx].env.borrow_mut(mc).bind(name, val);
        Ok(())
    }

    /// Assign `name` to an already-obtained `val`: inside a `new:{}` block bind locally
    /// (object init), else set up the lexical chain or bind. Shared by `StoreLocal`
    /// (pops) and `StoreLocalKeep` (peeks).
    fn store_set_local(
        &mut self,
        mc: &Mutation<'gc>,
        frame_idx: usize,
        name: Symbol,
        val: Value<'gc>,
    ) -> Result<(), QuoinError> {
        if matches!(name.as_str(), "true" | "false" | "nil") {
            let err_msg = format!("Can't modify reserved identifier {}", name);
            self.exceptions.active = Some(self.new_string(mc, err_msg.clone()));
            return Err(QuoinError::Other(err_msg));
        }
        let frame = &mut self.frames[frame_idx];
        // Init-form binding is STATIC (E): a `new:{...}` config literal's
        // assignments bind into its own frame however it is invoked — the
        // frame flag covers real instantiation, the template flag covers a
        // user-defined `new:` running the block as a plain closure
        // (previously that chain-walked the write: caller-dependent
        // semantics nothing could reason about, the AOT gates included).
        if frame.instantiating_obj.is_some() || frame.block.template.is_init_literal {
            frame.env.borrow_mut(mc).bind(name, val);
        } else if !EnvFrame::set(frame.env, mc, name, val) {
            frame.env.borrow_mut(mc).bind(name, val);
        }
        Ok(())
    }

    /// Store an already-obtained `val` into instance field `name` on `self`. Shared by
    /// `StoreField` (pops) and `StoreFieldKeep` (peeks).
    fn store_field_value(
        &mut self,
        mc: &Mutation<'gc>,
        frame_idx: usize,
        ip: usize,
        name: &str,
        val: Value<'gc>,
    ) -> Result<(), QuoinError> {
        let frame = &self.frames[frame_idx];
        let block = frame.block;
        let ic = frame.ic;
        let self_val = EnvFrame::get(frame.env, self_symbol()).unwrap_or_else(|| self.new_nil(mc));
        if let Value::Object(obj) = self_val {
            let class = obj.borrow().class;
            // Fast path: cached slot for this exact class at this call site. A hit
            // implies the field is declared; the length guard below still applies
            // (an instance can predate a later-added ivar).
            if let Some(slot) = self.field_probe(ic, ip, Gc::as_ptr(class) as usize)
                && slot < obj.borrow().fields.len()
            {
                obj.borrow_mut(mc).fields[slot] = val;
                return Ok(());
            }
            match self.field_slot(class, name) {
                Some(slot) if slot < obj.borrow().fields.len() => {
                    self.field_fill(mc, block, ip, class, slot);
                    obj.borrow_mut(mc).fields[slot] = val;
                }
                Some(_) => {
                    // Declared on the class, but this instance predates it (a mixin added
                    // the ivar after the object was created); shape is fixed at construction.
                    return Err(QuoinError::Other(format!(
                        "Instance of '{}' has no '@{}' (it was added after this instance was created)",
                        class.borrow().name,
                        name
                    )));
                }
                None => {
                    // You cannot create an instance variable by assigning to it.
                    return Err(QuoinError::Other(format!(
                        "No instance variable '@{}' declared on '{}'",
                        name,
                        class.borrow().name
                    )));
                }
            }
        } else {
            // Immediate value types (Integer/Double/Boolean/Nil) have no per-instance
            // fields — setting `@x` on one is an error.
            return Err(QuoinError::Other(format!(
                "Cannot set instance variable '@{}' on a value type ({})",
                name,
                self_val.type_name()
            )));
        }
        Ok(())
    }

    pub fn step(&mut self, mc: &Mutation<'gc>) -> Result<VmStatus<'gc>, QuoinError> {
        let res = self.step_internal(mc);
        if let Err(QuoinError::NonLocalReturn) = res {
            return Ok(VmStatus::Running);
        }
        // Cancellation bypasses source annotation (like NonLocalReturn) so it reaches
        // the scheduler as a bare `Cancelled` rather than wrapped in `WithSourceInfo`.
        if let Err(QuoinError::Cancelled) = res {
            return Err(QuoinError::Cancelled);
        }
        // A requested process exit likewise stays bare so the driver can match it.
        if let Err(QuoinError::ExitRequested(code)) = res {
            return Err(QuoinError::ExitRequested(code));
        }
        if let Err(e) = res {
            return Err(self.annotate_error(e));
        }
        res
    }

    /// Execute a single VM instruction. The one-step entry point kept for the synchronous
    /// sub-execution loops, the debugger, and `qn benchmark`; it clones the current frame's
    /// bytecode `Rc` per call. The hot path (`run_vm_loop`) uses `run_dispatch`, which hoists
    /// that clone out of the per-instruction path.
    pub(crate) fn step_internal(
        &mut self,
        mc: &Mutation<'gc>,
    ) -> Result<VmStatus<'gc>, QuoinError> {
        if self.sched.cancel_current {
            return Err(self.take_cancellation());
        }
        if self.frames.is_empty() {
            let ret = self.pop().unwrap_or_else(|_| self.new_nil(mc));
            return Ok(VmStatus::Finished(ret));
        }
        let bytecode = self.frames[self.frames.len() - 1]
            .block
            .template
            .bytecode
            .clone();
        self.dispatch_one(mc, &bytecode)
    }

    /// Run up to `budget` instructions in one flat loop, hoisting the current frame's bytecode
    /// `Rc` into a local — cloned only when the frame stack changes (a call pushes / a return
    /// pops), not once per instruction. This is the hot dispatch path driven by `run_vm_loop`.
    /// It folds in the cancellation, empty-stack, and error handling that `step` +
    /// `step_internal` do per instruction, so the result feeds `run_vm_loop` directly. Returns
    /// `Running` once the budget is spent (i.e. "yield now"). The held `Rc` keeps the bytecode
    /// alive across frame changes and GC, exactly as the per-step clone did.
    pub(crate) fn run_dispatch(
        &mut self,
        mc: &Mutation<'gc>,
        budget: u32,
    ) -> Result<VmStatus<'gc>, QuoinError> {
        let mut cached_len = usize::MAX;
        let mut bytecode: Option<SharedBytecode> = None;
        let mut steps = 0u32;
        loop {
            if self.sched.cancel_current {
                return Err(self.take_cancellation());
            }
            if self.frames.is_empty() {
                let ret = self.pop().unwrap_or_else(|_| self.new_nil(mc));
                return Ok(VmStatus::Finished(ret));
            }
            let flen = self.frames.len();
            if flen != cached_len {
                cached_len = flen;
                bytecode = Some(self.frames[flen - 1].block.template.bytecode.clone());
            }
            let bc = bytecode.as_ref().unwrap();
            match self.dispatch_one(mc, bc) {
                // A completed instruction, or a `^`/`^^` non-local return that unwound frames
                // (`step` maps `NonLocalReturn` to `Running`). Count it; the changed frame
                // stack re-hoists next iteration. An in-flight COMPILED-home `^^`
                // can never surface here: the owning `codegen::invoke` always sits
                // between the `^^` and this top loop (dispatch_one's AotCall arm
                // consumes the delivery) — asserted, because absorbing one would
                // desync the S5 protocol and truncate under a live frame.
                Ok(VmStatus::Running) | Err(QuoinError::NonLocalReturn) => {
                    // (No result binding here: this arm runs once per
                    // interpreted instruction, and binding the Drop-glued
                    // Result cost a measured ~6% on combinators. The assert
                    // holds on BOTH variants — the target must be None
                    // whenever the top loop is running at all.)
                    debug_assert!(
                        self.aot.nlr_target.is_none(),
                        "in-flight compiled-home ^^ surfaced at the top dispatch loop"
                    );
                    steps += 1;
                    if steps >= budget {
                        return Ok(VmStatus::Running);
                    }
                }
                Ok(other) => return Ok(other),
                Err(QuoinError::Cancelled) => return Err(QuoinError::Cancelled),
                Err(QuoinError::ExitRequested(code)) => {
                    return Err(QuoinError::ExitRequested(code));
                }
                Err(e) => return Err(self.annotate_error(e)),
            }
        }
    }

    /// One instruction, hoisted-bytecode form (the giant dispatch `match`). `bytecode` is the
    /// current frame's bytecode `Rc` held by the caller (`step_internal` per-call, or
    /// `run_dispatch` once per frame-entry), so `inst` borrows the caller's local — not
    /// `self` — leaving handlers full `&mut self`. Callers guarantee `self.frames` is
    /// non-empty and no cancellation is pending.
    ///
    /// `ip` is hoisted into a local (Slice b2, the ip-register hoist on top of b1's flat
    /// loop): fall-through arms advance it as `ip += 1` in a register, instead of a
    /// bounds-checked `self.frames[frame_idx].ip += 1` per instruction, and the guarded
    /// write-back at the tail syncs it to the frame. **Invariant:** an arm that advances `ip`
    /// and then leaves via an early `return` (or a value-return like `ExecuteBlockWithSelf`'s
    /// `return if …`), rather than falling through, MUST sync it itself with
    /// `self.frames[frame_idx].ip = ip` — the tail write-back only runs on fall-through. A
    /// violation is never silent: it surfaces immediately as a stack imbalance under the
    /// `.qn` suite.
    /// Run the deferred calls queued on `frames[frame_idx]` (e.g. mixin
    /// requirement checks) *before* popping it, so the defer queue — and any
    /// Values it references — stays GC-rooted via `self.frames` even if a
    /// defer yields and a collection happens during the suspension. Iterates
    /// a clone to satisfy the borrow checker; the originals stay in the
    /// (still-live) frame to keep their Values reachable. Defers run only on
    /// NORMAL completion (the `Return` and implicit end-of-bytecode arms —
    /// never a `^^` unwinding through the frame); if one throws and this is
    /// a new class definition, the class is unregistered first.
    #[inline]
    fn run_frame_defers(&mut self, mc: &Mutation<'gc>, frame_idx: usize) -> Result<(), QuoinError> {
        if self.frames[frame_idx].defers.is_empty() {
            return Ok(());
        }
        let defers = self.frames[frame_idx].defers.clone();
        if let Err(e) = self.run_defers(mc, &defers) {
            if let Some(name) = self.frames[frame_idx].unregister_on_defer_failure.clone() {
                self.globals.borrow_mut(mc).remove(&name);
                // The class is gone; its pointer could be reused, so drop
                // any memoized resolutions that might reference it.
                self.invalidate_method_cache();
            }
            return Err(e);
        }
        Ok(())
    }

    /// Consume a just-popped frame COMPLETELY — the discipline every pop
    /// site shares (used by the `MethodReturn` unwind and the implicit-
    /// return SLOW path; the `Return` arm and the implicit fast path
    /// OPEN-CODE the same steps for speed — their comments say why. Keep
    /// them in lockstep with this): destructure the frame's fields out
    /// first, because `finalize_instantiation` can park (an init that
    /// sleeps) and a collection while parked leaves any Gc pointer still
    /// held on this suspended stack dangling (the S0 segfault). The rooting
    /// contract across that park: the instantiating object rides the VM
    /// stack, and the frame's env rides `last_popped_env`. The
    /// receiver-return applies first and the instantiation pop overwrites
    /// it. Returns the (possibly replaced) return value plus the frame's
    /// `spec_tid` — the CALLER decides whether to observe the return (the
    /// `MethodReturn` unwind observes only at the target frame) and owns
    /// the value-stack truncation policy (per-frame vs once-at-target).
    fn consume_popped_frame(
        &mut self,
        mc: &Mutation<'gc>,
        frame: Frame<'gc>,
        mut ret_val: Value<'gc>,
    ) -> Result<(Value<'gc>, u32), QuoinError> {
        let Frame {
            spec_tid,
            env,
            receiver,
            return_receiver,
            instantiating_obj,
            ..
        } = frame;
        self.last_popped_env = Some(env);
        if return_receiver && let Some(rx) = receiver {
            ret_val = rx;
        }
        if let Some(obj) = instantiating_obj {
            self.push(Value::Object(obj));
            self.finalize_instantiation(mc, obj, env)?;
            ret_val = self.pop()?;
        }
        Ok((ret_val, spec_tid))
    }

    pub(crate) fn dispatch_one(
        &mut self,
        mc: &Mutation<'gc>,
        bytecode: &SharedBytecode,
    ) -> Result<VmStatus<'gc>, QuoinError> {
        let frame_idx = self.frames.len() - 1;
        // Hoisted instruction pointer (Slice b2): read once, advanced in a register by the
        // arms, synced back at the tail. See the invariant on this fn.
        let mut ip = self.frames[frame_idx].ip;
        let inst = match bytecode.0.get(ip) {
            Some(i) => i,
            None => {
                // Implicit return Nil — a NORMAL completion, so the frame-
                // teardown discipline is the `Return` arm's. This arm is HOT
                // (a fused loop's exit jump lands one past the last
                // instruction), so the common plain frame stays on a minimal
                // path; the rare shapes (defers, an instantiation, a
                // receiver-return) take the full shared discipline — they
                // used to be silently SKIPPED here, a divergence waiting for
                // the first such frame to end implicitly.
                let f = &self.frames[frame_idx];
                if !f.defers.is_empty() || f.instantiating_obj.is_some() || f.return_receiver {
                    self.run_frame_defers(mc, frame_idx)?;
                    let ret_val = self.new_nil(mc);
                    let popped = self.frames.pop().unwrap();
                    self.stack.truncate(popped.stack_base);
                    let (ret_val, spec_tid) = self.consume_popped_frame(mc, popped, ret_val)?;
                    if spec_tid != 0 {
                        self.spec_observe_return(spec_tid, ret_val);
                    }
                    self.push(ret_val);
                    return Ok(VmStatus::Running);
                }
                let ret_val = self.new_nil(mc);
                let popped = self.frames.pop().unwrap();
                self.stack.truncate(popped.stack_base);
                if popped.spec_tid != 0 {
                    self.spec_observe_return(popped.spec_tid, ret_val);
                }
                self.last_popped_env = Some(popped.env);
                self.push(ret_val);
                return Ok(VmStatus::Running);
            }
        };

        // Debugger checkpoint: only active while a session is attached (otherwise one bool
        // load). May suspend with `DebugBreak` to hand control to the driver; transparent —
        // execution continues here (then dispatches `inst`) on resume. `inst` borrows the
        // local `bytecode` clone, not `self`, so `&mut self` here is fine.
        if self.instrumentation.debug.is_some() {
            self.debug_checkpoint(frame_idx, ip)?;
        }
        if self.instrumentation.coverage.is_some() {
            self.coverage_tick(frame_idx, ip);
        }

        match inst {
            Instruction::LoadLocal(name) => {
                let name = *name;
                let frame = &self.frames[frame_idx];
                let val = EnvFrame::get(frame.env, name).unwrap_or_else(|| self.new_nil(mc));
                self.push(val);
                ip += 1;
            }
            Instruction::DefineLocal(name) => {
                let name = *name;
                let val = self.pop()?;
                self.store_define_local(mc, frame_idx, name, val)?;
                ip += 1;
            }
            // Store-and-keep: store the top of stack without popping it (fused `Dup;
            // DefineLocal`, an assignment used as an expression).
            Instruction::DefineLocalKeep(name) => {
                let name = *name;
                let val = self.peek()?;
                self.store_define_local(mc, frame_idx, name, val)?;
                ip += 1;
            }
            Instruction::StoreLocal(name) => {
                let name = *name;
                let val = self.pop()?;
                self.store_set_local(mc, frame_idx, name, val)?;
                ip += 1;
            }
            Instruction::StoreLocalKeep(name) => {
                let name = *name;
                let val = self.peek()?;
                self.store_set_local(mc, frame_idx, name, val)?;
                ip += 1;
            }
            Instruction::LoadGlobal(name) => {
                // A name bound to nothing is an error, not `nil`. Reading it used to yield
                // `nil`, so a typo propagated silently even though *assigning* to an
                // undeclared local is a compile error. A compile-time check is impossible
                // here — `use` executes at run time, so a unit cannot see the globals its
                // own `use` will define — but by the time this instruction runs, every
                // `use` has run and every class is defined. Ask whether a class exists with
                // `Class.exists?:#Name`.
                let Some(val) = self.globals.borrow().get(name).copied() else {
                    return Err(QuoinError::NameError(format!(
                        "undefined name `{name}` — nothing with that name is in scope"
                    )));
                };
                self.push(val);
                ip += 1;
            }
            Instruction::StoreGlobal(name, is_define) => {
                let val = self.pop()?;
                if name.name == "true" || name.name == "false" || name.name == "nil" {
                    let err_msg = format!("Can't modify reserved identifier {}", name.name);
                    self.exceptions.active = Some(self.new_string(mc, err_msg.clone()));
                    return Err(QuoinError::Other(err_msg));
                }
                let first_char = name.name.chars().next().unwrap_or('\0');
                if first_char.is_ascii_uppercase() {
                    let exists = self.globals.borrow().contains_key(name);
                    if *is_define {
                        if exists {
                            let err_msg = format!(
                                "Global {} is already defined in this scope",
                                name.to_explicit_string()
                            );
                            self.exceptions.active = Some(self.new_string(mc, err_msg.clone()));
                            return Err(QuoinError::Other(err_msg));
                        }
                    } else {
                        if exists {
                            let err_msg = format!(
                                "Can't modify global constant {}",
                                name.to_explicit_string()
                            );
                            self.exceptions.active = Some(self.new_string(mc, err_msg.clone()));
                            return Err(QuoinError::Other(err_msg));
                        }
                    }
                }
                self.globals.borrow_mut(mc).insert(name.clone(), val);
                ip += 1;
            }
            Instruction::Push(constant) => {
                let val = self.materialize_constant(mc, constant);
                self.push(val);
                ip += 1;
            }
            Instruction::Pop => {
                self.pop()?;
                ip += 1;
            }
            Instruction::Dup => {
                let val = self.peek()?;
                self.push(val);
                ip += 1;
            }
            // Devirtualized Integer operators (Slice 2a/2f). Fast path when both operands are
            // `Value::Int`: compute directly and push. Semantics match Integer's native ops
            // (`+`/`-`/`*` plain i64, wrap in release; `/`/`%` raise "Division by zero";
            // compares yield a Bool). A non-Int operand (a var whose inferred `Int` type went
            // stale, or an untyped operand) falls back to the real send — so `Int` can be
            // inferred optimistically rather than only trusted for annotated params.
            // The standalone `Int` ops (stack operands — e.g. `1 + 2`). All compute through the
            // shared `int_bin_compute` → `devirt_ops::int_bin`, so they can't drift from the fused
            // ops or the native `Integer` methods. A non-Int operand falls back to the real send.
            Instruction::IntAdd => {
                if let Some((a, b)) = self.take_two_ints() {
                    self.push(Self::int_bin_compute(IntBinKind::Add, a, b)?);
                    ip += 1;
                } else {
                    return self.exec_send(mc, frame_idx, Symbol::intern("+:"), 1);
                }
            }
            Instruction::IntSub => {
                if let Some((a, b)) = self.take_two_ints() {
                    self.push(Self::int_bin_compute(IntBinKind::Sub, a, b)?);
                    ip += 1;
                } else {
                    return self.exec_send(mc, frame_idx, Symbol::intern("-:"), 1);
                }
            }
            Instruction::IntMul => {
                if let Some((a, b)) = self.take_two_ints() {
                    self.push(Self::int_bin_compute(IntBinKind::Mul, a, b)?);
                    ip += 1;
                } else {
                    return self.exec_send(mc, frame_idx, Symbol::intern("*:"), 1);
                }
            }
            Instruction::IntDiv => {
                if let Some((a, b)) = self.take_two_ints() {
                    self.push(Self::int_bin_compute(IntBinKind::Div, a, b)?);
                    ip += 1;
                } else {
                    return self.exec_send(mc, frame_idx, Symbol::intern("/:"), 1);
                }
            }
            Instruction::IntMod => {
                if let Some((a, b)) = self.take_two_ints() {
                    self.push(Self::int_bin_compute(IntBinKind::Mod, a, b)?);
                    ip += 1;
                } else {
                    return self.exec_send(mc, frame_idx, Symbol::intern("%:"), 1);
                }
            }
            Instruction::IntLt => {
                if let Some((a, b)) = self.take_two_ints() {
                    self.push(Self::int_bin_compute(IntBinKind::Lt, a, b)?);
                    ip += 1;
                } else {
                    return self.exec_send(mc, frame_idx, Symbol::intern("<:"), 1);
                }
            }
            Instruction::IntLe => {
                if let Some((a, b)) = self.take_two_ints() {
                    self.push(Self::int_bin_compute(IntBinKind::Le, a, b)?);
                    ip += 1;
                } else {
                    return self.exec_send(mc, frame_idx, Symbol::intern("<=:"), 1);
                }
            }
            Instruction::IntGt => {
                if let Some((a, b)) = self.take_two_ints() {
                    self.push(Self::int_bin_compute(IntBinKind::Gt, a, b)?);
                    ip += 1;
                } else {
                    return self.exec_send(mc, frame_idx, Symbol::intern(">:"), 1);
                }
            }
            Instruction::IntGe => {
                if let Some((a, b)) = self.take_two_ints() {
                    self.push(Self::int_bin_compute(IntBinKind::Ge, a, b)?);
                    ip += 1;
                } else {
                    return self.exec_send(mc, frame_idx, Symbol::intern(">=:"), 1);
                }
            }
            Instruction::IntEq => {
                if let Some((a, b)) = self.take_two_ints() {
                    self.push(Self::int_bin_compute(IntBinKind::Eq, a, b)?);
                    ip += 1;
                } else {
                    return self.exec_send(mc, frame_idx, Symbol::intern("==:"), 1);
                }
            }
            Instruction::IntNe => {
                if let Some((a, b)) = self.take_two_ints() {
                    self.push(Self::int_bin_compute(IntBinKind::Ne, a, b)?);
                    ip += 1;
                } else {
                    return self.exec_send(mc, frame_idx, Symbol::intern("!=:"), 1);
                }
            }
            // Fused Int superinstructions (Slice a1): load the operand(s) directly and compute;
            // on a non-Int operand push the operands and fall back to the real send (matching
            // the standalone `Int` ops' contract, so MNU / user redefinition still work).
            Instruction::IntBinLL(a, b, kind) => {
                let (a, b, kind) = (*a, *b, *kind);
                let (va, vb) = {
                    let frame = &self.frames[frame_idx];
                    (EnvFrame::get(frame.env, a), EnvFrame::get(frame.env, b))
                };
                if let (Some(Value::Int(x)), Some(Value::Int(y))) = (va, vb) {
                    let res = Self::int_bin_compute(kind, x, y)?;
                    self.push(res);
                    ip += 1;
                } else {
                    let va = va.unwrap_or_else(|| self.new_nil(mc));
                    let vb = vb.unwrap_or_else(|| self.new_nil(mc));
                    self.push(va);
                    self.push(vb);
                    return self.exec_send(mc, frame_idx, Symbol::intern(kind.selector()), 1);
                }
            }
            Instruction::IntBinLC(a, c, kind) => {
                let (a, kind) = (*a, *kind);
                let va = {
                    let frame = &self.frames[frame_idx];
                    EnvFrame::get(frame.env, a)
                };
                if let (Some(Value::Int(x)), Some(y)) = (va, c.as_int()) {
                    let res = Self::int_bin_compute(kind, x, y)?;
                    self.push(res);
                    ip += 1;
                } else {
                    let va = va.unwrap_or_else(|| self.new_nil(mc));
                    self.push(va);
                    let cv = self.materialize_constant(mc, c);
                    self.push(cv);
                    return self.exec_send(mc, frame_idx, Symbol::intern(kind.selector()), 1);
                }
            }
            // Devirtualized Double operators (mirror of the Integer arms). Plain IEEE-754 f64 —
            // `/`/`%` do NOT check for zero (inf/NaN, matching native Double). A non-Double
            // operand falls back to the real send.
            Instruction::DoubleAdd => {
                if let Some((a, b)) = self.take_two_doubles() {
                    self.push(Self::double_bin_compute(IntBinKind::Add, a, b));
                    ip += 1;
                } else {
                    return self.exec_send(mc, frame_idx, Symbol::intern("+:"), 1);
                }
            }
            Instruction::DoubleSub => {
                if let Some((a, b)) = self.take_two_doubles() {
                    self.push(Self::double_bin_compute(IntBinKind::Sub, a, b));
                    ip += 1;
                } else {
                    return self.exec_send(mc, frame_idx, Symbol::intern("-:"), 1);
                }
            }
            Instruction::DoubleMul => {
                if let Some((a, b)) = self.take_two_doubles() {
                    self.push(Self::double_bin_compute(IntBinKind::Mul, a, b));
                    ip += 1;
                } else {
                    return self.exec_send(mc, frame_idx, Symbol::intern("*:"), 1);
                }
            }
            Instruction::DoubleDiv => {
                if let Some((a, b)) = self.take_two_doubles() {
                    self.push(Self::double_bin_compute(IntBinKind::Div, a, b));
                    ip += 1;
                } else {
                    return self.exec_send(mc, frame_idx, Symbol::intern("/:"), 1);
                }
            }
            Instruction::DoubleMod => {
                if let Some((a, b)) = self.take_two_doubles() {
                    self.push(Self::double_bin_compute(IntBinKind::Mod, a, b));
                    ip += 1;
                } else {
                    return self.exec_send(mc, frame_idx, Symbol::intern("%:"), 1);
                }
            }
            Instruction::DoubleLt => {
                if let Some((a, b)) = self.take_two_doubles() {
                    self.push(Self::double_bin_compute(IntBinKind::Lt, a, b));
                    ip += 1;
                } else {
                    return self.exec_send(mc, frame_idx, Symbol::intern("<:"), 1);
                }
            }
            Instruction::DoubleLe => {
                if let Some((a, b)) = self.take_two_doubles() {
                    self.push(Self::double_bin_compute(IntBinKind::Le, a, b));
                    ip += 1;
                } else {
                    return self.exec_send(mc, frame_idx, Symbol::intern("<=:"), 1);
                }
            }
            Instruction::DoubleGt => {
                if let Some((a, b)) = self.take_two_doubles() {
                    self.push(Self::double_bin_compute(IntBinKind::Gt, a, b));
                    ip += 1;
                } else {
                    return self.exec_send(mc, frame_idx, Symbol::intern(">:"), 1);
                }
            }
            Instruction::DoubleGe => {
                if let Some((a, b)) = self.take_two_doubles() {
                    self.push(Self::double_bin_compute(IntBinKind::Ge, a, b));
                    ip += 1;
                } else {
                    return self.exec_send(mc, frame_idx, Symbol::intern(">=:"), 1);
                }
            }
            Instruction::DoubleEq => {
                if let Some((a, b)) = self.take_two_doubles() {
                    self.push(Self::double_bin_compute(IntBinKind::Eq, a, b));
                    ip += 1;
                } else {
                    return self.exec_send(mc, frame_idx, Symbol::intern("==:"), 1);
                }
            }
            Instruction::DoubleNe => {
                if let Some((a, b)) = self.take_two_doubles() {
                    self.push(Self::double_bin_compute(IntBinKind::Ne, a, b));
                    ip += 1;
                } else {
                    return self.exec_send(mc, frame_idx, Symbol::intern("!=:"), 1);
                }
            }
            Instruction::DoubleBinLL(a, b, kind) => {
                let (a, b, kind) = (*a, *b, *kind);
                let (va, vb) = {
                    let frame = &self.frames[frame_idx];
                    (EnvFrame::get(frame.env, a), EnvFrame::get(frame.env, b))
                };
                if let (Some(Value::Double(x)), Some(Value::Double(y))) = (va, vb) {
                    self.push(Self::double_bin_compute(kind, x, y));
                    ip += 1;
                } else {
                    let va = va.unwrap_or_else(|| self.new_nil(mc));
                    let vb = vb.unwrap_or_else(|| self.new_nil(mc));
                    self.push(va);
                    self.push(vb);
                    return self.exec_send(mc, frame_idx, Symbol::intern(kind.selector()), 1);
                }
            }
            Instruction::DoubleBinLC(a, c, kind) => {
                let (a, kind) = (*a, *kind);
                let va = {
                    let frame = &self.frames[frame_idx];
                    EnvFrame::get(frame.env, a)
                };
                if let (Some(Value::Double(x)), Some(y)) = (va, c.as_double()) {
                    self.push(Self::double_bin_compute(kind, x, y));
                    ip += 1;
                } else {
                    let va = va.unwrap_or_else(|| self.new_nil(mc));
                    self.push(va);
                    let cv = self.materialize_constant(mc, c);
                    self.push(cv);
                    return self.exec_send(mc, frame_idx, Symbol::intern(kind.selector()), 1);
                }
            }
            // Devirtualized List accessors (Slice 2e). Operands are already on the stack in
            // send order; if the receiver isn't a native list (or the index isn't an
            // Integer, matching the typed native `at:`/`at:put:`), fall back to the real send.
            Instruction::ListGet => {
                let n = self.stack.len();
                let index = self.stack[n - 1];
                let receiver = self.stack[n - 2];
                if let Value::Int(i) = index {
                    let got = receiver.with_native_state::<NativeListState, _, _>(|l| {
                        devirt_ops::list_get(l.get_vec(), i)
                    });
                    if let Ok(elem) = got {
                        self.stack.truncate(n - 2);
                        self.push(elem.unwrap_or(Value::Nil));
                        // b2: early-return arm — sync the hoisted ip (see dispatch_one invariant).
                        self.frames[frame_idx].ip = ip + 1;
                        return Ok(VmStatus::Running);
                    }
                }
                return self.exec_send(mc, frame_idx, Symbol::intern("at:"), 1);
            }
            Instruction::TagCollection(tag) => {
                let v = *self.stack.last().expect("TagCollection: literal on stack");
                self.tag_fresh_collection(mc, v, *tag)?;
                ip += 1;
            }
            Instruction::ListSet => {
                let n = self.stack.len();
                let value = self.stack[n - 1];
                let index = self.stack[n - 2];
                let receiver = self.stack[n - 3];
                if let Value::Int(i) = index {
                    // Untagged (the whole pre-generics world): exactly the old
                    // body behind one `is_none`. Tagged lists take the
                    // out-of-line checked path (docs/GENERICS_ARCH.md §6).
                    let res = receiver.with_native_state_mut::<NativeListState, _, _>(mc, |l| {
                        match l.elem {
                            None => Some(devirt_ops::list_set(l.get_vec_mut(), i, value)),
                            // Scalar tags decide inside the one borrow; the tag
                            // check precedes the bounds check (the VALUE is
                            // illegal regardless of index). Class tags escalate.
                            Some(t) => match t.matches_value(&value) {
                                Some(true) => Some(devirt_ops::list_set(l.get_vec_mut(), i, value)),
                                Some(false) => {
                                    Some(Err(elem_tag::elem_type_error("List", t, &value, Some(i))))
                                }
                                None => None,
                            },
                        }
                    });
                    if let Ok(fast) = res {
                        let inner = match fast {
                            Some(inner) => inner,
                            None => {
                                let r = self.tagged_list_set(mc, receiver, i, value);
                                self.stack.truncate(n - 3);
                                r?;
                                self.push(receiver);
                                self.frames[frame_idx].ip = ip + 1;
                                return Ok(VmStatus::Running);
                            }
                        };
                        self.stack.truncate(n - 3);
                        inner?; // propagate an IndexError or tag TypeError
                        self.push(receiver); // `at:put:` evaluates to the receiver
                        // b2: early-return arm — sync the hoisted ip (see dispatch_one invariant).
                        self.frames[frame_idx].ip = ip + 1;
                        return Ok(VmStatus::Running);
                    }
                }
                return self.exec_send(mc, frame_idx, Symbol::intern("at:put:"), 2);
            }
            Instruction::ListPush => {
                let n = self.stack.len();
                let value = self.stack[n - 1];
                let receiver = self.stack[n - 2];
                let res = receiver.with_native_state_mut::<NativeListState, _, _>(mc, |l| {
                    match l.elem {
                        None => {
                            l.get_vec_mut().push(value);
                            Some(Ok(()))
                        }
                        // Scalar tags decide inside the one borrow (vm-free);
                        // only a Class tag escalates to the dispatch walk.
                        Some(t) => match t.matches_value(&value) {
                            Some(true) => {
                                l.get_vec_mut().push(value);
                                Some(Ok(()))
                            }
                            Some(false) => {
                                Some(Err(elem_tag::elem_type_error("List", t, &value, None)))
                            }
                            None => None,
                        },
                    }
                });
                if let Ok(fast) = res {
                    match fast {
                        Some(inner) => {
                            self.stack.truncate(n - 2);
                            inner?;
                        }
                        None => {
                            let r = self.tagged_list_push(mc, receiver, value);
                            self.stack.truncate(n - 2);
                            r?;
                        }
                    }
                    self.push(receiver); // `add:` evaluates to the receiver
                    self.frames[frame_idx].ip = ip + 1;
                    return Ok(VmStatus::Running);
                }
                return self.exec_send(mc, frame_idx, Symbol::intern("add:"), 1);
            }
            // Devirtualized Map accessors (mirror of List). Map is `IndexMap<String, Value>`, so
            // the key must be a String at runtime; a non-String key (or non-Map receiver) falls
            // back to the real send.
            Instruction::MapGet => {
                let n = self.stack.len();
                let key = self.stack[n - 1];
                let receiver = self.stack[n - 2];
                // Inline fast path for ANY scalar-exact key (String, Int,
                // Double, Symbol, …): hash in Rust, no guest dispatch
                // possible. Instance keys (guest hash/==:) fall back to the
                // real `at:` send, which handles dispatch and parking.
                if let Ok(Some(hit)) =
                    receiver.with_native_state::<NativeMapState, _, _>(|m| m.get_scalar(&key))
                {
                    self.stack.truncate(n - 2);
                    self.push(hit.unwrap_or(Value::Nil)); // missing key → nil (native `at:`)
                    self.frames[frame_idx].ip = ip + 1;
                    return Ok(VmStatus::Running);
                }
                return self.exec_send(mc, frame_idx, Symbol::intern("at:"), 1);
            }
            Instruction::MapSet => {
                let n = self.stack.len();
                let value = self.stack[n - 1];
                let key = self.stack[n - 2];
                let receiver = self.stack[n - 3];
                // Same widening as MapGet: any scalar-exact key inlines;
                // instance keys — and tag checks that need the full
                // type-matcher — fall back to the real `at:put:` send.
                if crate::value::key_native_exact(&key)
                    && crate::value::value_hash_scalar(&key).is_some()
                {
                    let res =
                        receiver.with_native_state_mut::<NativeMapState, _, _>(mc, |m| {
                            match m.elem {
                                None => {
                                    m.insert_scalar(key, value);
                                    Some(Ok(()))
                                }
                                Some(t) => match t.matches_value(&value) {
                                    Some(true) => {
                                        m.insert_scalar(key, value);
                                        Some(Ok(()))
                                    }
                                    Some(false) => Some(Err(elem_tag::elem_type_error(
                                        "Map String",
                                        t,
                                        &value,
                                        None,
                                    ))),
                                    None => None,
                                },
                            }
                        });
                    if let Ok(Some(inner)) = res {
                        self.stack.truncate(n - 3);
                        inner?;
                        self.push(receiver); // `at:put:` evaluates to the receiver
                        self.frames[frame_idx].ip = ip + 1;
                        return Ok(VmStatus::Running);
                    }
                }
                return self.exec_send(mc, frame_idx, Symbol::intern("at:put:"), 2);
            }
            Instruction::Send(selector, num_args) => {
                let (selector, num_args) = (*selector, *num_args);
                return self.exec_send(mc, frame_idx, selector, num_args);
            }
            // Fused superinstructions (see `Instruction::SendLocal` doc): push the last
            // operand the send consumes, then run the identical send path.
            Instruction::SendLocal(var, selector, num_args) => {
                let (var, selector, num_args) = (*var, *selector, *num_args);
                let frame = &self.frames[frame_idx];
                let val = EnvFrame::get(frame.env, var).unwrap_or_else(|| self.new_nil(mc));
                self.push(val);
                return self.exec_send(mc, frame_idx, selector, num_args);
            }
            Instruction::SendConst(constant, selector, num_args) => {
                let (selector, num_args) = (*selector, *num_args);
                let val = self.materialize_constant(mc, constant);
                self.push(val);
                return self.exec_send(mc, frame_idx, selector, num_args);
            }
            Instruction::SendField(field, selector, num_args) => {
                let (selector, num_args) = (*selector, *num_args);
                let val = self.load_field(mc, frame_idx, None, field);
                self.push(val);
                return self.exec_send(mc, frame_idx, selector, num_args);
            }
            // 3-instruction sends: push two operands (left-to-right) then dispatch.
            Instruction::SendLocalLocal(a, b, selector, num_args) => {
                let (a, b, selector, num_args) = (*a, *b, *selector, *num_args);
                let env = self.frames[frame_idx].env;
                let va = EnvFrame::get(env, a).unwrap_or_else(|| self.new_nil(mc));
                self.push(va);
                let vb = EnvFrame::get(env, b).unwrap_or_else(|| self.new_nil(mc));
                self.push(vb);
                return self.exec_send(mc, frame_idx, selector, num_args);
            }
            Instruction::SendLocalConst(a, constant, selector, num_args) => {
                let (a, selector, num_args) = (*a, *selector, *num_args);
                let env = self.frames[frame_idx].env;
                let va = EnvFrame::get(env, a).unwrap_or_else(|| self.new_nil(mc));
                self.push(va);
                let vc = self.materialize_constant(mc, constant);
                self.push(vc);
                return self.exec_send(mc, frame_idx, selector, num_args);
            }
            Instruction::Return | Instruction::BlockReturn => {
                if !self.frames[frame_idx].defers.is_empty() {
                    self.run_frame_defers(mc, frame_idx)?;
                }
                let mut ret_val = self.pop()?;
                let popped_frame = self.frames.pop().unwrap();
                // Open-coded `consume_popped_frame` (see its doc for the
                // copy-before-park contract): this is the hottest opcode in
                // the interpreter, and routing the frame through the helper
                // cost a measured ~20% on combinators (a fat Result plus a
                // real Frame move per return). Keep the two in lockstep.
                let spec_tid = popped_frame.spec_tid;
                self.last_popped_env = Some(popped_frame.env);
                self.stack.truncate(popped_frame.stack_base);
                if popped_frame.return_receiver
                    && let Some(rx) = popped_frame.receiver
                {
                    ret_val = rx;
                }
                if let Some(obj) = popped_frame.instantiating_obj {
                    self.push(Value::Object(obj));
                    self.finalize_instantiation(mc, obj, popped_frame.env)?;
                    ret_val = self.pop()?;
                }
                if spec_tid != 0 {
                    self.spec_observe_return(spec_tid, ret_val);
                }
                self.push(ret_val);
            }
            Instruction::MethodReturn => {
                let ret_val = self.pop()?;
                let enclosing_id = self.frames[frame_idx].enclosing_method_id;

                return if let Some(target_id) = enclosing_id {
                    // The home may be a live COMPILED invocation (S5): it has
                    // no interpreter frame — its mark says where its outcall
                    // frames and slot window begin. Pop only the frames above
                    // the mark, deliver the value at the window base, and let
                    // the AOT error channel unwind the native frames
                    // (`codegen::invoke` consumes `aot.nlr_target`). A dead
                    // home matches neither a frame nor a mark (ids are never
                    // reused) and drains like an interpreted dead home.
                    let compiled_home = self
                        .aot
                        .frame_marks
                        .iter()
                        .rev()
                        .find(|m| m.id == target_id)
                        .copied();
                    let mut ret_val = ret_val;
                    let mut target_stack_base = None;
                    loop {
                        if let Some(m) = compiled_home
                            && self.frames.len() <= m.frames_len
                        {
                            target_stack_base = Some(m.stack_base);
                            self.aot.nlr_target = Some(target_id);
                            break;
                        }
                        let Some(f) = self.frames.pop() else { break };
                        let f_id = f.id;
                        let f_stack_base = f.stack_base;
                        let (rv, spec_tid) = self.consume_popped_frame(mc, f, ret_val)?;
                        ret_val = rv;
                        if f_id == target_id {
                            if spec_tid != 0 {
                                self.spec_observe_return(spec_tid, ret_val);
                            }
                            target_stack_base = Some(f_stack_base);
                            break;
                        }
                    }
                    if let Some(base) = target_stack_base {
                        self.stack.truncate(base);
                    }
                    self.push(ret_val);
                    Err(QuoinError::NonLocalReturn)
                } else {
                    Err("MethodReturn executed outside of a method context".into())
                };
            }
            Instruction::Yeet => {
                let yeeted_val = self.pop()?;
                self.frames.clear();
                return Ok(VmStatus::Yeeted(yeeted_val));
            }
            Instruction::Jump(offset) => {
                let offset = *offset;
                ip = (ip as isize + offset) as usize;
            }
            Instruction::IfJump(offset) => {
                let offset = *offset;
                let cond = self.pop()?;
                if cond.is_truthy() {
                    ip = (ip as isize + offset) as usize;
                } else {
                    ip += 1;
                }
            }
            Instruction::ElseJump(offset) => {
                let offset = *offset;
                let cond = self.pop()?;
                if !cond.is_truthy() {
                    ip = (ip as isize + offset) as usize;
                } else {
                    ip += 1;
                }
            }
            Instruction::BranchIfNotBool(offset) => {
                let offset = *offset;
                // Peek the receiver (do not pop): a non-Bool takes the cold path (the real
                // send), which needs it on the stack; a Bool falls through to the inlined
                // branch, which consumes it.
                let is_bool = matches!(self.stack.last(), Some(Value::Bool(_)));
                if is_bool {
                    ip += 1;
                } else {
                    ip = (ip as isize + offset) as usize;
                }
            }
            Instruction::RequireBool => match self.stack.last() {
                Some(Value::Bool(_)) => ip += 1,
                other => {
                    let got = other
                        .map(|v| v.class_name())
                        .unwrap_or_else(|| "Nil".to_string());
                    return Err(QuoinError::MessageNotUnderstood {
                        receiver: got,
                        selector: "whileDo: (a loop condition must be a Boolean)".to_string(),
                        args: Vec::new(),
                        candidates: Vec::new(),
                    });
                }
            },
            Instruction::BranchIfNotList(offset, block_tid) => {
                let offset = *offset;
                let block_tid = *block_tid;
                // Peek the `each:` receiver (do not pop): a native List falls through to
                // the fused index loop (which consumes it); anything else takes the cold
                // path (the real `each:` send), which needs it on the stack. One downcast
                // per each: CALL, not per element.
                let list_probe = self.stack.last().and_then(|v| {
                    v.with_native_state::<NativeListState, _, _>(|l| {
                        let v = l.get_vec();
                        (v.len(), v.first().copied())
                    })
                    .ok()
                });
                match list_probe {
                    None => ip = (ip as isize + offset) as usize,
                    Some((len, first)) => {
                        // A COMPILED argument block flips the choice: the cold path's
                        // real send reaches it per element (invoke_block), beating the
                        // interpreted splice ~2x. The guard also feeds the template's
                        // warmth (by element count) and its argument observation (the
                        // elements ARE the args), so splice-only programs tier up.
                        if let Some(tid) = block_tid
                            && crate::tuning::aot_enabled()
                            && crate::codegen::fused_site_prefers_send(self, tid, len, first)
                        {
                            ip = (ip as isize + offset) as usize;
                        } else {
                            ip += 1;
                        }
                    }
                }
            }
            Instruction::BranchIfNotPlainNew(offset) => {
                let offset = *offset;
                // Peek the `new:` receiver (do not pop): a plain-instantiable class falls
                // through to the fused field-expression path; anything else takes the
                // cold path (the real send: user meta `new:`, abstract-class error,
                // non-class MNU), which needs it on the stack.
                let receiver = *self
                    .stack
                    .last()
                    .ok_or_else(|| QuoinError::Other("Stack underflow".to_string()))?;
                let (cell, bc_len) = {
                    let frame = &self.frames[frame_idx];
                    (frame.ic, frame.block.template.bytecode.len())
                };
                if self.plain_new_check_cached(mc, Some((cell, bc_len)), ip, receiver) {
                    ip += 1;
                } else {
                    ip = (ip as isize + offset) as usize;
                }
            }
            Instruction::NewWithFields(names) => {
                let names = names.clone();
                let base = self
                    .stack
                    .len()
                    .checked_sub(names.len())
                    .ok_or_else(|| QuoinError::Other("Stack underflow".to_string()))?;
                self.exec_new_with_fields(mc, base, &names)?;
                ip += 1;
            }
            Instruction::ListLen => {
                let n = self.stack.len();
                let receiver = self.stack[n - 1];
                let got =
                    receiver.with_native_state::<NativeListState, _, _>(|l| l.get_vec().len());
                if let Ok(len) = got {
                    self.stack.truncate(n - 1);
                    self.push(Value::Int(len as i64));
                    // b2: early-return arm — sync the hoisted ip (see dispatch_one invariant).
                    self.frames[frame_idx].ip = ip + 1;
                    return Ok(VmStatus::Running);
                }
                return self.exec_send(mc, frame_idx, Symbol::intern("count"), 0);
            }
            Instruction::NewList(n) => {
                let n = *n;
                let mut elements = Vec::new();
                for _ in 0..n {
                    elements.push(self.pop()?);
                }
                elements.reverse();
                let list = self.new_list(mc, elements);
                self.push(list);
                ip += 1;
            }
            Instruction::NewMap(n) => {
                let n = *n;
                // ANY value keys. Same rooting discipline as NewSet below: an
                // instance key's hash/==: can PARK, so the pairs stay rooted
                // in place on the VM stack, the fresh map rides on top, and
                // each insert re-reads through the stack.
                {
                    let map_val = self.new_map(mc, Vec::new());
                    self.push(map_val);
                }
                let base = self.stack.len() - 1 - 2 * n;
                for i in 0..n {
                    let map_val = *self.stack.last().expect("map on top");
                    let key = self.stack[base + 2 * i];
                    let val = self.stack[base + 2 * i + 1];
                    // Duplicate keys: the later entry wins, as before.
                    crate::runtime::map::map_put_any(self, mc, map_val, key, val)?;
                }
                let map_val = self.pop()?;
                self.stack.truncate(base);
                self.push(map_val);
                ip += 1;
            }
            Instruction::NewSet(n) => {
                let n = *n;
                // Build by inserting through set_add so the literal is deduplicated
                // by `==:`, the same way `add:` enforces uniqueness at runtime.
                // A user `==:` can PARK, so nothing GC-managed may live in Rust
                // locals across the inserts: the elements stay rooted in place on
                // the VM stack and the set rides on top, re-read after each
                // insert (popping into a Vec here once left both the elements
                // and the fresh set collectible mid-dedup).
                {
                    let set_val = self.new_set(mc, Vec::new());
                    self.push(set_val);
                }
                let base = self.stack.len() - 1 - n;
                for i in 0..n {
                    let sv = *self.stack.last().expect("set literal under construction");
                    let v = self.stack[base + i];
                    self.set_add(mc, sv, v)?;
                }
                let sv = self.pop()?;
                self.stack.truncate(base);
                self.push(sv);
                ip += 1;
            }
            Instruction::NewRegex => {
                let pattern_val = self.pop()?;
                if let Value::Object(obj) = pattern_val
                    && let ObjectPayload::String(s) = &obj.borrow().payload
                {
                    let re = Regex::new(&**s).map_err(|e| format!("Invalid regex: {}", e))?;
                    let regex_val = self.new_regex(mc, re);
                    self.push(regex_val);
                } else {
                    return Err(QuoinError::TypeError {
                        expected: "String".to_string(),
                        got: pattern_val.type_name().to_string(),
                        msg: format!("Regex pattern must be a String, got: {:?}", pattern_val),
                    });
                }
                ip += 1;
            }
            Instruction::RecordClassSite { name, source } => {
                self.class_meta
                    .entry(name.clone())
                    .or_default()
                    .extensions
                    .push(source.clone());
                ip += 1;
            }
            Instruction::DefineClass {
                name,
                parent_name,
                instance_vars,
                source,
            } => {
                // Definition wins over any earlier record (a REPL redefinition moves the
                // class); a native class's `.class_doc(..)` set at registration survives.
                if source.is_some() {
                    self.class_meta.entry(name.clone()).or_default().source = source.clone();
                }
                let parent = if let Some(p_name) = parent_name {
                    let val = self
                        .globals
                        .borrow()
                        .get(p_name)
                        .copied()
                        .ok_or_else(|| format!("Parent class {} not found", p_name))?;
                    if let Value::Class(parent_class) = val {
                        if parent_class.borrow().is_sealed {
                            // A typed ClassError, matching the sealed-EXTENSION error above
                            // (ensure_extensible): `catch:{|e:Error|}` must see both. It was
                            // a bare String throw — the F12 family (RELEASE_PREP Tier 4b).
                            return Err(QuoinError::ClassError(format!(
                                "Cannot subclass sealed class {}",
                                parent_class.borrow().name.to_explicit_string()
                            )));
                        }
                        Some(parent_class)
                    } else {
                        return Err(format!("Parent {} is not a Class", p_name).into());
                    }
                } else {
                    if !(name.path.is_empty() && name.name == "Object") {
                        let obj_key = NamespacedName::new(Vec::new(), "Object".to_string());
                        if let Some(Value::Class(obj_class)) =
                            self.globals.borrow().get(&obj_key).copied()
                        {
                            Some(obj_class)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };

                if let Some(existing_val) = self.globals.borrow().get(name).copied() {
                    if let Value::Class(_) = existing_val {
                        return Err(format!(
                            "Cannot redefine class {} because it already exists",
                            name.to_explicit_string()
                        )
                        .into());
                    }
                }

                let class_obj = gcl!(
                    mc,
                    Class {
                        name: name.clone(),
                        parent,
                        instance_vars: instance_vars.clone(),
                        instance_methods: FxHashMap::default(),
                        class_methods: FxHashMap::default(),
                        mixin_classes: Vec::new(),
                        field_slots: FxHashMap::default(),
                        init_plan: None,
                        is_eigenclass: false,
                        is_sealed: false,
                        is_abstract: false,
                        native_new_refusal: None,
                    }
                );
                self.globals
                    .borrow_mut(mc)
                    .insert(name.clone(), Value::Class(class_obj));
                // The class is registered now (so it can reference itself), but if
                // the body's deferred mixin checks fail it must be unregistered.
                // Hand the name to the upcoming ExecuteBlockWithSelf (the body).
                self.pending_class_def = Some(name.clone());
                self.push(Value::Class(class_obj));
                ip += 1;
            }
            Instruction::ExecuteBlockWithSelf => {
                let block_val = self.pop()?;
                let self_val = self.pop()?;
                if self_val.is_nil() {
                    return Err(QuoinError::Other(
                        "Cannot extend nil or non-existent class/object".to_string(),
                    ));
                }
                return if let Value::Object(obj) = block_val
                    && let ObjectPayload::Block(block) = &obj.borrow().payload
                {
                    self.frames[frame_idx].ip = ip + 1;
                    self.start_block_as_method(mc, *block, self_val, Vec::new(), None, false);
                    // A new class definition (DefineClass ran just before) marks its
                    // body frame so a failed deferred mixin check unregisters the class.
                    // Extensions don't set pending_class_def, so they get no marker.
                    let pending = self.pending_class_def.take();
                    let body_frame = self.frames.last_mut().unwrap();
                    body_frame.return_receiver = true;
                    body_frame.unregister_on_defer_failure = pending;
                    Ok(VmStatus::Running)
                } else {
                    Err(QuoinError::TypeError {
                        expected: "Block".to_string(),
                        got: block_val.type_name().to_string(),
                        msg: format!("ExecuteBlockWithSelf expects a Block, got {:?}", block_val),
                    })
                };
            }
            Instruction::DefineMethod(selector) => {
                let block_val = self.pop()?;
                if let Value::Object(obj) = block_val
                    && let ObjectPayload::Block(_) = &obj.borrow().payload
                {
                    let self_val = EnvFrame::get(self.frames[frame_idx].env, self_symbol())
                        .unwrap_or_else(|| self.new_nil(mc));
                    let target_class = self
                        .get_target_class_for_def(mc, self_val)
                        .map_err(|e| QuoinError::Other(e))?;
                    self.ensure_not_sealed(target_class)?;

                    let method_obj = self.new_method(mc, selector.clone(), block_val, false);
                    let sel_sym = Symbol::intern(selector);
                    let is_class_side = matches!(self_val, Value::ClassMeta(_));
                    if is_class_side {
                        if let Some(existing_val) =
                            target_class.borrow().class_methods.get(&sel_sym).copied()
                        {
                            self.replace_or_append_method_in_chain(mc, existing_val, method_obj)?;
                        } else {
                            target_class
                                .borrow_mut(mc)
                                .class_methods
                                .insert(sel_sym, method_obj);
                        }
                    } else {
                        if let Some(existing_val) = target_class
                            .borrow()
                            .instance_methods
                            .get(&sel_sym)
                            .copied()
                        {
                            self.replace_or_append_method_in_chain(mc, existing_val, method_obj)?;
                        } else {
                            target_class
                                .borrow_mut(mc)
                                .instance_methods
                                .insert(sel_sym, method_obj);
                        }
                    }
                    // The class's method table just changed — drop memoized resolutions
                    // and invalidate compiled direct-self recursion (S2).
                    self.invalidate_method_cache();
                    crate::codegen::bump_redef_epoch();
                    self.push(method_obj);
                    ip += 1;
                } else {
                    return Err(QuoinError::TypeError {
                        expected: "Block".to_string(),
                        got: block_val.type_name().to_string(),
                        msg: format!("DefineMethod expects a Block, got {:?}", block_val),
                    });
                }
            }
            Instruction::OverrideMethod(selector) => {
                let block_val = self.pop()?;
                if let Value::Object(obj) = block_val
                    && let ObjectPayload::Block(_) = &obj.borrow().payload
                {
                    let self_val = EnvFrame::get(self.frames[frame_idx].env, self_symbol())
                        .unwrap_or_else(|| self.new_nil(mc));
                    let target_class = self
                        .get_target_class_for_def(mc, self_val)
                        .map_err(|e| QuoinError::Other(e))?;
                    self.ensure_not_sealed(target_class)?;

                    let method_obj = self.new_method(mc, selector.clone(), block_val, true);
                    let is_class_side = matches!(self_val, Value::ClassMeta(_));
                    let exists = self
                        .lookup_in_class_hierarchy(target_class, selector, is_class_side)
                        .is_some();
                    if !exists {
                        return Err(QuoinError::Other(format!(
                            "Method {} does not exist in hierarchy of Class {} to override",
                            selector,
                            target_class.borrow().name
                        )));
                    }

                    let sel_sym = Symbol::intern(selector);
                    if is_class_side {
                        if let Some(existing_val) =
                            target_class.borrow().class_methods.get(&sel_sym).copied()
                        {
                            self.replace_or_append_method_in_chain(mc, existing_val, method_obj)?;
                        } else {
                            target_class
                                .borrow_mut(mc)
                                .class_methods
                                .insert(sel_sym, method_obj);
                        }
                    } else {
                        if let Some(existing_val) = target_class
                            .borrow()
                            .instance_methods
                            .get(&sel_sym)
                            .copied()
                        {
                            self.replace_or_append_method_in_chain(mc, existing_val, method_obj)?;
                        } else {
                            target_class
                                .borrow_mut(mc)
                                .instance_methods
                                .insert(sel_sym, method_obj);
                        }
                    }
                    // The class's method table just changed — drop memoized resolutions
                    // and invalidate compiled direct-self recursion (S2).
                    self.invalidate_method_cache();
                    crate::codegen::bump_redef_epoch();
                    self.push(method_obj);
                    ip += 1;
                } else {
                    return Err(QuoinError::TypeError {
                        expected: "Block".to_string(),
                        got: block_val.type_name().to_string(),
                        msg: format!("OverrideMethod expects a Block, got {:?}", block_val),
                    });
                }
            }

            Instruction::LoadField(name) => {
                let val = self.load_field(mc, frame_idx, Some(ip), name);
                self.push(val);
                ip += 1;
            }
            // Phase 5·3: read a field off the object on top of the stack (an inlined `v.x`).
            Instruction::LoadFieldOf(name) => {
                let obj = self.pop()?;
                let block = self.frames[frame_idx].block;
                let ic = self.frames[frame_idx].ic;
                let val = self.field_of(mc, block, ic, Some(ip), obj, name);
                self.push(val);
                ip += 1;
            }
            Instruction::StoreField(name) => {
                let val = self.pop()?;
                self.store_field_value(mc, frame_idx, ip, name, val)?;
                ip += 1;
            }
            Instruction::StoreFieldKeep(name) => {
                let val = self.peek()?;
                self.store_field_value(mc, frame_idx, ip, name, val)?;
                ip += 1;
            }
            Instruction::Use {
                package,
                path,
                glob,
            } => {
                // Clone out so the `inst` borrow is released before `load_unit` takes
                // `&mut self`. Advance ip first: `load_unit` runs the loaded unit in a
                // nested frame (frame-balanced), so this frame resumes at the next ip.
                let package = package.clone();
                let path = path.clone();
                let glob = *glob;
                ip += 1;
                if glob {
                    load_glob(self, mc, package.as_deref(), &path)?;
                } else {
                    load_unit(self, mc, package.as_deref(), &path)?;
                }
                // A `use` evaluates to nil — push one value so the statement nets +1 on
                // the stack (`compile_program` pops between statements).
                let nil = self.new_nil(mc);
                self.push(nil);
            }
        }

        // Sync the hoisted `ip` (Slice b2) back to the current frame on fall-through. Guarded
        // so a pop-arm (a non-local return that shrank the frame stack) doesn't index a popped
        // frame; early-returning arms that advanced `ip` sync it themselves (see the invariant).
        if frame_idx < self.frames.len() {
            self.frames[frame_idx].ip = ip;
        }
        Ok(VmStatus::Running)
    }
}

#[cfg(test)]
#[path = "vm_tests.rs"]
mod tests;
