//! `Extension` — the Quoin-facing handle to an out-of-process native extension
//! (Tier 1; see `docs/internal/FUTURE_EXT_ARCH.md`). Slice 1 is the **transport keystone**:
//! spawn a subprocess, connect a unix domain socket, and round-trip one scalar op —
//! with the calling fiber parking on the socket fd through the existing reactor
//! (`await_io` `Write` then `Read`), so a slow extension never stalls the VM.
//!
//! This is a legacy (`&mut VmState`) native class, not an `ext_sdk` one: it is itself
//! an async/IO primitive that needs `await_io`, which lives below the SDK surface.
//!
//! Slice 3a adds the **handle table** (`docs/internal/FUTURE_EXT_ARCH.md` §2): a `call:with:` is no
//! longer a one-shot request/reply but a re-entrant *conversation*. After sending the `Call`,
//! the host services a loop of frames — each is either a host-op request the extension issued
//! mid-call (answered with `HostOpReturn`) or the terminal `CallReturn`. Handles minted during
//! the call are call-local and swept on return (`HandleTable::begin_call`/`end_call`); the
//! extension `Retain`s any it wants to keep.
//!
//! The host-ops are `MakeString`/`HandleToString`/`Retain`/`Release` (Slice 3a),
//! `CallMethodOnHandle` (Slice 3b — send a Quoin message to a handle), and `InvokeBlock`
//! (Slice 4 — invoke a host *block* handle over a batch of argument tuples in one round-trip).
//! Every frame is one MessagePack array (codec + `PROTOCOL.md` contract in `quoin-ext-proto`)
//! inside a u32 length-prefixed frame; the protocol version is checked in the manifest
//! handshake, the first exchange on a fresh connection.
//!
//! Slice 5b makes handles general `Call` arguments: `call:with:args:` passes a list whose elements
//! become either host-value handles (`Call.handles` — a block is one of these; the Slice-4 `block`
//! field is gone) or, for an `ExtResource`, the ext-side resource id (`Call.resources`). The mirror
//! direction — **ext-resource handles** — lets a call return an ext-owned resource
//! (`CallReturnResource`) which the host holds as an opaque `ExtResource` token and reaps on drop
//! (batched onto the next call's `Call.releases`, since a GC `Drop` can't send a frame).
//!
//! Slice 6b adds the **bulk data plane**: an `Array` arg routes into `Call.arrays` (copy-through, a
//! 3rd arg kind) and a call can return a bulk column via `CallReturnArray`, reconstructed host-side
//! as an `Array`. Whole columns cross the boundary as one buffer — never exploded into per-element
//! Values.
//!
//! Slice 5a adds **crash isolation**: a call whose I/O fails because the child exited surfaces a
//! typed `IoError` (not a hang), marks the extension dead so later calls fail fast, and `Drop`
//! reaps the host-side fd via the shared reap queue. A later slice adds **per-peer handle
//! bulk-release** (a dead/dropped extension's retained handles are freed via `release_for_ext`).
//!
//! **Timeouts** reuse the general `Async.timeout:do:` combinator (it aborts the parked socket
//! read and raises a catchable `TimeoutError`); the only extension-specific part is that a
//! cancelled (timed-out) call leaves the framed conversation desynced, so the extension is marked
//! dead — its connection can't be safely reused.
//!
//! **Structured values** (Phase 1): `call:with:data:` passes a Quoin value serialized to a wire
//! `DataValue` tree (`Call.data`), and a call may return one (`CallReturnData`), materialized back
//! into a nested Quoin Value. Both directions use the direct [`value_to_wire`] / [`wire_to_value`]
//! walkers (one traversal each way — the runtime `DataValue` used by the structured *formats*
//! is not involved).

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use gc_arena::collect::Trace;
use gc_arena::{Gc, lock::RefLock};

use quoin_ext_proto::{
    Arg, ArrowArray, ArrowDType, ClassDecl, DataValue as WireData, Msg, PROTOCOL_VERSION,
};

use crate::arg;
use crate::error::{PeerDeathReason, QuoinError};
use crate::io_backend::{IoRequest, IoResult, StreamId};
use crate::runtime::array::{self, ArrayDType};
use crate::runtime::big_decimal::NativeBigDecimal;
use crate::runtime::big_integer::NativeBigInteger;
use crate::runtime::claims::{PeerClaims, WaitKind};
use crate::runtime::list::NativeListState;
use crate::runtime::map::NativeMapState;
use crate::runtime::runtime::eval_string;
use crate::runtime::worker_service::{claim_peer_object, end_peer_call_and_wake};
use crate::symbol::Symbol;
use crate::value::{AnyCollect, Class, NamespacedName, NativeClassBuilder, ObjectPayload, Value};
use crate::vm::VmState;

/// Resolve a name in the host's global table to its `Value` (a class is a class-valued global).
/// The name is parsed as a `NamespacedName`, so a namespaced class such as `[ADBC]Database` resolves
/// the same way it was installed (not as a bare name). `None` if unbound. Used by the `get_global`
/// host-op (Phase 2) and to resolve a returned resource's class (Phase 3 cross-class returns).
pub(crate) fn resolve_global<'gc>(vm: &VmState<'gc>, name: &str) -> Option<Value<'gc>> {
    let key = NamespacedName::parse(name);
    vm.globals.borrow().get(&key).copied()
}

fn unrepresentable(type_name: &str) -> QuoinError {
    QuoinError::TypeError {
        expected: "a serializable value".to_string(),
        got: type_name.to_string(),
        msg: format!("cannot serialize a {type_name} (no data representation)"),
    }
}

/// Walk a Quoin value directly into the wire `DataValue` (the send side): one traversal, no
/// intermediate tree. Errors on values with no data representation (a Block, a Symbol, a user
/// instance, another native type like Duration/DateTime). Map pairs keep insertion order;
/// `BigInteger`/`BigDecimal` cross as their decimal-string form.
///
/// `owner` is the *target* extension's resource-reap queue (its identity): an extension-backed
/// instance owned by that extension crosses as a live `Resource` reference; one owned by a
/// *different* extension — or any instance when `owner` is `None` (host-op channels carry plain
/// data) — is an error, so the tree-level caller can fall back (or refuse) explicitly.
pub(crate) fn value_to_wire(
    v: Value<'_>,
    owner: Option<&Rc<RefCell<Vec<u64>>>>,
) -> Result<WireData, QuoinError> {
    match v {
        Value::Nil => Ok(WireData::Null),
        Value::Bool(b) => Ok(WireData::Bool(b)),
        Value::Int(i) => Ok(WireData::Int(i)),
        Value::Double(f) => Ok(WireData::Float(f)),
        Value::Object(obj) => {
            {
                let borrowed = obj.borrow();
                match &borrowed.payload {
                    ObjectPayload::String(s) => return Ok(WireData::Str((**s).clone())),
                    ObjectPayload::Bytes(b) => return Ok(WireData::Bytes((**b).clone())),
                    ObjectPayload::Symbol(_) => return Err(unrepresentable("Symbol")),
                    ObjectPayload::Block(_) => return Err(unrepresentable("Block")),
                    ObjectPayload::Instance => return Err(unrepresentable(&borrowed.class_name())),
                    ObjectPayload::NativeState(_) => {} // dispatched below, after dropping the borrow
                }
            }
            if let Ok(owned) = v.with_native_state::<NativeExtResource, _, _>(|r| {
                owner
                    .is_some_and(|o| Rc::ptr_eq(&r.reap, o))
                    .then_some(r.resource_id)
            }) {
                return match owned {
                    // Host -> ext, the class name is redundant (the extension resolves the id
                    // in its own object table), so it stays empty.
                    Some(id) => Ok(WireData::Resource {
                        id,
                        class_name: String::new(),
                    }),
                    None => Err(QuoinError::Other(
                        "extension: cannot send this extension-backed instance here — it \
                         belongs to a different extension or to a DEAD INCARNATION of this \
                         one (a restarted extension cannot accept the old incarnation's \
                         instances; re-create them), or this channel carries plain data only"
                            .to_string(),
                    )),
                };
            }
            if let Ok(items) =
                v.with_native_state::<NativeListState, _, _>(|l| l.get_vec().to_vec())
            {
                let items = items
                    .iter()
                    .map(|e| value_to_wire(*e, owner))
                    .collect::<Result<Vec<_>, _>>()?;
                return Ok(WireData::List(items));
            }
            if let Ok(map) = v.with_native_state::<NativeMapState, _, _>(|m| m.entries().to_vec()) {
                let mut entries = Vec::with_capacity(map.len());
                for (_, k, val) in map {
                    let Value::Object(kobj) = k else {
                        return Err(unrepresentable("Map with non-String keys"));
                    };
                    let ObjectPayload::String(ks) = &kobj.borrow().payload else {
                        return Err(unrepresentable("Map with non-String keys"));
                    };
                    entries.push(((**ks).clone(), value_to_wire(val, owner)?));
                }
                return Ok(WireData::Map(entries));
            }
            if let Ok(big) = v.with_native_state::<NativeBigInteger, _, _>(|d| d.0.to_string()) {
                return Ok(WireData::BigInt(big));
            }
            if let Ok(dec) = v.with_native_state::<NativeBigDecimal, _, _>(|d| d.0.to_string()) {
                return Ok(WireData::Decimal(dec));
            }
            Err(unrepresentable(v.type_name()))
        }
        _ => Err(unrepresentable(v.type_name())),
    }
}

/// Context for materializing `Resource` leaves in a received tree: the owning extension's
/// resource-reap queue (cloned into each wrapper, so drops release normally) and its package
/// namespace (to resolve the declared class — cross-class returns inside data). Absent where
/// resources are not accepted (host-op values — deferred).
pub(crate) struct ResCtx<'a> {
    reap: &'a Rc<RefCell<Vec<u64>>>,
    namespace: Option<&'a str>,
}

/// Build a Quoin value directly from a wire `DataValue` (the receive side): `Map` → `Map`,
/// `List` → `List`, decimal-string `BigInt`/`Decimal` parsed back to arbitrary precision, a
/// `Resource` leaf wrapped as a live extension-backed instance (when `res` allows it). The
/// nesting depth of a received tree is already capped by the decoder.
pub(crate) fn wire_to_value<'gc>(
    vm: &VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    dv: &WireData,
    res: Option<&ResCtx<'_>>,
) -> Result<Value<'gc>, QuoinError> {
    Ok(match dv {
        WireData::Null => vm.new_nil(mc),
        WireData::Bool(b) => vm.new_bool(mc, *b),
        WireData::Int(i) => vm.new_int(mc, *i),
        WireData::BigInt(s) => {
            let n = s
                .parse()
                .map_err(|_| QuoinError::Other(format!("extension: invalid BigInt {s:?}")))?;
            let class = vm.get_or_create_builtin_class(mc, "BigInteger");
            vm.new_native_state(mc, class, NativeBigInteger(n))
        }
        WireData::Float(f) => vm.new_double(mc, *f),
        WireData::Decimal(s) => {
            let d = s
                .parse()
                .map_err(|_| QuoinError::Other(format!("extension: invalid Decimal {s:?}")))?;
            let class = vm.get_or_create_builtin_class(mc, "BigDecimal");
            vm.new_native_state(mc, class, NativeBigDecimal(d))
        }
        WireData::Str(s) => vm.new_string(mc, s.clone()),
        WireData::Bytes(b) => vm.new_bytes(mc, b.clone()),
        WireData::List(items) => {
            let vals = items
                .iter()
                .map(|e| wire_to_value(vm, mc, e, res))
                .collect::<Result<Vec<_>, _>>()?;
            vm.new_list(mc, vals)
        }
        WireData::Map(entries) => {
            let mut map = Vec::with_capacity(entries.len());
            for (k, val) in entries {
                map.push((k.clone(), wire_to_value(vm, mc, val, res)?));
            }
            vm.new_map(mc, map)
        }
        WireData::Resource { id, class_name } => {
            let Some(res) = res else {
                return Err(QuoinError::Other(
                    "extension: a live extension instance cannot appear in this value".to_string(),
                ));
            };
            let class = resolve_ext_class(vm, class_name, res.namespace);
            wrap_resource(vm, mc, *id, res.reap.clone(), class)
        }
    })
}

/// Bridge the host-side `Array` dtype to the wire `ArrowDType`.
fn to_wire_dtype(d: ArrayDType) -> ArrowDType {
    match d {
        ArrayDType::Float64 => ArrowDType::Float64,
        ArrayDType::Int64 => ArrowDType::Int64,
    }
}

/// Bridge the wire `ArrowDType` back to the host-side `Array` dtype.
fn from_wire_dtype(d: ArrowDType) -> ArrayDType {
    match d {
        ArrowDType::Int64 => ArrayDType::Int64,
        // Unknown future dtypes fall back to Float64 (forward-compat; trusted peer, §4).
        _ => ArrayDType::Float64,
    }
}

/// Boundary profiling (`ACTOR_OBJECTS.md` §7): per-`(class, selector)` counters for
/// every call that crosses this peer's boundary. Always on — a few `Instant` reads and
/// one map update against a ≥10µs round-trip floor. One table per extension, registered
/// in `vm.io.ext_stats` at spawn so `VM.boundaryStats` can enumerate peers; the entry
/// outlives a dead or dropped extension — post-mortem numbers are exactly when you want
/// the report.
#[derive(Debug)]
pub struct BoundaryStats {
    /// Peer label for reporting: the package namespace, or the raw spawn command.
    pub peer: String,
    /// Rows keyed `(class, selector)`; class is `""` for the generic `call:with:` path.
    pub rows: HashMap<(String, String), BoundaryRow>,
}

/// One row of [`BoundaryStats`]. All time totals are in microseconds; a mean is
/// `total / calls`. The decomposition the report leans on:
/// `wall = transport/encode + handler`, with `claim_wait` (mailbox contention) tracked
/// separately — a call's full cost as its caller felt it is `claim_wait + wall`.
#[derive(Debug, Default, Clone, Copy)]
pub struct BoundaryRow {
    pub calls: u64,
    /// Calls that ended in an error terminal (or transport failure).
    pub errors: u64,
    pub bytes_out: u64,
    pub bytes_in: u64,
    /// In-call wall time: from opening the conversation to its terminal, nested
    /// host-op servicing included, claim-queue wait excluded.
    pub wall_micros: u64,
    /// Time parked waiting for the connection claim (another task's call in flight).
    pub claim_wait_micros: u64,
    /// The peer-reported servicing time (`ReplyMeta.handler_micros`; 0 = the peer's
    /// SDK predates the field). `wall - handler` is the transport/encode share.
    pub handler_micros: u64,
}

/// Per-call traffic/timing accumulator threaded through one [`extension_call`]
/// conversation (nested host-op frames included).
#[derive(Default)]
struct CallMeter {
    bytes_out: u64,
    bytes_in: u64,
    handler_micros: u64,
}

/// Fold one finished call into the peer's stats table.
fn record_boundary(
    stats: &RefCell<BoundaryStats>,
    class: &str,
    op: &str,
    meter: &CallMeter,
    wall_micros: u64,
    claim_wait_micros: u64,
    errored: bool,
) {
    let mut s = stats.borrow_mut();
    let row = s
        .rows
        .entry((class.to_string(), op.to_string()))
        .or_default();
    row.calls += 1;
    row.errors += u64::from(errored);
    row.bytes_out += meter.bytes_out;
    row.bytes_in += meter.bytes_in;
    row.wall_micros += wall_micros;
    row.claim_wait_micros += claim_wait_micros;
    row.handler_micros += meter.handler_micros;
}

/// Boundary fold for hosted-service calls (worker_service.rs): no byte meter —
/// thread lanes carry owned frames, so there is nothing to weigh; `handler`
/// comes from the dispatch-side stamp (`DispatchReq.handler_micros`, 0 for
/// process backing until the pumps carry `ReplyMeta`).
pub(crate) fn record_boundary_row(
    stats: &RefCell<BoundaryStats>,
    class: &str,
    op: &str,
    wall_micros: u64,
    claim_wait_micros: u64,
    handler_micros: u64,
    errored: bool,
) {
    let meter = CallMeter {
        bytes_out: 0,
        bytes_in: 0,
        handler_micros,
    };
    record_boundary(
        stats,
        class,
        op,
        &meter,
        wall_micros,
        claim_wait_micros,
        errored,
    );
}

/// Native state behind an `Extension` value: the lane sockets to the child process, its socket
/// path (for cleanup), the shared fd-reap queue, whether the extension has been observed dead,
/// the queue of ext-side resource ids dropped by the host (flushed as `Call.releases`), and the
/// §5.1 claim state that arbitrates the lanes.
#[derive(Debug)]
pub struct NativeExtension {
    /// The lane sockets, in connect order (index 0 carried the manifest handshake). Each is a
    /// request/response stream with no request ids, so a conversation pins one socket for its
    /// whole LIFO duration; the manifest's `lanes` declaration is how many exist.
    socks: Vec<StreamId>,
    /// The idle lane sockets. The lane tokens in `claims` bound concurrent conversations to
    /// `socks.len()`, so a granted top-level call always finds a socket here — the token is
    /// the right to pop one.
    free_socks: Vec<StreamId>,
    /// Open conversations by task id: the lane socket each is pinned to and its LIFO depth
    /// (1 = a plain call; >1 = re-entrant nesting — the same task calling back in from inside
    /// a block the extension is invoking; the nested call's frames ride the SAME socket,
    /// the extension servicing them while awaiting its own host-op reply). Capped by
    /// [`MAX_CALL_DEPTH`] so mutual host<->extension recursion dies loudly.
    convs: HashMap<usize, ExtConv>,
    /// §5.1 claims for this peer: per-resource mailboxes plus the lane pool (`claims.rs`, the
    /// same machinery hosted services run). Registered in `vm.io.claim_peers`, so extension
    /// claims appear in `VM.claims`/`VM.claimsReport` and the cross-peer deadlock walk.
    claims: Rc<RefCell<PeerClaims>>,
    /// Mints the pseudo-object id for each class-side/generic call: unique per call and
    /// high-bit-tagged (clear of real resource ids), so those calls contend only on lanes —
    /// parallel constructors are the point of declaring lanes (§5.1 rule 5's lane-only claim,
    /// expressed as a fresh never-contended object riding the ordinary joint acquisition).
    next_pseudo: u64,
    /// A process-unique, never-reused id for this extension; tags the host-value handles it mints
    /// so they can be bulk-released when it dies or is dropped (`HandleTable::release_for_ext`).
    ext_id: u64,
    child: Child,
    sock_path: String,
    /// Shared clone of `VmState::socket_reap`; `Drop` enqueues every lane socket so the driver
    /// closes the host-side UDS fds (the reactor can't be touched from `Drop`). Mirrors
    /// `NativeSocket`.
    reap: Rc<RefCell<Vec<StreamId>>>,
    /// Shared clone of `VmState::ext_handle_reap`; `Drop` enqueues `ext_id` so the driver
    /// bulk-releases this extension's host-value handles (a GC `Drop` can't touch the table).
    handle_reap: Rc<RefCell<Vec<u64>>>,
    /// Set once the child has been observed exited, so further calls fail fast (crash isolation).
    dead: bool,
    /// Ext-side resource ids whose host `ExtResource` was dropped, awaiting flush to the
    /// extension as `Call.releases`. Cloned into each `ExtResource` this extension hands out so
    /// its `Drop` can enqueue here (a GC `Drop` can't send a frame; mirrors the fd-reap pattern).
    resource_reap: Rc<RefCell<Vec<u64>>>,
    /// The package namespace this extension's classes were installed under (`loadPackage:`), or
    /// `None` for the raw `spawn:` escape hatch (verbatim names). The extension itself only ever
    /// speaks *simple* class names (`EXT_PACKAGING.md` §4); the host translates — stripping the
    /// namespace on outbound dispatch and re-applying it to resolve a returned instance's class
    /// (cross-class returns).
    namespace: Option<String>,
    /// Boundary-profiling counters (shared with `vm.io.ext_stats`, which outlives us).
    stats: Rc<RefCell<BoundaryStats>>,
    /// Lifecycle sink (shared with `vm.io.lives`; SUPERVISION.md slice 1):
    /// the death seams below emit through it — single source, first terminal
    /// wins, so the exit watch and the lazy detection never double-report.
    /// Swapped for a fresh sink at each `restart`.
    life: std::sync::Arc<crate::runtime::lifecycle::LifeSink>,
    /// Its index in `vm.io.lives` — what `events` hands the pump and watch.
    life_idx: usize,
    /// The frozen respawn recipe (SUPERVISION.md slice 2): the spawn command
    /// line plus the ORIGINAL manifest — the rule-9 equality gate a restarted
    /// child must match, selector for selector, lane for lane.
    recipe: Rc<ExtRecipe>,
    /// The current incarnation (bumped by a successful `restart`); stale
    /// resources name it in their error.
    incarnation: u64,
    /// The supervision policy (SUPERVISION.md slice 3), attached post-spawn
    /// via `supervise:` or the package's `quoin.toml [extension]` keys.
    policy: Option<crate::runtime::supervise::SupervisePolicy>,
    /// The policy's budget is spent: permanently dead this process — the
    /// circuit breaker. Calls fail with the typed `#gaveUp`.
    gave_up: bool,
    /// The user's restart hook (`onRestart:`) as a `vm.pins` ticket — runs
    /// inside every restart attempt, after the rebind; it outlives
    /// incarnations (rebind never touches it).
    hook: std::cell::Cell<Option<crate::pin_table::PinId>>,
}

/// An extension's frozen respawn recipe (SUPERVISION.md slice 2, rule 2):
/// exactly the spawn inputs, plus the first manifest as the equality gate.
#[derive(Debug)]
struct ExtRecipe {
    command: String,
    args: Vec<String>,
    cwd: Option<std::path::PathBuf>,
    manifest: Vec<ClassDecl>,
    lanes: u32,
}

/// One open conversation, pinned to a lane socket (keyed by task in
/// [`NativeExtension::convs`]).
#[derive(Debug)]
struct ExtConv {
    sock: StreamId,
    depth: u32,
}

/// The tag bit for per-call pseudo-object ids (class-side and generic calls), keeping them
/// clear of extension-assigned resource ids (which count up from 1).
const PSEUDO_OBJECT_BASE: u64 = 1 << 63;

/// Native state behind an `ExtResource` value: an opaque token for a resource that lives in the
/// extension process. Holds the extension-assigned id and a clone of that extension's
/// `resource_reap` queue; `Drop` enqueues the id so the next `Call` tells the extension to free it.
#[derive(Debug)]
pub struct NativeExtResource {
    resource_id: u64,
    reap: Rc<RefCell<Vec<u64>>>,
}

impl AnyCollect for NativeExtResource {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn trace_gc<'gc>(&self, _cc: &mut dyn Trace<'gc>) {}
}

impl Drop for NativeExtResource {
    fn drop(&mut self) {
        self.reap.borrow_mut().push(self.resource_id);
    }
}

impl NativeExtension {
    /// Tear the extension down *now* rather than lingering until GC drop: mark it dead and
    /// kill + reap the child and remove the socket file. Called when a call leaves the
    /// connection permanently unusable (a timeout / cancel desyncs the framed conversation),
    /// so a wedged or slow child does not keep running — holding a process slot and its
    /// socket file — until the `Extension` value is eventually collected (which may be much
    /// later, or never if the program keeps the handle). Idempotent and mirrors `Drop`; the
    /// host-side fd + handle reap still happen in `Drop`.
    fn kill_now(&mut self) {
        self.dead = true;
        self.claims.borrow_mut().gone = Some("died");
        self.life.emit_died(
            crate::error::PeerDeathReason::Exited,
            "killed after a cancelled call desynced the connection",
        );
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.sock_path);
    }

    /// If a call's I/O failed *because* the child exited, mark the extension dead and return a
    /// short description of how it exited; otherwise `None` (the failure was something else).
    /// `try_wait` is non-blocking, so this is cheap and only runs on the error path.
    fn note_if_exited(&mut self) -> Option<String> {
        match self.child.try_wait() {
            Ok(Some(status)) => {
                self.dead = true;
                self.claims.borrow_mut().gone = Some("died");
                let detail = match status.code() {
                    Some(code) => format!("exited with status {code}"),
                    None => "terminated by signal".to_string(),
                };
                self.life
                    .emit_died(crate::error::PeerDeathReason::Exited, &detail);
                Some(detail)
            }
            _ => None,
        }
    }
}

impl AnyCollect for NativeExtension {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    // Holds no GC values — nothing to trace.
    fn trace_gc<'gc>(&self, _cc: &mut dyn Trace<'gc>) {}
}

impl Drop for NativeExtension {
    fn drop(&mut self) {
        // Dropping the handle is the owner letting go — a STOP, not a death
        // (first terminal wins: the teardown kill below must not read as one).
        self.life.emit_stopped("extension handle dropped");
        // Best-effort teardown: enqueue every lane fd and this extension's handles for the
        // driver to reap, kill + reap the child, and remove the socket file.
        self.reap.borrow_mut().extend(self.socks.iter().copied());
        self.handle_reap.borrow_mut().push(self.ext_id);
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.sock_path);
    }
}

/// The typed error raised when an extension's process has died (during or before a call).
/// Surfaces to Quoin as a `PeerDiedError` (reason `#exited`) — the peer-death root class,
/// deliberately NOT an `IoError` (SUPERVISION.md §10.4): a dead isolate must not share a
/// catch clause with the I/O a program does on purpose.
fn extension_dead_error(peer: &str, detail: &str) -> QuoinError {
    QuoinError::peer_died(
        peer,
        crate::error::PeerDeathReason::Exited,
        format!("Extension process died ({detail})"),
    )
}

/// A process-unique, never-reused extension id (used to tag and bulk-release handles).
fn unique_ext_id() -> u64 {
    static COUNTER: AtomicU64 = AtomicU64::new(1);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// A short, unique unix-socket path. `/tmp` (not `temp_dir()`) keeps it well under the
/// ~104-byte `sun_path` limit on macOS, where `temp_dir()` is deep.
fn unique_sock_path() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("/tmp/quoin-ext-{}-{}.sock", std::process::id(), n)
}

/// Read up to one chunk from the extension stream, parking the fiber on the socket.
fn read_chunk<'gc>(vm: &mut VmState<'gc>, id: StreamId) -> Result<Vec<u8>, QuoinError> {
    match vm.await_io(IoRequest::Read {
        id,
        max: 4096,
        buf: Vec::new(),
    })? {
        IoResult::Read(b) => Ok(b),
        IoResult::Err(e) => Err(QuoinError::from_io_error(&e)),
        other => Err(QuoinError::Other(format!(
            "Extension: unexpected read result {other:?}"
        ))),
    }
}

/// Read exactly one length-prefixed reply frame (u32-LE length + payload), looping
/// over `Read`s (each a park point) until the whole frame has arrived.
fn read_reply_frame<'gc>(vm: &mut VmState<'gc>, id: StreamId) -> Result<Vec<u8>, QuoinError> {
    let mut buf: Vec<u8> = Vec::new();
    while buf.len() < 4 {
        let chunk = read_chunk(vm, id)?;
        if chunk.is_empty() {
            // Typed as transport I/O so `finish_outcome` can classify it as a
            // death (§2) even when the child's exit isn't reap-visible yet.
            return Err(QuoinError::io(
                crate::error::IoErrorKind::UnexpectedEof,
                "Extension call: connection closed before reply",
            ));
        }
        buf.extend_from_slice(&chunk);
    }
    let len = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
    if len > quoin_ext_proto::MAX_FRAME_LEN {
        // A corrupt/hostile length would otherwise drive unbounded accumulation. Refuse
        // before growing `buf`; the connection is desynced, so this is a hard error.
        return Err(QuoinError::Other(format!(
            "Extension call: reply frame length {len} exceeds the {} byte limit",
            quoin_ext_proto::MAX_FRAME_LEN
        )));
    }
    while buf.len() < 4 + len {
        let chunk = read_chunk(vm, id)?;
        if chunk.is_empty() {
            return Err(QuoinError::Other(
                "Extension call: truncated reply".to_string(),
            ));
        }
        buf.extend_from_slice(&chunk);
    }
    // The protocol is strict request/response (one frame in flight per direction), so a
    // read that pulled in bytes past this frame means a pipelining/desync bug — silently
    // dropping them (as the old `buf[4..4+len]` slice did) would lose the next frame and
    // mask the fault. The SDK reads with `read_exact`; hold the host to the same
    // discipline and surface the extra bytes as an error.
    if buf.len() > 4 + len {
        return Err(QuoinError::Other(format!(
            "Extension call: {} trailing byte(s) after a {len}-byte reply frame (protocol desync)",
            buf.len() - (4 + len)
        )));
    }
    Ok(buf[4..4 + len].to_vec())
}

/// Encode `msg` and write it as one length-prefixed frame, parking the fiber on the
/// socket. Answers the frame's full wire size (prefix included) for boundary profiling.
fn write_msg<'gc>(vm: &mut VmState<'gc>, id: StreamId, msg: &Msg) -> Result<u64, QuoinError> {
    let payload = quoin_ext_proto::encode(msg);
    let mut frame = (payload.len() as u32).to_le_bytes().to_vec();
    frame.extend_from_slice(&payload);
    let wire_len = frame.len() as u64;
    match vm.await_io(IoRequest::Write { id, bytes: frame })? {
        IoResult::Wrote(_) => Ok(wire_len),
        IoResult::Err(e) => Err(QuoinError::from_io_error(&e)),
        other => Err(QuoinError::Other(format!(
            "Extension: unexpected write result {other:?}"
        ))),
    }
}

/// Resolve a receiver handle and a list of argument handles to their host `Value`s
/// (each `Copy`), so the table borrow is released before a `&mut self` method send.
fn resolve_handles<'gc>(
    vm: &VmState<'gc>,
    receiver: u64,
    args: &[u64],
) -> Result<(Value<'gc>, Vec<Value<'gc>>), String> {
    let recv = vm.handle_table.get(receiver)?;
    let mut arg_vals = Vec::with_capacity(args.len());
    for &handle in args {
        arg_vals.push(vm.handle_table.get(handle)?);
    }
    Ok((recv, arg_vals))
}

/// Read the Rust string behind a host `String` value, or `None` if it isn't one.
fn read_string_value(value: Value<'_>) -> Option<String> {
    match value {
        Value::Object(obj) => match &obj.borrow().payload {
            ObjectPayload::String(s) => Some(s.as_str().to_string()),
            _ => None,
        },
        _ => None,
    }
}

/// Service one re-entrant host-op the extension issued mid-call, writing back its
/// `HostOpReturn`. Returns `Ok(())` for every host-op; the caller's loop handles `CallReturn`.
/// Answers the reply frame's wire size, so the enclosing call's meter counts the
/// nested conversation's outbound traffic too.
fn service_host_op<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    id: StreamId,
    epoch: u32,
    ext_id: u64,
    msg: Msg,
) -> Result<u64, QuoinError> {
    let reply = match msg {
        Msg::MakeString { value } => {
            let v = vm.new_string(mc, value);
            let handle = vm.handle_table.mint_local(v, epoch, ext_id);
            Msg::HostOpReturn {
                handle,
                str: None,
                error: None,
                remote_stack: String::new(),
            }
        }
        Msg::HandleToString { handle } => match vm.handle_table.get(handle) {
            Ok(v) => match read_string_value(v) {
                Some(s) => Msg::HostOpReturn {
                    handle: 0,
                    str: Some(s),
                    error: None,
                    remote_stack: String::new(),
                },
                None => host_op_error(format!("handle {handle} does not refer to a String")),
            },
            Err(e) => host_op_error(e),
        },
        Msg::Retain { handle } => match vm.handle_table.retain(handle) {
            Ok(()) => ack(),
            Err(e) => host_op_error(e),
        },
        Msg::Release { handles } => {
            vm.handle_table.release(&handles);
            ack()
        }
        Msg::CallMethodOnHandle {
            receiver,
            selector,
            args,
        } => match resolve_handles(vm, receiver, &args) {
            // Resolve all handles first (dropping the table borrow), then perform a real
            // host-side send; mint a call-local handle for the result. A Quoin-level raise
            // surfaces to the extension as a host-op error, not a failed `call:with:`.
            Ok((recv, arg_vals)) => match vm.call_method(mc, recv, &selector, arg_vals) {
                Ok(result) => {
                    let handle = vm.handle_table.mint_local(result, epoch, ext_id);
                    Msg::HostOpReturn {
                        handle,
                        str: None,
                        error: None,
                        remote_stack: String::new(),
                    }
                }
                Err(e) => host_op_failure(&format!("call '{selector}' on handle: "), &e),
            },
            Err(e) => host_op_error(e),
        },
        Msg::InvokeBlock { block, batches } => {
            match invoke_block_batches(vm, mc, epoch, ext_id, block, &batches) {
                Ok(results) => Msg::InvokeBlockReturn {
                    results,
                    error: None,
                    remote_stack: String::new(),
                },
                Err((message, segment)) => Msg::InvokeBlockReturn {
                    results: Vec::new(),
                    error: Some(message),
                    remote_stack: segment,
                },
            }
        }
        // Phase 2 — host reach.
        Msg::GetGlobal { name } => match resolve_global(vm, &name) {
            Some(value) => {
                let handle = vm.handle_table.mint_local(value, epoch, ext_id);
                Msg::HostOpReturn {
                    handle,
                    str: None,
                    error: None,
                    remote_stack: String::new(),
                }
            }
            None => host_op_error(format!("get_global: no global named '{name}'")),
        },
        // Resources-in-data stay refused on the host-op channels (`res: None`) — a
        // `make_value`/`read_handle` value is plain data (deferred; revisit with a use case).
        Msg::MakeValue { value } => match wire_to_value(vm, mc, &value, None) {
            Ok(v) => {
                let handle = vm.handle_table.mint_local(v, epoch, ext_id);
                Msg::HostOpReturn {
                    handle,
                    str: None,
                    error: None,
                    remote_stack: String::new(),
                }
            }
            Err(e) => host_op_error(format!("make_value: {e}")),
        },
        Msg::ReadHandle { handle } => match vm.handle_table.get(handle) {
            Ok(value) => match value_to_wire(value, None) {
                Ok(wire) => Msg::ReadHandleReturn {
                    value: wire,
                    error: None,
                    remote_stack: String::new(),
                },
                Err(e) => Msg::ReadHandleReturn {
                    value: WireData::Null,
                    error: Some(format!("read_handle: {e}")),
                    remote_stack: String::new(),
                },
            },
            Err(e) => Msg::ReadHandleReturn {
                value: WireData::Null,
                error: Some(e),
                remote_stack: String::new(),
            },
        },
        other => {
            return Err(QuoinError::Other(format!(
                "Extension call: unexpected message from extension: {other:?}"
            )));
        }
    };
    write_msg(vm, id, &reply)
}

/// Invoke the host block behind `block_handle` once per tuple in `batches`, minting a
/// call-local handle for each result. The host runs the block N times locally — the batch is
/// one re-entrant round-trip. Any bad handle or a raise during a block run fails the whole batch.
fn invoke_block_batches<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    epoch: u32,
    ext_id: u64,
    block_handle: u64,
    batches: &[Vec<u64>],
) -> Result<Vec<u64>, (String, String)> {
    // Resolve the handle to a block value (rooted in the handle table, so safe to hold).
    let block = match vm
        .handle_table
        .get(block_handle)
        .map_err(|e| (e, String::new()))?
    {
        Value::Object(obj) => match &obj.borrow().payload {
            ObjectPayload::Block(b) => *b,
            _ => {
                return Err((
                    format!("handle {block_handle} does not refer to a block"),
                    String::new(),
                ));
            }
        },
        _ => {
            return Err((
                format!("handle {block_handle} does not refer to a block"),
                String::new(),
            ));
        }
    };

    let mut results = Vec::with_capacity(batches.len());
    for tuple in batches {
        let mut arg_vals = Vec::with_capacity(tuple.len());
        for &handle in tuple {
            arg_vals.push(
                vm.handle_table
                    .get(handle)
                    .map_err(|e| (e, String::new()))?,
            );
        }
        // A raise inside the block travels to the peer as (short message, this host's
        // rendered stack segment) — the segment keeps the cross-process interleave.
        let result = vm.execute_block(mc, block, arg_vals, None).map_err(|e| {
            (
                format!("block invocation: {}", e.innermost()),
                quoin_stack_segment(&e),
            )
        })?;
        results.push(vm.handle_table.mint_local(result, epoch, ext_id));
    }
    Ok(results)
}

/// Cap an inbound cross-process stack blob: it is untrusted foreign text on an error
/// path — plenty for any real traceback, boring for a hostile peer. Truncation is noted
/// in-band so a clipped blob is never mistaken for a complete one.
const MAX_REMOTE_STACK: usize = 64 * 1024;

pub(crate) fn truncate_blob(mut blob: String) -> String {
    if blob.len() > MAX_REMOTE_STACK {
        let mut cut = MAX_REMOTE_STACK;
        while !blob.is_char_boundary(cut) {
            cut -= 1;
        }
        blob.truncate(cut);
        blob.push_str("\n[remote stack truncated]\n");
    }
    blob
}

/// Render a Quoin error as this host's segment of the cross-process stack blob: the
/// message, the frame lines the error carried, and — when the failure was itself a deeper
/// extension error — that error's own blob appended, preserving the interleave through
/// arbitrarily deep host<->extension nesting. Opaque to the peer (PROTOCOL.md §Errors).
pub(crate) fn quoin_stack_segment(e: &QuoinError) -> String {
    quoin_stack_segment_labeled(e, "host")
}

/// [`quoin_stack_segment`] with the side named — a worker's segment says
/// "(worker)", the host's says "(host)".
pub(crate) fn quoin_stack_segment_labeled(e: &QuoinError, side: &str) -> String {
    let mut seg = format!("--- Quoin ({side}) ---\n{}\n", e.innermost());
    if let QuoinError::WithSourceInfo { trace, .. } = e {
        for frame in trace {
            seg.push_str(frame);
            seg.push('\n');
        }
    }
    if let QuoinError::ExtensionError { remote_stack, .. } = e.innermost()
        && !remote_stack.is_empty()
    {
        seg.push_str(remote_stack);
        if !seg.ends_with('\n') {
            seg.push('\n');
        }
    }
    seg
}

fn ack() -> Msg {
    Msg::HostOpReturn {
        handle: 0,
        str: None,
        error: None,
        remote_stack: String::new(),
    }
}

fn host_op_error(message: String) -> Msg {
    Msg::HostOpReturn {
        handle: 0,
        str: None,
        error: Some(message),
        remote_stack: String::new(),
    }
}

/// A host-op that failed with a full Quoin error (a raise inside `call_method` or a block):
/// the short message plus this host's stack segment for the peer's blob.
fn host_op_failure(context: &str, e: &QuoinError) -> Msg {
    Msg::HostOpReturn {
        handle: 0,
        str: None,
        error: Some(format!("{context}{}", e.innermost())),
        remote_stack: quoin_stack_segment(e),
    }
}

/// Classify one extension-backed-class method argument (Phase 3) into a wire [`Arg`]: an instance
/// of *this* extension passes its object-table id (so a method can take another of the extension's
/// objects); a bulk `Array` passes inline on the data plane; a data-representable value passes its
/// `DataValue` (live instances of this extension allowed inside); anything else — a block, a
/// non-data host object, or a value involving *another* extension's instance — is minted a
/// call-local host-value handle the extension drives via `invoke_block` / `call_method`.
fn classify_arg<'gc>(
    vm: &mut VmState<'gc>,
    value: Value<'gc>,
    epoch: u32,
    ext_id: u64,
    owner: &Rc<RefCell<Vec<u64>>>,
) -> Arg {
    let owned = value
        .with_native_state::<NativeExtResource, _, _>(|r| {
            Rc::ptr_eq(&r.reap, owner).then_some(r.resource_id)
        })
        .ok()
        .flatten();
    if let Some(resource_id) = owned {
        Arg::Resource(resource_id)
    } else if let Some((dtype, data)) = array::array_parts(value) {
        let length = (data.len() / 8) as u64;
        Arg::Array(ArrowArray {
            dtype: to_wire_dtype(dtype),
            length,
            data,
        })
    } else if let Ok(wire) = value_to_wire(value, Some(owner)) {
        Arg::Data(wire)
    } else {
        Arg::Handle(vm.handle_table.mint_local(value, epoch, ext_id))
    }
}

/// Drive one extension call to completion: open a call epoch, optionally mint a call-local
/// handle for a host `block` the extension may invoke, send the `Call`, then service the
/// re-entrant host-op conversation until the terminal `CallReturn`. The epoch is closed out
/// unconditionally so the call's transient handles (including the block) never leak.
#[allow(clippy::too_many_arguments)] // extension call boundary: forwards the full dispatch context to the host
fn extension_call<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    id: StreamId,
    ext_id: u64,
    owner: &Rc<RefCell<Vec<u64>>>,
    op: String,
    argv: String,
    args: Vec<Value<'gc>>,
    data: Option<WireData>,
    class_name: String,
    recv: u64,
    releases: Vec<u64>,
    meter: &mut CallMeter,
) -> Result<CallOutcome<'gc>, QuoinError> {
    let epoch = vm.handle_table.begin_call();

    let mut handles = Vec::new();
    let mut resources = Vec::new();
    let mut arrays = Vec::new();
    let mut method_args = Vec::new();
    if class_name.is_empty() {
        // Generic `call:with:` paths: route each arg by token space — an `ExtResource` of *this*
        // extension passes its (ext-side) resource id; an `Array` is serialized into the bulk
        // data plane; any other value — including another extension's resource, whose id would
        // be misread in this extension's table — is minted a call-local host-value handle
        // (a block is one of these).
        for value in args {
            let owned = value
                .with_native_state::<NativeExtResource, _, _>(|r| {
                    Rc::ptr_eq(&r.reap, owner).then_some(r.resource_id)
                })
                .ok()
                .flatten();
            if let Some(resource_id) = owned {
                resources.push(resource_id);
            } else if let Some((dtype, data)) = array::array_parts(value) {
                let length = (data.len() / 8) as u64;
                arrays.push(ArrowArray {
                    dtype: to_wire_dtype(dtype),
                    length,
                    data,
                });
            } else {
                handles.push(vm.handle_table.mint_local(value, epoch, ext_id));
            }
        }
    } else {
        // Extension-backed-class method (Phase 3): build the ordered, tagged argument list, so a
        // method can take data, another of the extension's instances, and host blocks together.
        for value in args {
            method_args.push(classify_arg(vm, value, epoch, ext_id, owner));
        }
    }

    let outcome: Result<CallOutcome<'gc>, QuoinError> = (|| {
        meter.bytes_out += write_msg(
            vm,
            id,
            &Msg::Call {
                op,
                arg: argv,
                handles,
                resources,
                releases,
                arrays,
                data,
                class_name,
                recv,
                method_args,
            },
        )?;
        loop {
            let frame = read_reply_frame(vm, id)?;
            meter.bytes_in += 4 + frame.len() as u64;
            let (msg, reply_meta) = quoin_ext_proto::decode_frame_with_meta(&frame)
                .map_err(|e| QuoinError::Other(format!("Extension call: malformed frame: {e}")))?;
            // Host-op frames carry no meta (0); the terminal's — decoded last — wins.
            meter.handler_micros = reply_meta.handler_micros;
            match msg {
                Msg::CallReturn { result } => return Ok(CallOutcome::Scalar(result)),
                Msg::CallReturnResource {
                    resource,
                    class_name,
                } => {
                    return Ok(CallOutcome::Resource {
                        resource_id: resource,
                        class_name,
                    });
                }
                Msg::CallReturnArray { array } => return Ok(CallOutcome::Array(array)),
                Msg::CallReturnData { value } => return Ok(CallOutcome::Data(value)),
                // Resolve the returned handle to its `Value` *now*, before `end_call` sweeps the
                // call-local handle; the Value is returned to the caller (rooted by being live).
                Msg::CallReturnHandle { handle } => {
                    let value = vm.handle_table.get(handle).map_err(QuoinError::Other)?;
                    return Ok(CallOutcome::Value(value));
                }
                // ext -> host: the call failed recoverably. Raise a catchable Quoin error; the
                // extension stays alive (a normal terminal frame, not a crash) — `finish_outcome`'s
                // error path runs `note_if_exited`, which finds the child still running and so
                // propagates the error without marking the extension dead. The opaque stack blob
                // rides along (capped — untrusted foreign text): the printer shows it fenced,
                // Quoin code reads it as `ex.remoteStack`.
                Msg::CallReturnError {
                    message,
                    remote_stack,
                } => {
                    return Err(QuoinError::ExtensionError {
                        message,
                        remote_stack: truncate_blob(remote_stack),
                    });
                }
                host_op => {
                    meter.bytes_out += service_host_op(vm, mc, id, epoch, ext_id, host_op)?;
                }
            }
        }
    })();

    vm.handle_table.end_call(epoch);
    outcome
}

/// How a call finished: a scalar string, an ext-side resource the host will wrap as a token, a
/// bulk `Array`, a structured value, or a live host `Value` (a returned handle, already resolved).
enum CallOutcome<'gc> {
    Scalar(String),
    /// An ext-side resource; `class_name` names the registered extension-backed class it's an
    /// instance of (Phase 3 cross-class returns), or is empty for the opaque `ExtResource`.
    Resource {
        resource_id: u64,
        class_name: String,
    },
    Array(ArrowArray),
    Data(WireData),
    Value(Value<'gc>),
}

/// Wrap an ext-assigned resource id in a host value tied to `reap` so its `Drop` enqueues the id
/// for release on this extension's next call. `class` is the extension-backed class to wrap it as
/// (Phase 3), or `None` for the generic `call:with:` path, which wraps it as `ExtResource`.
fn wrap_resource<'gc>(
    vm: &VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    resource_id: u64,
    reap: Rc<RefCell<Vec<u64>>>,
    class: Option<Gc<'gc, RefLock<Class<'gc>>>>,
) -> Value<'gc> {
    let class = class.unwrap_or_else(|| vm.get_or_create_builtin_class(mc, "ExtResource"));
    vm.new_native_state(mc, class, NativeExtResource { resource_id, reap })
}

/// Resolve a returned resource's `class_name` (Phase 3) to the installed extension-backed-class
/// global it should be wrapped as. Empty — or a name that isn't a class global — is `None`, i.e.
/// the opaque `ExtResource` token (the generic `call:with:` path, or a defensive fallback).
fn resolve_ext_class<'gc>(
    vm: &VmState<'gc>,
    class_name: &str,
    namespace: Option<&str>,
) -> Option<Gc<'gc, RefLock<Class<'gc>>>> {
    if class_name.is_empty() {
        return None;
    }
    // The extension names a *simple* class (§4); re-apply the package namespace to find the
    // installed global (`[Ns]Name`), or resolve the bare name for the `spawn:` escape hatch.
    let full = match namespace {
        Some(ns) => format!("[{ns}]{class_name}"),
        None => class_name.to_string(),
    };
    match resolve_global(vm, &full) {
        Some(Value::Class(c)) => Some(c),
        _ => None,
    }
}

/// Materialize a finished call's outcome into a Quoin Value, and handle the error/death cases —
/// shared by the generic `call:with:` path and extension-backed-class dispatch (Phase 3). A
/// returned resource wraps as the class its `class_name` names (cross-class returns), or as the
/// opaque `ExtResource` when unnamed.
fn finish_outcome<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    ext_receiver: Value<'gc>,
    ext_id: u64,
    resource_reap: Rc<RefCell<Vec<u64>>>,
    outcome: Result<CallOutcome<'gc>, QuoinError>,
) -> Result<Value<'gc>, QuoinError> {
    match outcome {
        Ok(CallOutcome::Scalar(result)) => Ok(vm.new_string(mc, result)),
        Ok(CallOutcome::Resource {
            resource_id,
            class_name,
        }) => {
            // A cross-class return names a simple class; wrap it under this extension's package
            // namespace (the receiver's), so the returned instance is the right installed class.
            let namespace = ext_receiver
                .with_native_state::<NativeExtension, _, _>(|e| e.namespace.clone())
                .ok()
                .flatten();
            let class = resolve_ext_class(vm, &class_name, namespace.as_deref());
            Ok(wrap_resource(vm, mc, resource_id, resource_reap, class))
        }
        Ok(CallOutcome::Array(array)) => Ok(array::new_array(
            vm,
            mc,
            from_wire_dtype(array.dtype),
            array.data,
        )),
        // Materialize a returned structured value into a nested Quoin Value with the direct
        // walker. `Resource` leaves wrap as live instances of this extension (its reap queue +
        // namespace), so a method can return e.g. a List of instances.
        Ok(CallOutcome::Data(wire)) => {
            let namespace = ext_receiver
                .with_native_state::<NativeExtension, _, _>(|e| e.namespace.clone())
                .ok()
                .flatten();
            wire_to_value(
                vm,
                mc,
                &wire,
                Some(&ResCtx {
                    reap: &resource_reap,
                    namespace: namespace.as_deref(),
                }),
            )
        }
        // A returned live host value (already resolved from its handle).
        Ok(CallOutcome::Value(value)) => Ok(value),
        // A cancellation (a timeout via `Async.timeout:do:`, or a task cancel) interrupted the
        // call mid-conversation: the host abandoned a read, so the connection is desynced (a slow
        // extension's reply would arrive later, unread, and corrupt the next call). Mark the
        // extension dead + release its handles, then re-raise `Cancelled` unchanged so the timeout
        // combinator still turns it into a catchable `TimeoutError`. The peer is now unusable; the
        // program spawns a fresh `Extension` to retry.
        Err(QuoinError::Cancelled) => {
            // The peer is unusable (desynced) — tear its child + socket down promptly rather
            // than let a slow/wedged process linger until this `Extension` value is collected.
            let _ = ext_receiver
                .with_native_state_mut::<NativeExtension, _, _>(mc, |ext| ext.kill_now());
            vm.handle_table.release_for_ext(ext_id);
            Err(QuoinError::Cancelled)
        }
        Err(e) => {
            let exit = ext_receiver
                .with_native_state_mut::<NativeExtension, _, _>(mc, |ext| ext.note_if_exited())
                .ok()
                .flatten();
            match exit {
                // The child died: release the host-value handles it still held (its retained
                // globals) so they drop their GC roots instead of leaking until VM exit.
                Some(detail) => {
                    vm.handle_table.release_for_ext(ext_id);
                    let peer = ext_receiver
                        .with_native_state::<NativeExtension, _, _>(|e| {
                            e.stats.borrow().peer.clone()
                        })
                        .unwrap_or_else(|_| "extension".to_string());
                    Err(extension_dead_error(&peer, &detail))
                }
                // A LANE transport failure under a call is a death by §2
                // ("its connection closed under a call") even before the exit
                // is reap-visible — the EOF races `try_wait`. The framing is
                // desynced either way, so the peer is unusable: tear it down
                // like the cancel path does. User/callback errors never
                // travel this path (they cross as reply frames), so `Io`
                // here can only be the lane.
                None if matches!(e.innermost(), QuoinError::Io { .. }) => {
                    let peer = ext_receiver
                        .with_native_state_mut::<NativeExtension, _, _>(mc, |ext| {
                            ext.kill_now();
                            ext.stats.borrow().peer.clone()
                        })
                        .unwrap_or_else(|_| "extension".to_string());
                    vm.handle_table.release_for_ext(ext_id);
                    Err(extension_dead_error(
                        &peer,
                        "its connection failed mid-call",
                    ))
                }
                None => Err(e),
            }
        }
    }
}

/// The per-call context peeked from an `Extension`'s native state.
struct ExtCall {
    /// The lane socket this call's conversation is pinned to.
    id: StreamId,
    /// The claimed object: an ext resource id, or a per-call pseudo id (class-side/generic).
    object_id: u64,
    ext_id: u64,
    dead: bool,
    /// The supervision budget is spent (slice 3): the dead check raises the
    /// typed `#gaveUp` instead of `#exited`.
    gave_up: bool,
    /// Shared reap queue — to flush dropped-resource releases and to clone into a returned resource.
    resource_reap: Rc<RefCell<Vec<u64>>>,
    /// The dropped-resource ids drained from the reap queue, flushed to the extension as this
    /// call's `releases`.
    releases: Vec<u64>,
    /// The peer's boundary-profiling table (shared; the call folds itself in when it ends).
    stats: Rc<RefCell<BoundaryStats>>,
    /// How long this call parked waiting for its claim (0 on the uncontended path).
    claim_wait_micros: u64,
}

/// What a call claims: the receiver's resource id, or a fresh per-call pseudo-object
/// (class-side constructors/methods and the generic `call:with:` path — no instance to
/// serialize on, so the call contends only on lanes).
enum ExtClaimKey {
    Resource(u64),
    PerCall,
}

/// The most deeply one task may nest conversations on one lane socket (a host block
/// calling back into the extension that is invoking it, recursively). Both processes spend
/// real stack per level; past this, mutual host<->extension recursion is a bug — die loudly
/// and catchably rather than exhausting either side.
const MAX_CALL_DEPTH: u32 = 16;

/// Open one call on the extension, shared by the generic `call:with:` path and
/// extension-backed-class dispatch: claim the target object under §5.1 (per-resource
/// mailboxes + the lane pool — `claim_peer_object`, the exact machinery hosted services
/// run), then pin the conversation to a lane socket and drain the pending dropped-resource
/// releases. A top-level call takes a joint (object, lane) claim and pops a free socket
/// (the lane token guarantees one); the SAME task re-entering (from inside a block the
/// extension is invoking) NESTS — object-only claim, frames riding its existing socket
/// strictly LIFO. Calls to different instances overlap up to the lane count; calls to one
/// instance serialize FIFO. The caller must close the call (`ext_end_call`) once it
/// completes — nested or not.
fn ext_prelude<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    receiver: Value<'gc>,
    key: ExtClaimKey,
    object_label: &str,
    what: &str,
) -> Result<ExtCall, QuoinError> {
    let me = vm.sched.current_task;
    // Peek: mint the object id, find whether this task already has a conversation open
    // (→ nested), and take the claims handle. The depth cap is checked here, before the
    // claim, so an over-deep nest never holds a fresh object claim it would have to undo.
    let (object_id, conv_depth, claims) = receiver
        .with_native_state_mut::<NativeExtension, _, _>(mc, |e| {
            let object_id = match key {
                ExtClaimKey::Resource(id) => id,
                ExtClaimKey::PerCall => {
                    e.next_pseudo += 1;
                    PSEUDO_OBJECT_BASE | e.next_pseudo
                }
            };
            let conv_depth = e.convs.get(&me.0).map(|c| c.depth);
            (object_id, conv_depth, e.claims.clone())
        })
        .map_err(QuoinError::Other)?;
    if conv_depth.is_some_and(|d| d >= MAX_CALL_DEPTH) {
        return Err(QuoinError::Other(format!(
            "extension call nested {MAX_CALL_DEPTH} levels deep on one connection — \
             mutual host<->extension recursion? (each level is a live call frame in \
             both processes)"
        )));
    }
    let kind = if conv_depth.is_some() {
        WaitKind::Nested
    } else {
        WaitKind::TopLevel
    };
    let claim_wait_micros = claim_peer_object(
        vm,
        receiver,
        &claims,
        object_id,
        object_label,
        what,
        kind,
        "extension claim",
    )?;
    // Claimed: pin the conversation to a lane socket and build the call context (releases
    // drained here, once per call).
    receiver
        .with_native_state_mut::<NativeExtension, _, _>(mc, |e| {
            if !e.convs.contains_key(&me.0) {
                let sock = e
                    .free_socks
                    .pop()
                    .expect("extension lane token granted with no free socket");
                e.convs.insert(me.0, ExtConv { sock, depth: 0 });
            }
            let conv = e.convs.get_mut(&me.0).expect("inserted above");
            conv.depth += 1;
            ExtCall {
                id: conv.sock,
                object_id,
                ext_id: e.ext_id,
                dead: e.dead,
                gave_up: e.gave_up,
                resource_reap: e.resource_reap.clone(),
                releases: e.resource_reap.borrow_mut().drain(..).collect(),
                stats: e.stats.clone(),
                claim_wait_micros,
            }
        })
        .map_err(QuoinError::Other)
}

/// Close the call opened by [`ext_prelude`] (whether it succeeded, errored, or the
/// extension died): pop one conversation level — the outermost pop returns the lane socket
/// to the pool — then release the object claim, waking FIFO handoffs (stale cancelled
/// waiters skipped by park-epoch identity, exactly as in `channel.rs`). Never touches
/// state when the extension is gone from the table.
fn ext_end_call<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    receiver: Value<'gc>,
    object_id: u64,
) {
    let me = vm.sched.current_task;
    let claims = receiver
        .with_native_state_mut::<NativeExtension, _, _>(mc, |e| {
            if let Some(conv) = e.convs.get_mut(&me.0) {
                conv.depth = conv.depth.saturating_sub(1);
                if conv.depth == 0 {
                    let sock = conv.sock;
                    e.convs.remove(&me.0);
                    // Returned BEFORE the grants are delivered below: a woken top-level
                    // waiter pops its socket when it actually resumes.
                    e.free_socks.push(sock);
                }
            }
            e.claims.clone()
        })
        .ok();
    if let Some(claims) = claims {
        end_peer_call_and_wake(vm, &claims, object_id, me.0);
    }
}

/// The shared body of the `call:` selectors: fail fast if the extension is already known dead,
/// flush dropped-resource releases, run the call, and materialize the result (or surface a typed
/// "died"/cancelled error). The generic path passes no `class_name`/`recv` and wraps a returned
/// resource as the opaque `ExtResource` token.
fn run_extension_method<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    receiver: Value<'gc>,
    op: String,
    argv: String,
    args: Vec<Value<'gc>>,
    data_arg: Option<Value<'gc>>,
) -> Result<Value<'gc>, QuoinError> {
    // The generic path has no receiver instance: each call claims a fresh pseudo-object,
    // so generic calls serialize only on lane availability.
    let what = format!("extension call '{op}'");
    let ctx = ext_prelude(vm, mc, receiver, ExtClaimKey::PerCall, &op, &what)?;
    if ctx.dead {
        ext_end_call(vm, mc, receiver, ctx.object_id);
        let peer = ctx.stats.borrow().peer.clone();
        if ctx.gave_up {
            return Err(QuoinError::peer_died(
                peer.clone(),
                crate::error::PeerDeathReason::GaveUp,
                format!("`{peer}` gave up — its supervision budget is spent"),
            ));
        }
        return Err(extension_dead_error(&peer, "already exited"));
    }
    // Serialize the optional structured-value payload before opening the call (this extension's
    // own live instances are allowed inside). If it fails (e.g. a value with no data
    // representation) release the in-flight claim first.
    let data = match data_arg {
        Some(value) => match value_to_wire(value, Some(&ctx.resource_reap)) {
            Ok(d) => Some(d),
            Err(e) => {
                ext_end_call(vm, mc, receiver, ctx.object_id);
                return Err(e);
            }
        },
        None => None,
    };
    let op_name = op.clone();
    let mut meter = CallMeter::default();
    let started = Instant::now();
    let outcome = extension_call(
        vm,
        mc,
        ctx.id,
        ctx.ext_id,
        &ctx.resource_reap,
        op,
        argv,
        args,
        data,
        String::new(),
        0,
        ctx.releases,
        &mut meter,
    );
    record_boundary(
        &ctx.stats,
        "",
        &op_name,
        &meter,
        started.elapsed().as_micros() as u64,
        ctx.claim_wait_micros,
        outcome.is_err(),
    );
    ext_end_call(vm, mc, receiver, ctx.object_id);
    finish_outcome(vm, mc, receiver, ctx.ext_id, ctx.resource_reap, outcome)
}

/// Dispatch a method send on an extension-backed class (Phase 3) over the socket. `ext` is the
/// owning `Extension` value; `receiver` is the class itself (class-side — a constructor) or an
/// instance (instance-side). The selector is forwarded as the `Call.op`, the class name routes it
/// on the extension side, and `recv` is the receiver instance's resource id (0 for class-side).
/// The method arguments are routed into the tagged `method_args` (data / ext-instances / blocks);
/// a returned resource wraps as the class its `class_name` names (cross-class returns).
pub fn dispatch_ext_method<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    ext: Value<'gc>,
    receiver: Value<'gc>,
    selector: Symbol,
    args: Vec<Value<'gc>>,
) -> Result<Value<'gc>, QuoinError> {
    // The receiver determines the class, the dispatch side, and (for an instance) the resource id.
    let (class_obj, recv) = match receiver {
        Value::Class(c) => (c, 0u64),
        Value::Object(o) => {
            let class = o.borrow().class;
            let (resource_id, res_reap) = receiver
                .with_native_state::<NativeExtResource, _, _>(|r| (r.resource_id, r.reap.clone()))
                .map_err(|_| {
                    QuoinError::Other(format!(
                        "'{}' is not an extension-backed instance",
                        selector.as_str()
                    ))
                })?;
            // Rule-6 staleness (SUPERVISION.md slice 2): a resource minted by
            // a dead incarnation holds ITS reap queue — a restarted extension
            // swapped in a fresh one, so identity mismatch = permanently
            // stale. (The argument path refuses the same way via the
            // value_to_wire ownership check.)
            let stale = ext
                .with_native_state::<NativeExtension, _, _>(|e| {
                    (!Rc::ptr_eq(&e.resource_reap, &res_reap))
                        .then(|| (e.incarnation, e.stats.borrow().peer.clone()))
                })
                .ok()
                .flatten();
            if let Some((inc, peer)) = stale {
                return Err(QuoinError::peer_died(
                    peer.clone(),
                    crate::error::PeerDeathReason::StaleIncarnation,
                    format!(
                        "extension call '{}': this instance belongs to a dead incarnation \
                         of `{peer}` (the extension is now incarnation {inc}) — re-create \
                         it from the current one",
                        selector.as_str()
                    ),
                ));
            }
            (class, resource_id)
        }
        _ => {
            return Err(QuoinError::Other(format!(
                "extension method '{}' has an unexpected receiver",
                selector.as_str()
            )));
        }
    };
    // The extension routes on the *simple* class name it registered; the host-applied package
    // namespace (`EXT_PACKAGING.md` §4) is stripped here — `name.name` is the bare class name for
    // both `[Ns]Name` (loadPackage) and a verbatim bare name (spawn:).
    let class_name = class_obj.borrow().name.name.clone();

    // An instance send claims ITS resource — the per-object mailbox, so calls to one
    // instance serialize while calls to different instances overlap up to the lane count.
    // A class-side send (recv 0) claims a fresh pseudo-object: constructors contend only
    // on lanes (parallel `connect`s are the point of a DB extension declaring lanes).
    let key = if recv == 0 {
        ExtClaimKey::PerCall
    } else {
        ExtClaimKey::Resource(recv)
    };
    let what = format!("extension call '{}.{}'", class_name, selector.as_str());
    let ctx = ext_prelude(vm, mc, ext, key, &class_name, &what)?;
    if ctx.dead {
        ext_end_call(vm, mc, ext, ctx.object_id);
        let peer = ctx.stats.borrow().peer.clone();
        if ctx.gave_up {
            return Err(QuoinError::peer_died(
                peer.clone(),
                crate::error::PeerDeathReason::GaveUp,
                format!("`{peer}` gave up — its supervision budget is spent"),
            ));
        }
        return Err(extension_dead_error(&peer, "already exited"));
    }
    // The method arguments are routed by `extension_call` (ext-class mode) into the ordered
    // `method_args` — data, ext-instances, and host blocks each by their kind.
    let mut meter = CallMeter::default();
    let started = Instant::now();
    let outcome = extension_call(
        vm,
        mc,
        ctx.id,
        ctx.ext_id,
        &ctx.resource_reap,
        selector.as_str().to_string(),
        String::new(),
        args,
        None,
        class_name.clone(),
        recv,
        ctx.releases,
        &mut meter,
    );
    record_boundary(
        &ctx.stats,
        &class_name,
        selector.as_str(),
        &meter,
        started.elapsed().as_micros() as u64,
        ctx.claim_wait_micros,
        outcome.is_err(),
    );
    ext_end_call(vm, mc, ext, ctx.object_id);
    finish_outcome(vm, mc, ext, ctx.ext_id, ctx.resource_reap, outcome)
}

/// Read one length-prefixed reply frame, but fail with a `TimedOut` error if the whole
/// read takes longer than `timeout_ms`. Like [`read_reply_frame`], but each `Read` park
/// carries the *remaining* budget (via `IoRequest::ReadTimed`), so a peer that accepts
/// the socket and then goes silent cannot hang the caller.
fn read_reply_frame_timed<'gc>(
    vm: &mut VmState<'gc>,
    id: StreamId,
    timeout_ms: u64,
) -> Result<Vec<u8>, QuoinError> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
    let mut buf: Vec<u8> = Vec::new();
    let read_more = |vm: &mut VmState<'gc>, buf: &mut Vec<u8>| -> Result<(), QuoinError> {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Err(QuoinError::Io {
                kind: crate::error::IoErrorKind::TimedOut,
                message: format!("extension handshake timed out after {timeout_ms}ms"),
            });
        }
        match vm.await_io(IoRequest::ReadTimed {
            id,
            max: 4096,
            ms: remaining.as_millis() as u64,
            buf: Vec::new(),
        })? {
            IoResult::Read(chunk) if chunk.is_empty() => Err(QuoinError::Other(
                "Extension handshake: connection closed before manifest".to_string(),
            )),
            IoResult::Read(chunk) => {
                buf.extend_from_slice(&chunk);
                Ok(())
            }
            IoResult::Err(e) => Err(QuoinError::from_io_error(&e)),
            other => Err(QuoinError::Other(format!(
                "Extension handshake: unexpected read result {other:?}"
            ))),
        }
    };
    while buf.len() < 4 {
        read_more(vm, &mut buf)?;
    }
    let len = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
    if len > quoin_ext_proto::MAX_FRAME_LEN {
        return Err(QuoinError::Other(format!(
            "Extension handshake: manifest frame length {len} exceeds the {} byte limit",
            quoin_ext_proto::MAX_FRAME_LEN
        )));
    }
    while buf.len() < 4 + len {
        read_more(vm, &mut buf)?;
    }
    Ok(buf[4..4 + len].to_vec())
}

/// Fetch an extension's class manifest right after connect (Phase 3): send `GetManifest` and read
/// the single `ManifestReturn`. An extension that provides no classes returns an empty list, so the
/// generic `call:with:` extensions stay backward-compatible. The read is time-bounded so a silent
/// extension fails the spawn instead of hanging the VM. This exchange is also the protocol-version
/// handshake — an SDK speaking a different version is refused here, with both versions named,
/// before any other frame is interpreted. Also returns the manifest's declared lane count,
/// normalized: 0/absent (a pre-lanes SDK) means 1, and the cap matches the worker surface's.
fn fetch_manifest<'gc>(
    vm: &mut VmState<'gc>,
    id: StreamId,
) -> Result<(Vec<ClassDecl>, u32), QuoinError> {
    write_msg(
        vm,
        id,
        &Msg::GetManifest {
            version: PROTOCOL_VERSION,
        },
    )?;
    let frame = read_reply_frame_timed(vm, id, crate::tuning::ext_handshake_timeout_ms())?;
    match quoin_ext_proto::decode_frame(&frame)
        .map_err(|e| QuoinError::Other(format!("Extension manifest: malformed frame: {e}")))?
    {
        Msg::ManifestReturn {
            classes,
            version,
            lanes,
        } => {
            if version != PROTOCOL_VERSION {
                return Err(QuoinError::Other(format!(
                    "Extension manifest: the extension SDK speaks protocol version {version}, \
                     this host speaks {PROTOCOL_VERSION} — update the older side"
                )));
            }
            Ok((classes, lanes.clamp(1, 1024)))
        }
        other => Err(QuoinError::Other(format!(
            "Extension manifest: expected ManifestReturn, got {other:?}"
        ))),
    }
}

/// Extract the elements of a Quoin list value passed as the `args:` argument.
pub(crate) fn extract_args<'gc>(value: Value<'gc>) -> Result<Vec<Value<'gc>>, QuoinError> {
    value
        .with_native_state::<NativeListState, _, _>(|l| l.get_vec().to_vec())
        .map_err(|_| QuoinError::TypeError {
            expected: "List".to_string(),
            got: value.type_name().to_string(),
            msg: "call:with:args: expects a list of arguments".to_string(),
        })
}

/// Spawn an extension subprocess and bring it up: exec `command` with `args` (with the unix-socket
/// path appended as the final argv, as the transport requires), optionally in `cwd`; connect the
/// UDS (retrying until the child binds it, each attempt parking the fiber); fetch the class
/// manifest; and build the `Extension` value. Returns the value plus its manifest — the caller
/// installs the classes, deciding the naming (verbatim for `spawn:`, namespaced for `loadPackage:`).
/// The manifest is fetched *before* the value exists, so no GC value is held across the parking I/O.
fn spawn_and_connect<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    command: &str,
    args: &[String],
    cwd: Option<&Path>,
    namespace: Option<String>,
) -> Result<(Value<'gc>, Vec<ClassDecl>), QuoinError> {
    let spawned = spawn_ext_process(vm, command, args, cwd)?;
    finish_ext_spawn(vm, mc, command, args, cwd, namespace, spawned)
}

/// The spawn/connect/handshake front half — plain parts, no `Extension` value
/// yet — shared by the original spawn and `restart` (which GATES the manifest
/// against the recipe instead of installing it).
struct SpawnedExt {
    child: Child,
    sock_path: String,
    socks: Vec<StreamId>,
    manifest: Vec<ClassDecl>,
    lanes: u32,
}

impl SpawnedExt {
    /// Tear a spawned-but-unadopted child down (a failed restart gate): kill
    /// it, drop its socket file, and reap the connected fds.
    fn abandon(mut self, vm: &VmState<'_>) {
        vm.io
            .socket_reap
            .borrow_mut()
            .extend(self.socks.iter().copied());
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.sock_path);
    }
}

fn spawn_ext_process<'gc>(
    vm: &mut VmState<'gc>,
    command: &str,
    args: &[String],
    cwd: Option<&Path>,
) -> Result<SpawnedExt, QuoinError> {
    let sock_path = unique_sock_path();
    let mut cmd = Command::new(command);
    cmd.args(args).arg(&sock_path);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| QuoinError::Other(format!("Extension: failed to start '{command}': {e}")))?;

    // The child binds the socket asynchronously after exec, so retry the connect briefly until it's
    // listening (each attempt parks the fiber).
    //
    // A single-lane SDK unlinks the path as soon as it accepts; a multi-lane one keeps it until
    // its accept loop finishes (the extra lanes below connect to this same path). Either way a
    // healthy extension leaves nothing behind. The failure arms below still remove it: a child
    // that binds and then dies before it can accept -- or a third-party SDK that never unlinks --
    // would otherwise strand the file, and we are the last party that knows the path.
    let id = match connect_ext_lane(vm, &sock_path) {
        Ok(id) => id,
        Err(e) => {
            let _ = child.kill();
            let _ = std::fs::remove_file(&sock_path);
            return Err(e);
        }
    };

    // Fetch the class manifest (Phase 3) before creating the value: the fetch parks the fiber (a GC
    // point), so no GC value may be held across it. A generic extension returns an empty manifest.
    // On any handshake failure (including the timeout) the child isn't owned by an `Extension` value
    // yet, so kill it here rather than orphan it.
    let (manifest, lanes) = match fetch_manifest(vm, id) {
        Ok(m) => m,
        Err(e) => {
            let _ = child.kill();
            let _ = std::fs::remove_file(&sock_path);
            return Err(e);
        }
    };

    // The manifest declared its lane count: open the remaining lane connections to the same
    // path (the SDK's accept loop is serving them each on its own thread). One conversation
    // runs per lane; the §5.1 claim state below is what arbitrates them.
    let mut socks = vec![id];
    for _ in 1..lanes {
        match connect_ext_lane(vm, &sock_path) {
            Ok(lane_id) => socks.push(lane_id),
            Err(e) => {
                // The fds already connected are not yet owned by an `Extension` value;
                // enqueue them for the driver to close rather than leak them.
                vm.io.socket_reap.borrow_mut().extend(socks.iter().copied());
                let _ = child.kill();
                let _ = std::fs::remove_file(&sock_path);
                return Err(e);
            }
        }
    }
    Ok(SpawnedExt {
        child,
        sock_path,
        socks,
        manifest,
        lanes,
    })
}

/// The back half of the original spawn: registry rows, the recipe, and the
/// `Extension` value.
fn finish_ext_spawn<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    command: &str,
    args: &[String],
    cwd: Option<&Path>,
    namespace: Option<String>,
    spawned: SpawnedExt,
) -> Result<(Value<'gc>, Vec<ClassDecl>), QuoinError> {
    let SpawnedExt {
        child,
        sock_path,
        socks,
        manifest,
        lanes,
    } = spawned;
    // Boundary profiling: one table per peer, registered globally so `VM.boundaryStats`
    // can enumerate it — and keep it after the extension dies or is dropped. The claim
    // state registers beside it for `VM.claims`, under the same peer label.
    let peer = namespace.clone().unwrap_or_else(|| {
        Path::new(command)
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_else(|| command.to_string())
    });
    let stats = Rc::new(RefCell::new(BoundaryStats {
        peer: peer.clone(),
        rows: HashMap::new(),
    }));
    vm.io.ext_stats.borrow_mut().push(stats.clone());
    let claims = Rc::new(RefCell::new(PeerClaims::new(peer.clone(), lanes)));
    vm.io.claim_peers.borrow_mut().push(claims.clone());
    // Lifecycle sink (SUPERVISION.md slice 1): the roster row for `VM.peers`
    // and the staging behind `e.events`; the exit watch arms lazily there.
    let life =
        crate::runtime::lifecycle::LifeSink::new(peer, "extension", "process", Some(child.id()));
    let life_idx = {
        let lives = vm.io.lives.clone();
        let mut lives = lives.borrow_mut();
        lives.push(life.clone());
        lives.len() - 1
    };

    let class = vm.get_or_create_builtin_class(mc, "Extension");
    let ext_val = vm.new_native_state(
        mc,
        class,
        NativeExtension {
            free_socks: socks.clone(),
            socks,
            convs: HashMap::new(),
            claims,
            next_pseudo: 0,
            ext_id: unique_ext_id(),
            child,
            sock_path,
            reap: vm.io.socket_reap.clone(),
            handle_reap: vm.io.ext_handle_reap.clone(),
            dead: false,
            resource_reap: Rc::new(RefCell::new(Vec::new())),
            namespace,
            stats,
            life,
            life_idx,
            recipe: Rc::new(ExtRecipe {
                command: command.to_string(),
                args: args.to_vec(),
                cwd: cwd.map(|p| p.to_path_buf()),
                manifest: manifest.clone(),
                lanes,
            }),
            incarnation: 1,
            policy: None,
            gave_up: false,
            hook: std::cell::Cell::new(None),
        },
    );
    Ok((ext_val, manifest))
}

/// Connect one lane to the extension's socket path, retrying briefly (each attempt parks
/// the fiber): the child binds asynchronously after exec, and the extra lanes race its
/// accept loop.
fn connect_ext_lane<'gc>(vm: &mut VmState<'gc>, sock_path: &str) -> Result<StreamId, QuoinError> {
    let mut attempts = 0u32;
    loop {
        match vm.await_io(IoRequest::ConnectUnix {
            path: sock_path.to_string(),
        })? {
            IoResult::Connected(id) => return Ok(id),
            IoResult::Err(_) if attempts < 100 => {
                attempts += 1;
                vm.await_io(IoRequest::Sleep { ms: 5 })?;
            }
            IoResult::Err(e) => return Err(QuoinError::from_io_error(&e)),
            other => {
                return Err(QuoinError::Other(format!(
                    "Extension: unexpected connect result {other:?}"
                )));
            }
        }
    }
}

/// The launch + identity spec parsed from a package's `quoin.toml` (`EXT_PACKAGING.md` §3).
struct PackageSpec {
    /// `[package].name` — canonical metadata (the directory name is what `use` resolves).
    name: String,
    /// `[extension].command` — how to launch the subprocess.
    command: String,
    /// `[extension].args` — its arguments (the socket path is appended after these).
    args: Vec<String>,
    /// `[extension].namespace`, or PascalCase of the directory name — the namespace every provided
    /// class is installed under (§4).
    namespace: String,
    /// The `[extension]` supervision keys (SUPERVISION.md slice 3): `restart =
    /// "always"` opts in; `backoff-ms` / `cap-ms` / `max-restarts` /
    /// `window-ms` refine the defaults. Package-declared only in v1 (§10.5).
    supervise: Option<crate::runtime::supervise::SupervisePolicy>,
}

/// Read and parse `<dir>/quoin.toml` into a [`PackageSpec`] (v1: `[package].name` + the
/// `[extension]` launch spec). The namespace defaults to PascalCase of the directory name (§4).
fn read_package_manifest(dir: &Path) -> Result<PackageSpec, QuoinError> {
    let manifest_path = dir.join("quoin.toml");
    let text = std::fs::read_to_string(&manifest_path).map_err(|e| {
        QuoinError::Other(format!(
            "Extension loadPackage: cannot read {}: {e}",
            manifest_path.display()
        ))
    })?;
    let value: toml::Value = text.parse().map_err(|e| {
        QuoinError::Other(format!(
            "Extension loadPackage: invalid {}: {e}",
            manifest_path.display()
        ))
    })?;

    let dir_name = dir.file_name().and_then(|s| s.to_str()).unwrap_or("ext");
    let name = value
        .get("package")
        .and_then(|p| p.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or(dir_name)
        .to_string();

    let ext = value.get("extension").ok_or_else(|| {
        QuoinError::Other(format!(
            "Extension loadPackage: {} has no [extension] table",
            manifest_path.display()
        ))
    })?;
    let command = ext
        .get("command")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            QuoinError::Other(format!(
                "Extension loadPackage: {} [extension] is missing 'command'",
                manifest_path.display()
            ))
        })?
        .to_string();
    let args = ext
        .get("args")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let namespace = ext
        .get("namespace")
        .and_then(|v| v.as_str())
        .map(String::from)
        .unwrap_or_else(|| pascal_case(dir_name));

    // The supervision keys (slice 3): opt-in via `restart = "always"`;
    // anything else (or absence) is fail-fast. Refinements apply only when
    // opted in — a stray `backoff-ms` under `restart = "never"` is an error,
    // not a silent no-op.
    let restart_mode = ext.get("restart").and_then(|v| v.as_str());
    let tuning_key = ["backoff-ms", "cap-ms", "max-restarts", "window-ms"]
        .iter()
        .find(|k| ext.get(**k).is_some());
    let supervise = match restart_mode {
        Some("always") => {
            let mut p = crate::runtime::supervise::SupervisePolicy::default();
            let read = |key: &str, min: i64| -> Result<Option<u64>, QuoinError> {
                match ext.get(key) {
                    None => Ok(None),
                    Some(v) => match v.as_integer() {
                        Some(n) if n >= min => Ok(Some(n as u64)),
                        _ => Err(QuoinError::Other(format!(
                            "Extension loadPackage: [extension] {key} must be an integer >= {min}"
                        ))),
                    },
                }
            };
            if let Some(n) = read("backoff-ms", 0)? {
                p.backoff_ms = n;
            }
            if let Some(n) = read("cap-ms", 1)? {
                p.cap_ms = n;
            }
            if let Some(n) = read("max-restarts", 1)? {
                p.max_restarts = n as u32;
            }
            if let Some(n) = read("window-ms", 1)? {
                p.window_ms = n;
            }
            Some(p)
        }
        Some("never") | None => {
            if restart_mode.is_none()
                && let Some(k) = tuning_key
            {
                return Err(QuoinError::Other(format!(
                    "Extension loadPackage: [extension] {k} without `restart = \"always\"` \
                     does nothing — add the opt-in or drop the key"
                )));
            }
            None
        }
        Some(other) => {
            return Err(QuoinError::Other(format!(
                "Extension loadPackage: [extension] restart = \"{other}\" is not a mode \
                 (always|never)"
            )));
        }
    };

    Ok(PackageSpec {
        name,
        command,
        args,
        namespace,
        supervise,
    })
}

/// PascalCase a directory name for the default package namespace (`my-vectors` -> `MyVectors`).
pub(crate) fn pascal_case(s: &str) -> String {
    s.split(['-', '_', ' '])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut cs = w.chars();
            match cs.next() {
                Some(first) => first.to_uppercase().collect::<String>() + cs.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

/// Resolve a manifest `command` against the package dir: an absolute path is used as-is; a relative
/// path *with a separator* (`bin/ext`, `../target/release/adbc`) is taken relative to the package
/// dir so it finds the bundled binary; a bare command (`python3`) is left for a `PATH` lookup.
fn resolve_command(dir: &Path, command: &str) -> PathBuf {
    let p = Path::new(command);
    if p.is_absolute() {
        p.to_path_buf()
    } else if command.contains('/') {
        dir.join(p)
    } else {
        p.to_path_buf()
    }
}

/// Load an extension *package* (a `use`-able folder; `docs/internal/EXT_PACKAGING.md`): read its
/// `quoin.toml`, spawn the subprocess, install the provided classes **under the package
/// namespace** (so a package can never register a bare global — §4; an already-namespaced
/// `ClassDecl` name is rejected), run the package's optional `init.qn` Quoin glue now that its
/// classes exist, and cache the live `Extension` keyed by the canonical folder (idempotent: a
/// repeat load returns the cached extension rather than re-spawning).
// GC-rooting: `ext_val` is rooted by the classes installed from it (`install_ext_class`,
// which performs no Quoin execution and cannot yield); the only later yield — running an
// optional `init.qn` via `eval_string` — happens after those classes exist, so `ext_val`
// is reachable through the globals for the duration. See the inline note below.
fn load_package<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    dir: &str,
) -> Result<Value<'gc>, QuoinError> {
    let dir_path = std::fs::canonicalize(dir).map_err(|e| {
        QuoinError::Other(format!(
            "Extension loadPackage: cannot resolve package dir '{dir}': {e}"
        ))
    })?;
    let key = dir_path.to_string_lossy().to_string();

    // Idempotent: a folder already loaded this session returns its live extension (no re-spawn).
    if let Some(existing) = vm.modules.packages.borrow().get(&key).copied() {
        return Ok(existing);
    }

    let spec = read_package_manifest(&dir_path)?;
    let command = resolve_command(&dir_path, &spec.command);
    let command = command.to_string_lossy().to_string();
    let (ext_val, classes) = spawn_and_connect(
        vm,
        mc,
        &command,
        &spec.args,
        Some(&dir_path),
        Some(spec.namespace.clone()),
    )?;

    // Root `ext_val` on the VM stack for the rest of the load: the old claim
    // that the installed classes root it fails for a ZERO-class package whose
    // init.qn (a yield-capable eval) runs below.
    vm.push(ext_val);
    // Namespacing (§4): the host prefixes the package namespace onto each simple `ClassDecl` name;
    // a package that ships an already-namespaced name doesn't get to choose its namespace.
    for decl in &classes {
        if decl.name.contains('[') {
            return Err(QuoinError::Other(format!(
                "Extension loadPackage: package '{}' class '{}' must declare a simple name \
                 (the package namespace is applied by the host)",
                spec.name, decl.name
            )));
        }
        let full = format!("[{}]{}", spec.namespace, decl.name);
        vm.install_ext_class(
            mc,
            ext_val,
            &full,
            &decl.instance_selectors,
            &decl.class_selectors,
        );
    }

    // Run the package's Quoin-side glue (convenience methods / class reopenings) now that its
    // classes are installed. `init.qn` is optional; the loader holds the absolute dir, so it just
    // reads its own sibling — no "where am I on disk?" problem. No `await_io` (class defs), so
    // `ext_val` — already rooted by the installed classes — is not held across a collection point.
    let init_path = dir_path.join("init.qn");
    if let Ok(src) = std::fs::read_to_string(&init_path) {
        eval_string(vm, mc, &src, &init_path.to_string_lossy(), None, &[]).map_err(|e| {
            QuoinError::Other(format!(
                "Extension loadPackage: package '{}' init.qn failed: {e}",
                spec.name
            ))
        })?;
    }

    let ext_val = vm.pop()?;
    vm.modules.packages.borrow_mut(mc).insert(key, ext_val);
    // The `[extension]` supervision keys (slice 3): package-declared policy
    // attaches exactly like `e.supervise:` would — watch armed, supervisor
    // spawned. The package cache above keeps `ext_val` rooted for its life.
    if let Some(policy) = spec.supervise {
        attach_extension_policy(vm, mc, ext_val, policy, "Extension loadPackage")?;
    }
    Ok(ext_val)
}

/// The respawn body shared by the manual `restart` and the supervisor
/// (SUPERVISION.md slices 2–3): spawn from the frozen recipe, gate the
/// manifest (rule 9 — the installed classes must stay truthful, selector for
/// selector, lane for lane; a mismatch abandons the fresh child), clean the
/// dead incarnation's residue, and rebind the handle IN PLACE.
fn extension_restart_attempt<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    receiver: Value<'gc>,
    recipe: &Rc<ExtRecipe>,
    old_ext_id: u64,
    inc: u64,
    peer: &str,
) -> Result<(), QuoinError> {
    let spawned = spawn_ext_process(vm, &recipe.command, &recipe.args, recipe.cwd.as_deref())?;
    if spawned.manifest != recipe.manifest || spawned.lanes != recipe.lanes {
        spawned.abandon(vm);
        return Err(QuoinError::Other(format!(
            "Extension.restart: the new incarnation's manifest does not match the \
             installed classes of `{peer}` — the extension changed underfoot; \
             refusing to rebind (SUPERVISION.md §4 rule 9)"
        )));
    }
    // The dead incarnation's residue: release its host-value handles
    // (an idle death had no failing call to do it).
    vm.handle_table.release_for_ext(old_ext_id);
    // Per-incarnation rows: fresh claims and a fresh lifecycle sink
    // (`e.events` re-asked answers the new incarnation's stream).
    let next = inc + 1;
    let label = format!("{peer} (incarnation {next})");
    let claims = Rc::new(RefCell::new(PeerClaims::new(label, recipe.lanes)));
    vm.io.claim_peers.borrow_mut().push(claims.clone());
    let life = crate::runtime::lifecycle::LifeSink::new(
        peer.to_string(),
        "extension",
        "process",
        Some(spawned.child.id()),
    );
    let life_for_hook = life.clone();
    life.incarnation
        .store(next, std::sync::atomic::Ordering::Relaxed);
    let new_life_idx = {
        let lives = vm.io.lives.clone();
        let mut lives = lives.borrow_mut();
        lives.push(life.clone());
        lives.len() - 1
    };
    // Rebind IN PLACE: same value, same installed classes, fresh
    // transport. The old reap queue is the dead incarnation's
    // identity — its resources go #staleIncarnation by pointer
    // mismatch; the old lane fds reap; the old socket file goes.
    receiver
        .with_native_state_mut::<NativeExtension, _, _>(mc, |e| {
            let old_socks = std::mem::replace(&mut e.socks, spawned.socks.clone());
            e.reap.borrow_mut().extend(old_socks);
            let _ = std::fs::remove_file(&e.sock_path);
            e.free_socks = spawned.socks;
            e.convs.clear();
            e.claims = claims;
            e.next_pseudo = 0;
            e.ext_id = unique_ext_id();
            e.child = spawned.child;
            e.sock_path = spawned.sock_path;
            e.dead = false;
            e.resource_reap = Rc::new(RefCell::new(Vec::new()));
            e.life = life;
            e.life_idx = new_life_idx;
            e.incarnation = next;
        })
        .map_err(QuoinError::Other)?;
    // The user's restart hook (`onRestart:`), the attempt's tail: the
    // transport is live, so the hook reconfigures the fresh child through
    // the installed classes. A failure fails the ATTEMPT — the fresh child
    // is killed (it must not serve half-set-up), the death lands on its
    // sink, and the caller (supervisor or manual restart) counts/relays it.
    let hook = receiver
        .with_native_state::<NativeExtension, _, _>(|e| e.hook.get())
        .map_err(QuoinError::Other)?;
    if let Some(block) = hook.and_then(|pin| vm.pins.get(pin))
        && let Err(e) = vm.call_method(mc, block, "value:", vec![receiver])
    {
        // Swallowing the hook's throw natively: clear the in-flight exception
        // VALUE too, or a later `catch:` anywhere would be handed the residue.
        vm.exceptions.active = None;
        receiver
            .with_native_state_mut::<NativeExtension, _, _>(mc, |ext| {
                ext.kill_now();
                ext.dead = true;
            })
            .map_err(QuoinError::Other)?;
        life_for_hook.emit_died(
            PeerDeathReason::Exited,
            &format!("restart hook failed: {e}"),
        );
        return Err(QuoinError::Other(format!(
            "`{peer}` restart hook (incarnation {next}): {e}"
        )));
    }
    Ok(())
}

/// Attach a supervision policy to a live extension (SUPERVISION.md slice 3):
/// store it, arm the exit watch — supervised children get watches, §5 — and
/// spawn the supervisor task. Shared by `e.supervise:` and the package
/// loader's `quoin.toml [extension]` keys.
pub(crate) fn attach_extension_policy<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    receiver: Value<'gc>,
    policy: crate::runtime::supervise::SupervisePolicy,
    what: &str,
) -> Result<(), QuoinError> {
    let (already, life_idx, peer) = receiver
        .with_native_state_mut::<NativeExtension, _, _>(mc, |e| {
            let already = e.policy.is_some();
            if !already {
                e.policy = Some(policy);
            }
            (already, e.life_idx, e.stats.borrow().peer.clone())
        })
        .map_err(QuoinError::Other)?;
    if already {
        return Err(QuoinError::Other(format!(
            "{what}: `{peer}` is already supervised"
        )));
    }
    arm_exit_watch(vm, mc, life_idx)?;
    let boot = resolve_global(vm, "SuperviseBoot").ok_or_else(|| {
        QuoinError::Other(format!("{what}: SuperviseBoot is not installed (qnlib)"))
    })?;
    vm.call_method(mc, boot, "extension:", vec![receiver])?;
    Ok(())
}

/// Arm the pid exit watch for the peer at `life_idx` (idempotent per sink):
/// idle deaths must wake the events pump and the supervisor alike.
fn arm_exit_watch<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    life_idx: usize,
) -> Result<(), QuoinError> {
    let sink = vm.io.lives.borrow().get(life_idx).cloned();
    if let Some(sink) = sink
        && sink.pid.is_some()
        && !sink.is_terminal()
        && !sink
            .watch_armed
            .swap(true, std::sync::atomic::Ordering::SeqCst)
    {
        let boot = resolve_global(vm, "LifecycleBoot").ok_or_else(|| {
            QuoinError::Other("exit watch: LifecycleBoot is not installed (qnlib)".into())
        })?;
        let idx_val = vm.new_int(mc, life_idx as i64);
        vm.call_method(mc, boot, "watch:", vec![idx_val])?;
    }
    Ok(())
}

/// The per-extension supervisor (the `supervise_service_loop` twin): park on
/// the sink's terminal, run the restart cycle with backoff, re-arm the exit
/// watch for each new incarnation, and set the give-up state when the budget
/// is spent — the package circuit breaker. Extensions have no restart gate:
/// sends in the window fail fast typed (the recorded slice-2 residue).
pub(crate) fn supervise_extension_loop<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    ext: Value<'gc>,
) -> Result<Value<'gc>, QuoinError> {
    let mut deaths: Vec<Instant> = Vec::new();
    loop {
        let (life_idx, policy, recipe, ext_id, inc, peer) = ext
            .with_native_state::<NativeExtension, _, _>(|e| {
                (
                    e.life_idx,
                    e.policy,
                    e.recipe.clone(),
                    e.ext_id,
                    e.incarnation,
                    e.stats.borrow().peer.clone(),
                )
            })
            .map_err(QuoinError::Other)?;
        let Some(policy) = policy else {
            return Ok(vm.new_nil(mc));
        };
        let Some(sink) = vm.io.lives.borrow().get(life_idx).cloned() else {
            return Ok(vm.new_nil(mc));
        };
        let watch_rx = sink.watch();
        match vm.await_io(IoRequest::WorkerRecv(watch_rx))? {
            IoResult::WorkerMsg(_) => {}
            other => {
                return Err(QuoinError::Other(format!(
                    "supervisor: unexpected result {other:?}"
                )));
            }
        }
        match sink.status() {
            crate::runtime::lifecycle::LifeStatus::Stopped(_) => return Ok(vm.new_nil(mc)),
            crate::runtime::lifecycle::LifeStatus::Running => continue,
            crate::runtime::lifecycle::LifeStatus::Died { .. } => {}
        }
        deaths.push(Instant::now());
        let mut attempt: u32 = 0;
        let window = std::time::Duration::from_millis(policy.window_ms);
        let gave_up = loop {
            deaths.retain(|t| t.elapsed() <= window);
            if deaths.len() as u32 > policy.max_restarts {
                break true;
            }
            attempt += 1;
            let delay = policy.delay_ms(attempt);
            if delay > 0 {
                vm.await_io(IoRequest::Sleep { ms: delay })?;
            }
            match extension_restart_attempt(vm, mc, ext, &recipe, ext_id, inc, &peer) {
                Ok(()) => break false,
                Err(_) => deaths.push(Instant::now()),
            }
        };
        if gave_up {
            let _ = ext.with_native_state_mut::<NativeExtension, _, _>(mc, |e| {
                e.gave_up = true;
            });
            sink.gave_up
                .store(true, std::sync::atomic::Ordering::Relaxed);
            return Ok(vm.new_nil(mc));
        }
        // Success: the new incarnation needs its own exit watch.
        let new_idx = ext
            .with_native_state::<NativeExtension, _, _>(|e| e.life_idx)
            .map_err(QuoinError::Other)?;
        arm_exit_watch(vm, mc, new_idx)?;
    }
}

pub fn build_extension_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Extension", Some("Object"))
        .construct_with("use Extension.loadPackage:")
        .class_doc(
            "A connected out-of-process extension: a subprocess speaking the Quoin \
             extension wire, providing classes and operations to the host program. \
             `Extension.loadPackage:` is the managed entry point (spawn per the package's \
             quoin.toml, install its classes under the package namespace, run its \
             init.qn glue) -- `use name:*` does this for you. The handle's `call:with:` \
             family is the raw op surface that package glue builds on. See \
             docs/internal/EXT_PACKAGING.md.",
        )
        // `Extension spawn: '<path-to-binary>'` -> spawn the extension subprocess and connect to
        // it, returning an Extension handle. The unmanaged escape hatch (`EXT_PACKAGING.md` §4):
        // it installs the manifest's `ClassDecl` names *verbatim* (possibly bare globals), unlike
        // the namespace-enforcing `loadPackage:`. Dev/testing; the managed path is `loadPackage:`.
        .class_method("spawn:", |vm, mc, _receiver, args| {
            let bin_path = arg!(args, String, 0).to_string();
            let (ext_val, manifest) = spawn_and_connect(vm, mc, &bin_path, &[], None, None)?;
            // Install each provided class as a host global whose selectors dispatch back to this
            // extension. No `await_io` here, so `ext_val` is never held across a collection point.
            for decl in &manifest {
                vm.install_ext_class(
                    mc,
                    ext_val,
                    &decl.name,
                    &decl.instance_selectors,
                    &decl.class_selectors,
                );
            }
            Ok(ext_val)
        })
        .doc(
            "Spawn the extension binary at the String path, connect to it, and answer the \
             Extension handle. The unmanaged escape hatch for dev/testing: the manifest's \
             class names install VERBATIM as globals (no namespace enforcement); the \
             managed path is `loadPackage:`.",
        )
        .instance_method("events", |vm, mc, receiver, _args| {
            // Catch up with a death nobody observed yet (a dead-idle child is
            // a zombie until someone looks), arm the exit watch ONCE so
            // future idle deaths surface without a call, then hand out the
            // channel. Watch first, channel last: the channel Value must not
            // sit on this frame across the watch spawn's possible yield.
            let life_idx = receiver
                .with_native_state_mut::<NativeExtension, _, _>(mc, |e| {
                    let _ = e.note_if_exited();
                    e.life_idx
                })
                .map_err(QuoinError::Other)?;
            let sink = vm.io.lives.borrow().get(life_idx).cloned();
            if let Some(sink) = sink
                && sink.pid.is_some()
                && !sink.is_terminal()
                && !sink
                    .watch_armed
                    .swap(true, std::sync::atomic::Ordering::SeqCst)
            {
                let boot = resolve_global(vm, "LifecycleBoot").ok_or_else(|| {
                    QuoinError::Other(
                        "lifecycle events: LifecycleBoot is not installed (qnlib)".into(),
                    )
                })?;
                let idx_val = vm.new_int(mc, life_idx as i64);
                vm.call_method(mc, boot, "watch:", vec![idx_val])?;
            }
            crate::runtime::worker::life_events_channel(vm, mc, life_idx)
        })
        .doc(
            "This extension's lifecycle events, as a Channel of Maps -- 'kind' is 'spawned', \
             'stopped' (the handle was dropped), or 'died' (the process is GONE -- with \
             'reason' and 'message'; SUPERVISION.md). The first ask arms an OS exit watch, \
             so an idle crash surfaces here without anyone calling the extension; the \
             channel closes after the terminal event, history is kept from spawn time, and \
             asking twice answers the same channel.",
        )
        .instance_method("restart", |vm, mc, receiver, _args| {
            // Catch up lazy detection, then gate on status: restart only
            // follows a DEATH (SUPERVISION.md §2 — stop means stop).
            let (recipe, life_idx, old_ext_id, inc, peer) = receiver
                .with_native_state_mut::<NativeExtension, _, _>(mc, |e| {
                    let _ = e.note_if_exited();
                    (
                        e.recipe.clone(),
                        e.life_idx,
                        e.ext_id,
                        e.incarnation,
                        e.stats.borrow().peer.clone(),
                    )
                })
                .map_err(QuoinError::Other)?;
            let status = vm.io.lives.borrow().get(life_idx).map(|l| l.status());
            match status {
                Some(crate::runtime::lifecycle::LifeStatus::Died { .. }) => {}
                Some(crate::runtime::lifecycle::LifeStatus::Stopped(_)) => {
                    return Err(QuoinError::Other(format!(
                        "Extension.restart: `{peer}` was STOPPED, not died — stop means \
                         stop (SUPERVISION.md §2)"
                    )));
                }
                _ => {
                    return Err(QuoinError::Other(format!(
                        "Extension.restart: `{peer}` is running — restart only follows a \
                         death"
                    )));
                }
            }
            let supervised = receiver
                .with_native_state::<NativeExtension, _, _>(|e| (e.policy.is_some(), e.gave_up))
                .map_err(QuoinError::Other)?;
            if supervised.1 {
                return Err(QuoinError::Other(format!(
                    "Extension.restart: `{peer}` gave up — permanently dead this process"
                )));
            }
            if supervised.0 {
                return Err(QuoinError::Other(format!(
                    "Extension.restart: `{peer}` is supervised — the policy owns its restarts"
                )));
            }
            extension_restart_attempt(vm, mc, receiver, &recipe, old_ext_id, inc, &peer)?;
            Ok(vm.new_nil(mc))
        })
        .doc(
            "Restart a DEAD extension from its frozen spawn recipe (SUPERVISION.md slice 2) \
             and rebind this handle in place: the same installed classes keep working, new \
             calls reach the fresh process. Refuses while running or stopped (stop means \
             stop), and refuses -- leaving the extension dead but retryable -- if the new \
             process's manifest differs from the installed classes. Instances minted by \
             the dead incarnation are permanently stale (`PeerDiedError` with reason \
             `#staleIncarnation`); recreate them from the current incarnation. \
             `events` re-asked after a restart answers the new incarnation's stream.",
        )
        .instance_method("supervise:", |vm, mc, receiver, args| {
            let arg = *args.first().ok_or_else(|| {
                QuoinError::Other("Extension.supervise: expects a Supervise policy".into())
            })?;
            let parsed =
                crate::runtime::supervise::parse_policy(vm, mc, arg, "Extension.supervise:")?;
            let Some(policy) = parsed else {
                let supervised = receiver
                    .with_native_state::<NativeExtension, _, _>(|e| e.policy.is_some())
                    .map_err(QuoinError::Other)?;
                if supervised {
                    return Err(QuoinError::Other(
                        "Extension.supervise: already supervised — detaching is not \
                         supported (v1)"
                            .into(),
                    ));
                }
                return Ok(vm.new_nil(mc));
            };
            attach_extension_policy(vm, mc, receiver, policy, "Extension.supervise:")?;
            Ok(vm.new_nil(mc))
        })
        .doc(
            "Attach a Supervise policy (SUPERVISION.md): every DEATH of this extension -- \
             crash, kill, idle exit (an exit watch arms with the policy) -- triggers an \
             automatic respawn from the frozen recipe with exponential backoff, until more \
             than the policy's `max` deaths land inside its `window` -- then it GIVES UP \
             permanently (the circuit breaker) and calls raise `PeerDiedError` with reason \
             `#gaveUp`. Stops are instructions, never restarted. Attach once; package \
             extensions attach via `quoin.toml [extension]` keys instead.",
        )
        .instance_method("onRestart:", |vm, mc, receiver, args| {
            let arg = *args.first().ok_or_else(|| {
                QuoinError::Other("Extension.onRestart: expects a Block (or nil to clear)".into())
            })?;
            let (old, life_idx) = receiver
                .with_native_state::<NativeExtension, _, _>(|e| (e.hook.get(), e.life_idx))
                .map_err(QuoinError::Other)?;
            let new = if matches!(arg, Value::Nil) {
                None
            } else if crate::runtime::worker::block_parts(arg).is_some() {
                Some(vm.pins.pin(
                    crate::pin_table::PinOwner {
                        kind: "restart-hook",
                        id: life_idx as u64,
                    },
                    arg,
                ))
            } else {
                return Err(QuoinError::Other(
                    "Extension.onRestart: expects a Block (or nil to clear)".into(),
                ));
            };
            if let Some(pin) = old {
                vm.pins.unpin(pin);
            }
            receiver
                .with_native_state::<NativeExtension, _, _>(|e| e.hook.set(new))
                .map_err(QuoinError::Other)?;
            Ok(vm.new_nil(mc))
        })
        .doc(
            "Install the restart hook: a one-argument Block (it receives this handle) run \
             inside every restart attempt -- supervised or manual -- after the fresh child \
             is up, so user code can re-establish ambient child-side state (configuration, \
             registrations) the death threw away. The hook must be re-runnable; a hook \
             error fails the attempt (the fresh child is killed, a supervisor counts it \
             against the budget). Re-installing replaces the hook; nil clears it.",
        )
        // `Extension loadPackage: '<dir>'` -> load an extension *package* (a folder with an
        // `quoin.toml` launch/identity spec + an optional `init.qn` of Quoin-side glue;
        // `EXT_PACKAGING.md`). Spawns the subprocess per the manifest, installs the provided classes
        // **under the package namespace** (no bare-global pollution), runs `init.qn`, and caches the
        // live extension (idempotent per folder). The managed counterpart to `spawn:`.
        .class_method("loadPackage:", |vm, mc, _receiver, args| {
            let dir = arg!(args, String, 0).to_string();
            load_package(vm, mc, &dir)
        })
        .doc(
            "Load an extension PACKAGE -- a folder with an quoin.toml launch/identity \
             spec and optional init.qn glue: spawn the subprocess per the manifest, install \
             its classes under the package namespace (no bare-global pollution), run \
             init.qn, and cache the live extension (idempotent per folder). `use name:*` \
             resolves a package folder and calls this.",
        )
        // `ext call: '<op>' with: '<arg>'` -> send the `Call`, then service the conversation:
        // a loop of re-entrant host-ops the extension may issue (each answered inline) until it
        // sends the terminal `CallReturn`. Op + arg are strings; the result is a string or a
        // resource handle. No handle arguments.
        .instance_method("call:with:", |vm, mc, receiver, args| {
            let op = arg!(args, String, 0).to_string();
            let argv = arg!(args, String, 1).to_string();
            run_extension_method(vm, mc, receiver, op, argv, Vec::new(), None)
        })
        .doc(
            "The raw call surface: send the op (a String) with a String argument, then \
             service the conversation -- re-entrant host-ops the extension may issue -- \
             until it returns. Answers a String or a resource handle. Package glue normally \
             wraps this; `call:with:args:` / `call:with:data:` pass richer arguments.",
        )
        // `ext call: '<op>' with: '<arg>' args: #( v1 v2 … )` -> like `call:with:`, but also
        // passes typed handle arguments: each `ExtResource` in the list passes its resource id;
        // every other value (a block, string, etc.) is minted a call-local host-value handle.
        .instance_method("call:with:args:", |vm, mc, receiver, args| {
            let op = arg!(args, String, 0).to_string();
            let argv = arg!(args, String, 1).to_string();
            let list = *args.get(2).ok_or_else(|| {
                QuoinError::Other("call:with:args: missing args list".to_string())
            })?;
            let call_args = extract_args(list)?;
            run_extension_method(vm, mc, receiver, op, argv, call_args, None)
        })
        .doc(
            "As `call:with:`, additionally passing a List of typed handle arguments: an \
             extension-backed instance passes its resource id; every other value (a block, \
             a string, ...) is minted a call-local host-value handle the extension can call \
             back into.",
        )
        // `ext call: '<op>' with: '<arg>' data: <value>` -> like `call:with:`, but also passes a
        // structured-value payload (any Quoin value, serialized to a `DataValue` tree). The
        // extension reads it as native structured data; the result may likewise be structured.
        .instance_method("call:with:data:", |vm, mc, receiver, args| {
            let op = arg!(args, String, 0).to_string();
            let argv = arg!(args, String, 1).to_string();
            let data = *args.get(2).ok_or_else(|| {
                QuoinError::Other("call:with:data: missing data value".to_string())
            })?;
            run_extension_method(vm, mc, receiver, op, argv, Vec::new(), Some(data))
        })
        .doc(
            "As `call:with:`, additionally passing one structured payload (any Quoin data \
             value, serialized as a tree) that the extension reads as native structured \
             data; the result may likewise come back structured.",
        )
        // `Extension resourceIdOf: v` -> the extension-assigned instance id behind an
        // extension-backed value (its object-table key; unique within one extension). Identity
        // reflection for package glue: ext-backed instances can't carry Quoin-side state, and a
        // package may overload `==` (e.g. numpy's elementwise comparisons), so this is the only
        // stable per-instance key — numpy's init.qn dedups expression-graph leaves with it.
        .class_method("resourceIdOf:", |vm, mc, _receiver, args| {
            let v = *args.first().ok_or_else(|| {
                QuoinError::Other("Extension.resourceIdOf: expects a value".to_string())
            })?;
            let id = v
                .with_native_state::<NativeExtResource, _, _>(|r| r.resource_id)
                .map_err(|_| QuoinError::TypeError {
                    expected: "an extension-backed instance".to_string(),
                    got: v.type_name().to_string(),
                    msg: "Extension.resourceIdOf: expects an extension-backed instance".to_string(),
                })?;
            Ok(vm.new_int(mc, id as i64))
        })
        .doc(
            "The extension-assigned instance id behind an extension-backed value (its \
             object-table key, unique within one extension). Identity reflection for \
             package glue: ext-backed instances carry no Quoin-side state and a package may \
             overload `==` (numpy's elementwise comparisons, say), so this is the only \
             stable per-instance key.",
        )
}
