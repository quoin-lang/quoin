//! The C1 compute-offload pool (docs/internal/CONCURRENCY_ARCH.md §4): CPU-bound
//! native ops on DETACHED, owned data run on a small fixed thread pool while
//! the calling task parks exactly like an IO wait — other tasks keep running
//! on the VM thread, and `Async.gather:` over N offloading calls
//! parallelizes N-wide with no new guest API.
//!
//! Eligibility (the C1 analog of AOT refusal): an op offloads iff its inputs
//! detach to owned `Send` data, it makes NO callback into the VM, and its
//! result is plain data. Everything in [`ComputeOp`] is a pure
//! `Vec<u8> -> Result<Vec<u8>, String>` function — an offloaded op is
//! observationally a slow native op, nothing more.
//!
//! The `!Send` bridge: the driver's future set is single-threaded, so the
//! pool never runs a driver future. [`offload`] submits a closure to the
//! pool and hands the driver a trivial local future awaiting an
//! `async-channel` receiver; the pool thread computes on owned data and
//! sends the plain result, and async-io's waker machinery wakes the VM's
//! `block_on` — the same wake path an fd event takes. No arena access
//! off-thread, ever. Cancelling the *await* (task cancel / timeout) drops
//! the receiver; the pool op runs to completion and its send is ignored —
//! the same deliver-and-ignore posture as an aborted blocking DNS lookup.
//!
//! Tunables (`QN_*` convention): `QN_COMPUTE_THREADS` (pool size; default
//! cores − 2, min 1; `0` disables offload entirely — the kill switch) and
//! `QN_COMPUTE_MIN` (bytes; inputs below it run inline — the pool round
//! trip beats small payloads, the same measured-crossover discipline as the
//! numexpr gates; see profiling/compute-offload/notes.md).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};

/// One offloadable job: a LABEL (Debug/stats identity) plus a pure function
/// over inputs the call site already detached. The pool is pure transport —
/// it never learns what the job does — and the `Send + Sync + 'static`
/// bound makes the eligibility rule (owned data, no `Gc`, no VM handles) a
/// COMPILE ERROR rather than a review convention. `Arc<dyn Fn>` rather than
/// `Box<dyn FnOnce>` keeps `IoRequest`'s `Clone` derive honest (a clone
/// re-shares the same pure job; re-running it is semantically fine).
#[derive(Clone)]
pub struct ComputeJob {
    pub label: &'static str,
    run: Arc<dyn Fn() -> Result<ComputeOut, String> + Send + Sync>,
}

impl std::fmt::Debug for ComputeJob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ComputeJob({})", self.label)
    }
}

impl ComputeJob {
    pub fn new(
        label: &'static str,
        run: impl Fn() -> Result<ComputeOut, String> + Send + Sync + 'static,
    ) -> Self {
        Self {
            label,
            run: Arc::new(run),
        }
    }

    /// Run the pure function. Called on a pool thread (or inline by the
    /// mock backend) — must never touch VM state.
    pub fn run(&self) -> Result<ComputeOut, String> {
        (self.run)()
    }
}

/// Plain-data result shapes. Ops are OPEN (any closure); results stay a
/// small CLOSED enum because result shapes are structurally boring — data
/// is data — and keeping them plain preserves `IoResult`'s derives. Grow a
/// variant when a family genuinely needs one (e.g. dtype-tagged buffers for
/// `[Num]`), not per op.
#[derive(Clone, Debug)]
pub enum ComputeOut {
    Bytes(Vec<u8>),
}

/// Pool size: `QN_COMPUTE_THREADS`, default `cores - 2` (leave the VM
/// thread and async-io's reactor breathing room), min 1. `0` = offload
/// disabled (every gated op runs inline).
pub fn threads() -> usize {
    static N: OnceLock<usize> = OnceLock::new();
    *N.get_or_init(|| {
        if let Ok(v) = std::env::var("QN_COMPUTE_THREADS")
            && let Ok(n) = v.parse::<usize>()
        {
            return n;
        }
        std::thread::available_parallelism()
            .map(|n| n.get().saturating_sub(2).max(1))
            .unwrap_or(1)
    })
}

/// Offload threshold in input bytes: `QN_COMPUTE_MIN`, default 262144.
/// The pool round trip is a flat ~10us, so a single op never wins from
/// offloading — the win is OVERLAP — and the default is set where the
/// serial-code tax reaches noise (~3% at 256 KiB for the codec family;
/// profiling/compute-offload/notes.md). Gather-heavy programs with smaller
/// payloads tune it down.
pub fn offload_min() -> usize {
    static N: OnceLock<usize> = OnceLock::new();
    *N.get_or_init(|| {
        std::env::var("QN_COMPUTE_MIN")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(262144)
    })
}

/// True when this input should take the pool (enabled and past the gate).
pub fn should_offload(input_len: usize) -> bool {
    threads() > 0 && input_len >= offload_min()
}

// Counters for the `VM.stats` 'compute' section: jobs submitted to the
// pool, jobs whose result was delivered, and gated ops that ran inline.
static SUBMITTED: AtomicUsize = AtomicUsize::new(0);
static COMPLETED: AtomicUsize = AtomicUsize::new(0);
static INLINE: AtomicUsize = AtomicUsize::new(0);

/// `(submitted, completed, inline)` across the process so far.
pub fn stats() -> (usize, usize, usize) {
    (
        SUBMITTED.load(Ordering::Relaxed),
        COMPLETED.load(Ordering::Relaxed),
        INLINE.load(Ordering::Relaxed),
    )
}

/// Pool jobs submitted but not yet delivered (`VM.ps`).
pub fn in_flight() -> usize {
    let (s, c, _) = stats();
    s.saturating_sub(c)
}

/// Record a gated op that ran inline (below threshold or pool disabled).
pub fn note_inline() {
    INLINE.fetch_add(1, Ordering::Relaxed);
}

type Job = Box<dyn FnOnce() + Send>;

struct Pool {
    tx: mpsc::Sender<Job>,
}

/// The process-wide fixed pool, spawned lazily on first offload. Threads
/// share one queue behind a Mutex (jobs are coarse — buffer-sized — so
/// queue contention is noise) and exit if the sender is ever dropped
/// (process teardown).
fn pool() -> &'static Pool {
    static POOL: OnceLock<Pool> = OnceLock::new();
    POOL.get_or_init(|| {
        let n = threads().max(1);
        let (tx, rx) = mpsc::channel::<Job>();
        let rx = Arc::new(Mutex::new(rx));
        for i in 0..n {
            let rx = Arc::clone(&rx);
            std::thread::Builder::new()
                .name(format!("qn-compute-{i}"))
                .spawn(move || {
                    loop {
                        let job = rx.lock().unwrap().recv();
                        match job {
                            Ok(job) => job(),
                            Err(_) => break,
                        }
                    }
                })
                .expect("spawn compute pool thread");
        }
        Pool { tx }
    })
}

/// Submit `job` to the pool; resolve with its result. The returned future is
/// the driver-local half of the bridge (see the module doc) — it holds no
/// VM state and is safe to drop at any point (cancellation).
pub async fn offload(job: ComputeJob) -> Result<ComputeOut, String> {
    SUBMITTED.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = async_channel::bounded::<Result<ComputeOut, String>>(1);
    let job: Job = Box::new(move || {
        // The receiver may be gone (cancelled await) — deliver-and-ignore.
        let _ = tx.send_blocking(job.run());
    });
    pool()
        .tx
        .send(job)
        .map_err(|_| "compute pool is shut down".to_string())?;
    let out = rx
        .recv()
        .await
        .map_err(|_| "compute pool dropped the result".to_string())?;
    COMPLETED.fetch_add(1, Ordering::Relaxed);
    out
}
