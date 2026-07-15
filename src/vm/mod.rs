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
/// docs/internal/OUTCALL_ARCH.md): the same epoch + receiver/arg type-shape guards as
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
    /// Fuel/depth counters (docs/internal/AOT_ARCH.md §5): compiled code decrements
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

impl<'gc> Default for BuiltinCache<'gc> {
    fn default() -> Self {
        Self::new()
    }
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
    pub console_height: Option<u16>,
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

// The scheduler / task / guest-fiber subsystem lives in `scheduler.rs` (still
// intrinsically VM state); its public types are re-exported here so callers that
// `use crate::vm::{Task, Wake, ...}` are unaffected by the move.
mod alloc;
mod call;
mod class;
mod error;
mod exec;
pub mod scheduler;
pub use scheduler::{GatherState, Scheduler, Task, TaskId, Wake};

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
    /// The packages of the units currently executing their top level, innermost last —
    /// pushed/popped around each `load_unit` execution. `use self:` resolves against the top
    /// entry: inside a package's unit, `self:` addresses that package's own units (a named
    /// package never wants a file from its *caller*); at top level — an empty stack — it
    /// keeps meaning the entry script's root.
    #[collect(require_static)]
    pub load_stack: Vec<Option<String>>,
    /// The unit-cache hash chain (`runtime::unit_cache`): folds in each loaded
    /// unit's identity + source in load order, so a unit's cache key covers the
    /// whole compile context that preceded it. Advanced by `load_unit`.
    #[collect(require_static)]
    pub unit_chain: u64,
    /// Loaded extension *packages* (`Extension loadPackage:`), keyed by canonical package directory →
    /// the live `Extension` value, so a repeat `loadPackage:` of the same folder is idempotent. The
    /// installed classes also root the extension, but this is its canonical owner for the session.
    /// See `src/runtime/extension.rs` / `docs/internal/EXT_PACKAGING.md`.
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
    pub backend: crate::io_backend::DefaultBackend,
    /// fds whose QN `TcpSocket` handle has been closed or collected, awaiting a synchronous
    /// `IoBackend::close` by the driver. A non-GC queue (the handle's `Drop` can only push a plain
    /// `StreamId`); a shared `Rc` clone lives in each socket handle. See `docs/internal/ASYNC_ARCH.md`.
    pub socket_reap: std::rc::Rc<std::cell::RefCell<Vec<StreamId>>>,
    /// Extension ids whose `Extension` handle was dropped (GC'd), awaiting bulk-release of the
    /// host-value handles they held (`HandleTable::release_for_ext`). A non-GC queue mirroring
    /// `socket_reap`; a shared `Rc` clone lives in each `Extension` handle.
    pub ext_handle_reap: std::rc::Rc<std::cell::RefCell<Vec<u64>>>,
    /// Child ids whose `[OS]Process` handle was collected undetached (or explicitly killed at
    /// teardown), awaiting a synchronous `IoBackend::reap_child` (kill if running + deregister).
    /// A non-GC queue mirroring `socket_reap`; a shared `Rc` clone lives in each Process handle.
    pub child_reap: std::rc::Rc<std::cell::RefCell<Vec<u64>>>,
    /// Boundary-profiling tables, one per spawned extension peer (`ACTOR_OBJECTS.md` §7),
    /// registered at spawn and read by `VM.boundaryStats`. Entries deliberately outlive
    /// their extension — a dead peer's numbers are the post-mortem.
    pub ext_stats: std::rc::Rc<
        std::cell::RefCell<
            Vec<std::rc::Rc<std::cell::RefCell<crate::runtime::extension::BoundaryStats>>>,
        >,
    >,
    /// Claim state, one entry per hosted-service peer (`ACTOR_OBJECTS.md`
    /// §5.1), registered at host and read by `VM.claims` and the cross-peer
    /// deadlock walk. Entries outlive their peer — counters are the
    /// post-mortem.
    pub claim_peers: crate::runtime::claims::ClaimRegistry,
    /// Channel-relay state, one entry per worker link that this VM talks to
    /// (`ACTOR_OBJECTS.md` §6): the relay lanes, pending ops, and reap. A
    /// worker VM's entry 0 is its parent link; a parent VM gains one entry
    /// per spawned worker (registered when the handle/proxy is minted).
    pub chan_links: Vec<crate::worker::ChanLink>,
    /// Lifecycle sinks, one per spawned peer — hosted worker, plain worker,
    /// extension (SUPERVISION.md slice 1) — registered at spawn and read by
    /// `VM.peers` and the per-peer events pumps. Entries outlive their peer:
    /// the roster is also the post-mortem.
    pub lives: crate::runtime::lifecycle::LifeRegistry,
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
/// lazily, from the `"*` block above `source` (docs/internal/DOCS_ARCH.md §4); native classes carry it
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

/// One installed hosted-service class (see `VmState::service_classes`).
#[derive(Collect)]
#[collect(no_drop)]
pub struct ServiceClassEntry<'gc> {
    #[collect(require_static)]
    pub link: usize,
    #[collect(require_static)]
    pub name: String,
    pub class: Value<'gc>,
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
    /// Error channel for compiled code (docs/internal/AOT_ARCH.md v0.2): helpers store a
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
    #[allow(clippy::type_complexity)] // per-site GC-managed inline-cache slot arrays
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
    /// Set at boot on a WORKER's VM (docs/internal/CONCURRENCY_ARCH.md §5): the
    /// channel ends back to the parent. `None` on the main VM.
    #[collect(require_static)]
    pub worker_link: Option<crate::worker::WorkerLink>,
    /// WORKER-side: the conversation each serve task is currently inside
    /// (ACTOR_OBJECTS.md §3a) — how a `HostBlock` invocation finds its way
    /// back to the parent. Keyed by task so the per-object-lane world (§5.1)
    /// works unchanged; entries live only for the span of a dispatch.
    #[collect(require_static)]
    pub worker_convs: std::collections::HashMap<usize, crate::worker::ConvHandles>,
    /// WORKER-side: this VM's `chan_links` index for its PARENT link — where
    /// channels crossing plain lanes or dispatches relay (§6). `None` on the
    /// main VM and before boot registration.
    #[collect(require_static)]
    pub parent_chan_link: Option<usize>,
    /// The ENTRY unit this VM was booted to run (canonicalized), `None` for
    /// REPL/eval. "What program is this?" — `Worker.spawn:(VM.unit)` runs
    /// another copy of the current program (the same-unit provisioning
    /// model, docs/internal/WEB_ARCH.md workers). Deliberately NOT `__FILE__`: a
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
    /// `src/handle_table.rs` / `docs/internal/FUTURE_EXT_ARCH.md` §2.
    pub handle_table: crate::handle_table::HandleTable<'gc>,

    /// WORKER-side hosted-object table (`ACTOR_OBJECTS.md` §2): the objects this
    /// worker hosts for its parent, keyed by the id in `Call.recv` /
    /// `CallReturnResource.resource` (index + 1; 0 is never issued). A GC root set,
    /// like `handle_table` — a hosted object lives until the parent's proxy drop
    /// releases it (`Call.releases`) or the serve loop ends. Always empty outside
    /// `Worker.hostServe:`.
    pub hosted: Vec<Option<Value<'gc>>>,
    /// Installed hosted-service classes (ACTOR_OBJECTS.md §2 manifests), keyed
    /// by (worker link, class name) — deliberately unbound as globals; this
    /// registry is their GC root (and, through their method nodes, the
    /// services' root).
    pub service_classes: Vec<ServiceClassEntry<'gc>>,
    /// WORKER-side: the shipped block a `Worker.host:with:` / `Worker.with:`
    /// spawn carries — taken by `Worker.hostBlockRoot` once the unit (if any)
    /// has loaded, so the block's global references resolve against it.
    #[collect(require_static)]
    pub pending_host_block: Option<crate::worker::PortableBlock>,
    /// WORKER-side: hosted class names whose selector manifests this worker
    /// has already sent (the ready message or a `CallReturnResourceDecl`);
    /// later returns of the same class carry only the name.
    #[collect(require_static)]
    pub hosted_announced: std::collections::HashSet<String>,
    /// The per-peer lifecycle events Channels, indexed like `vm.io.lives`
    /// (SUPERVISION.md slice 1): the GC root and the ask-twice cache — a
    /// second `events` ask answers the SAME channel (one consumer stream per
    /// peer). `None` until asked.
    pub life_channels: Vec<Option<Value<'gc>>>,
}

pub enum VmStatus<'gc> {
    Running,
    Finished(Value<'gc>),
    Yeeted(Value<'gc>), // Uncaught exception
}

impl<'gc> VmState<'gc> {
    /// # Safety
    /// The stored yielder pointer must still be valid: only call while the
    /// coroutine that registered it (via `register_yielder`) is live on the
    /// current stack. The driver restores this slot before every resume, so
    /// it never points at a freed or different coroutine.
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
                load_stack: Vec::new(),
                unit_chain: 0,
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
            worker_convs: std::collections::HashMap::new(),
            parent_chan_link: None,
            unit_path: None,
            worker_registry: Vec::new(),
            io: Io {
                backend: crate::io_backend::DefaultBackend::new(),
                socket_reap: std::rc::Rc::new(std::cell::RefCell::new(Vec::new())),
                ext_handle_reap: std::rc::Rc::new(std::cell::RefCell::new(Vec::new())),
                child_reap: std::rc::Rc::new(std::cell::RefCell::new(Vec::new())),
                ext_stats: std::rc::Rc::new(std::cell::RefCell::new(Vec::new())),
                claim_peers: std::rc::Rc::new(std::cell::RefCell::new(Vec::new())),
                chan_links: Vec::new(),
                lives: std::rc::Rc::new(std::cell::RefCell::new(Vec::new())),
            },
            instrumentation: Instrumentation {
                debug: None,
                coverage: None,
            },
            options,
            handle_table: crate::handle_table::HandleTable::new(),
            hosted: Vec::new(),
            life_channels: Vec::new(),
            service_classes: Vec::new(),
            pending_host_block: None,
            hosted_announced: std::collections::HashSet::new(),
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

    /// Drain the captured program output (the DAP driver calls this between resumes to emit
    /// `output` events). Empty when nothing was captured.
    pub fn take_program_output(&mut self) -> Vec<OutputChunk> {
        std::mem::take(&mut self.output.chunks)
    }
}

#[cfg(test)]
mod tests;
