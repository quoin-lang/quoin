//! Peer lifecycle events (SUPERVISION.md slice 1): one [`LifeSink`] per spawned
//! peer — hosted worker, plain worker, extension — feeding `VM.peers` and the
//! per-peer events channels (`w.events` / `e.events` / `svc.serviceEvents`).
//!
//! Emission is SINGLE-SOURCE at the done-lane producers (the thread wrapper, the
//! process mailbox reader, the extension death seams, the exit watch): parent-side
//! detection seams like `note_service_dead` stay pure cleanup, and a `terminal`
//! flag makes the racing sources (lazy detection vs the exit watch) collapse to
//! one death. Records are plain wire data built at the emitter — producers run on
//! ordinary OS threads, so nothing here may touch the GC arena. Consumers get
//! them through a Quoin channel pumped by `Worker.lifeNext:` (the qnlib
//! `LifecycleBoot` helper), which parks on the ordinary `WorkerRecv` request —
//! the logged wake path, so guarantee 8 holds with no new machinery.
//!
//! The staging lane is bounded: a peer's whole life emits a handful of records,
//! and a consumer that never drains must not grow a queue forever. On overflow
//! the NEWEST record drops and `dropped` counts it (the design doc said
//! drop-oldest; `async_channel` cannot evict, and at this depth for events this
//! rare the difference is theoretical — recorded as a deviation).

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use quoin_ext_proto::DataValue as WireData;

use crate::error::PeerDeathReason;
use crate::worker::WorkerMsg;

/// How deep the staging lane buffers before dropping (and counting) new records.
const STAGING_DEPTH: usize = 64;

/// A peer's current lifecycle state, for `VM.peers`.
#[derive(Debug, Clone)]
pub enum LifeStatus {
    Running,
    /// The peer ended without dying: a worker finishing or failing its body,
    /// `serviceStop`, `terminate`, a dropped extension handle. The message says
    /// which (empty for a plain clean finish).
    Stopped(String),
    /// The peer DIED (SUPERVISION.md §2) — the `PeerDiedError` view of the
    /// same fact.
    Died {
        reason: PeerDeathReason,
        detail: String,
    },
}

/// The per-peer lifecycle sink: status + the staged event records. `Arc`-shared
/// into every producer seam (threads included — everything here is `Send`);
/// registered in `vm.io.lives`, which `VM.peers` and the pump read.
#[derive(Debug)]
pub struct LifeSink {
    /// Peer label for reports — matches the peer's claims/ps label.
    pub label: String,
    /// `"hosted"` (a service worker), `"worker"` (plain), or `"extension"`.
    pub kind: &'static str,
    /// `"thread"` / `"process"` for workers; `"process"` for extensions.
    pub backing: &'static str,
    /// The child pid for process-backed peers (`None` for threads) — what the
    /// extension exit watch (`Worker.lifeWatch:`) watches.
    pub pid: Option<u32>,
    tx: async_channel::Sender<WorkerMsg>,
    /// The pump's end (`Worker.lifeNext:` clones it per park).
    pub rx: async_channel::Receiver<WorkerMsg>,
    status: Mutex<LifeStatus>,
    /// Set by the first terminal (stopped/died); later terminals are the same
    /// fact observed by another seam and are dropped whole — no event, no
    /// status change.
    terminal: AtomicBool,
    /// For extensions: the exit watch task has been armed (armed once, on the
    /// first `events` ask — unwatched peers pay nothing).
    pub watch_armed: AtomicBool,
    dropped: AtomicU64,
}

impl LifeSink {
    pub fn new(
        label: String,
        kind: &'static str,
        backing: &'static str,
        pid: Option<u32>,
    ) -> std::sync::Arc<Self> {
        let (tx, rx) = async_channel::bounded(STAGING_DEPTH);
        let sink = std::sync::Arc::new(LifeSink {
            label,
            kind,
            backing,
            pid,
            tx,
            rx,
            status: Mutex::new(LifeStatus::Running),
            terminal: AtomicBool::new(false),
            watch_armed: AtomicBool::new(false),
            dropped: AtomicU64::new(0),
        });
        sink.push(sink.record("spawned", None, ""));
        sink
    }

    /// The peer ended without dying (clean finish, reported failure, stop,
    /// terminate, handle drop). First terminal wins.
    pub fn emit_stopped(&self, message: &str) {
        if self.terminal.swap(true, Ordering::SeqCst) {
            return;
        }
        *self.status.lock().expect("life status") = LifeStatus::Stopped(message.to_string());
        self.push(self.record("stopped", None, message));
        self.tx.close();
    }

    /// The peer DIED. First terminal wins — a death observed by both the lazy
    /// path and the exit watch emits once.
    pub fn emit_died(&self, reason: PeerDeathReason, detail: &str) {
        if self.terminal.swap(true, Ordering::SeqCst) {
            return;
        }
        *self.status.lock().expect("life status") = LifeStatus::Died {
            reason,
            detail: detail.to_string(),
        };
        self.push(self.record("died", Some(reason), detail));
        self.tx.close();
    }

    pub fn is_terminal(&self) -> bool {
        self.terminal.load(Ordering::SeqCst)
    }

    pub fn status(&self) -> LifeStatus {
        self.status.lock().expect("life status").clone()
    }

    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// One event record, as the wire data the pump rebuilds into a Quoin Map:
    /// `kind` always; `reason` (a death symbol name) and `message` when they
    /// say something.
    fn record(&self, kind: &str, reason: Option<PeerDeathReason>, message: &str) -> WireData {
        let mut fields = vec![
            ("kind".to_string(), WireData::Str(kind.to_string())),
            ("peer".to_string(), WireData::Str(self.label.clone())),
        ];
        if let Some(r) = reason {
            fields.push(("reason".to_string(), WireData::Str(r.symbol().to_string())));
        }
        if !message.is_empty() {
            fields.push(("message".to_string(), WireData::Str(message.to_string())));
        }
        WireData::Map(fields)
    }

    fn push(&self, record: WireData) {
        if self.tx.try_send(WorkerMsg::Data(record)).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// The registry (`vm.io.lives`): every peer this VM spawned, in spawn order.
/// Entries outlive their peer — `VM.peers` is also the post-mortem roster.
pub type LifeRegistry = std::rc::Rc<std::cell::RefCell<Vec<std::sync::Arc<LifeSink>>>>;

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod tests;
