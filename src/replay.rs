//! The wake log: record/replay hooks for the scheduler (`CONCURRENCY_MODEL.md`
//! guarantee 8, `ACTOR_OBJECTS.md` §8).
//!
//! The driver's behavior is fully determined by three decision streams — which ready
//! task the ready-pop picks, whether a cooperative-yield boundary preempts, and which
//! background completion is delivered when. Everything else in the scheduler is FIFO
//! and epoch-keyed, so recording those streams pins the whole concurrent execution.
//! Every wake path MUST flow through these logged points; an unlogged wake source is
//! a bug even before the full replayer (arc 4) exists — the divergence test in
//! `tests/wake_replay.rs` is the enforcement.
//!
//! Modes (env; all off = a single branch per event site):
//! - `QN_WAKE_LOG=1` — keep a bounded ring of recent wake events, dumped to stderr
//!   when the driver declares a global deadlock. Cheap always-on-able diagnostics.
//! - `QN_WAKE_RECORD=<path>` — record the full event stream to `<path>`.
//! - `QN_WAKE_REPLAY=<path>` — replay: every pick and preempt decision is forced from
//!   the log, and completions are delivered in logged order (early arrivals are held
//!   back). Combine with `QN_WAKE_RECORD` to emit the replayed run's own stream — the
//!   divergence test's comparison artifact.
//!
//! A process drives the scheduler more than once (the stdlib load drives before the
//! program does; the REPL drives per line), so the log file holds one `RUN` section
//! per driver run, recorded and replayed in process order through a process-global
//! cursor. The driver runs themselves are sequential — only their contents need
//! pinning.
//!
//! Scope (slice 1): the main VM only — worker VMs run their own drivers and stay
//! unlogged until the actor-objects convergence gives them identities in the log.
//! Replay re-performs real I/O and forces its delivery ORDER; result payloads are
//! fingerprinted so content divergence is reported, not silently absorbed. Injecting
//! recorded results instead of re-performing (replay of programs with genuinely
//! nondeterministic inputs) is the arc-4 replayer, layered behind these same hooks.
//! Record and replay must run under the same execution env — the yield cadence
//! (`QN_BATCH`, forced to 1 by stress modes) is validated via the log header.

use std::collections::VecDeque;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// Ring capacity for the `QN_WAKE_LOG` diagnostic mode.
const RING_CAP: usize = 512;

/// One scheduler decision or delivery. `usize` task ids (not `TaskId`) keep this
/// module independent of the VM types; the driver owns the conversion.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WakeEvent {
    /// The ready-pop chose this task.
    Pick { tid: usize },
    /// A cooperative-yield boundary decided to preempt the running task (or not).
    /// Logged at every boundary so replay never has to guess where yields fell.
    Rotate { preempt: bool },
    /// An I/O completion was delivered to `tid`. `aborted` = the op was cancelled;
    /// otherwise `hash` fingerprints the result payload.
    Io {
        tid: usize,
        aborted: bool,
        hash: u64,
    },
    /// A deadline timer firing was delivered. Stale firings are logged too —
    /// win/lose is resolved deterministically downstream by the epoch.
    Deadline {
        tid: usize,
        target: usize,
        epoch: u64,
    },
}

impl WakeEvent {
    pub fn is_delivery(&self) -> bool {
        matches!(self, WakeEvent::Io { .. } | WakeEvent::Deadline { .. })
    }

    fn to_line(self) -> String {
        match self {
            WakeEvent::Pick { tid } => format!("P {tid}"),
            WakeEvent::Rotate { preempt } => format!("R {}", preempt as u8),
            WakeEvent::Io { tid, aborted, hash } => {
                format!("I {tid} {} {hash:016x}", aborted as u8)
            }
            WakeEvent::Deadline { tid, target, epoch } => format!("D {tid} {target} {epoch}"),
        }
    }

    fn parse(line: &str) -> Option<WakeEvent> {
        let mut f = line.split_ascii_whitespace();
        let ev = match f.next()? {
            "P" => WakeEvent::Pick {
                tid: f.next()?.parse().ok()?,
            },
            "R" => WakeEvent::Rotate {
                preempt: f.next()? == "1",
            },
            "I" => WakeEvent::Io {
                tid: f.next()?.parse().ok()?,
                aborted: f.next()? == "1",
                hash: u64::from_str_radix(f.next()?, 16).ok()?,
            },
            "D" => WakeEvent::Deadline {
                tid: f.next()?.parse().ok()?,
                target: f.next()?.parse().ok()?,
                epoch: f.next()?.parse().ok()?,
            },
            _ => return None,
        };
        f.next().is_none().then_some(ev)
    }
}

/// Fingerprint a result payload via its `Debug` rendering. `DefaultHasher::new()`
/// uses fixed keys, so the hash is stable across the processes of a record/replay
/// pair (same binary — the contract; it is NOT stable across compiler versions).
pub fn hash_debug(v: &impl std::fmt::Debug) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    format!("{v:?}").hash(&mut h);
    h.finish()
}

/// Parse a whole log file into per-driver-run sections, validating the header —
/// the yield cadence (`batch`) changes where `Rotate` events fall, so it must match.
fn parse_log(text: &str, current_batch: u32) -> Result<VecDeque<Vec<WakeEvent>>, String> {
    let mut lines = text.lines();
    let header = lines.next().unwrap_or_default();
    let mut parts = header.split_ascii_whitespace();
    if parts.next() != Some("qn-wake-log") || parts.next() != Some("v1") {
        return Err("QN_WAKE_REPLAY: not a qn-wake-log v1 file".to_string());
    }
    let batch: Option<u32> = parts
        .find_map(|p| p.strip_prefix("batch="))
        .and_then(|v| v.parse().ok());
    if batch != Some(current_batch) {
        return Err(format!(
            "QN_WAKE_REPLAY: log was recorded with batch={} but this run has batch={} — \
             yield boundaries will not line up; run replay under the same \
             QN_BATCH/QN_SCHED_STRESS/QN_GC_STRESS settings as the recording",
            batch.map_or("?".to_string(), |b| b.to_string()),
            current_batch
        ));
    }
    let mut sections: VecDeque<Vec<WakeEvent>> = VecDeque::new();
    for (i, line) in lines.enumerate() {
        if line.is_empty() {
            continue;
        }
        if line == "RUN" {
            sections.push_back(Vec::new());
            continue;
        }
        let ev = WakeEvent::parse(line)
            .ok_or_else(|| format!("QN_WAKE_REPLAY: unparseable log line {}: {line:?}", i + 2))?;
        sections
            .back_mut()
            .ok_or_else(|| "QN_WAKE_REPLAY: event before the first RUN marker".to_string())?
            .push(ev);
    }
    Ok(sections)
}

/// Process-global state: driver runs are sequential, and record/replay must pair
/// them up in process order across the runs' individual `ReplayCtx`s.
#[derive(Default)]
struct GlobalWake {
    /// Record mode: sections flushed so far; the file is rewritten whole at each
    /// run's end (so an error exit still leaves a complete, usable log).
    recorded: Vec<Vec<WakeEvent>>,
    /// Replay mode: the recorded sections not yet claimed by a driver run.
    pending: Option<VecDeque<Vec<WakeEvent>>>,
}

fn global() -> &'static Mutex<GlobalWake> {
    static GLOBAL: OnceLock<Mutex<GlobalWake>> = OnceLock::new();
    GLOBAL.get_or_init(|| Mutex::new(GlobalWake::default()))
}

/// The full-stream recorder for one driver run; its section joins the global log
/// (and the file is rewritten) when the `ReplayCtx` drops.
struct Recorder {
    path: PathBuf,
    events: Vec<WakeEvent>,
}

/// The log section a replay run consumes front to back.
#[derive(Debug)]
struct Replayer {
    events: Vec<WakeEvent>,
    pos: usize,
}

impl Replayer {
    fn peek(&self) -> Option<WakeEvent> {
        self.events.get(self.pos).copied()
    }
}

/// The driver's per-run logging/replay state. All-`None` (the default) costs one
/// branch per event site.
#[derive(Default)]
pub struct ReplayCtx {
    ring: Option<VecDeque<WakeEvent>>,
    recorder: Option<Recorder>,
    replayer: Option<Replayer>,
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

fn env_flag(name: &str) -> bool {
    match std::env::var(name) {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "" | "0" | "false" | "no"
        ),
        Err(_) => false,
    }
}

impl ReplayCtx {
    /// Build from the environment. Worker VMs pass `is_worker` and get the inert
    /// context — their wake events join the log with the actor-objects convergence.
    pub fn from_env(is_worker: bool) -> Result<ReplayCtx, String> {
        if is_worker {
            return Ok(ReplayCtx::default());
        }
        let mut ctx = ReplayCtx::default();
        if env_flag("QN_WAKE_LOG") {
            ctx.ring = Some(VecDeque::with_capacity(RING_CAP));
        }
        if let Some(path) = env_path("QN_WAKE_RECORD") {
            ctx.recorder = Some(Recorder {
                path,
                events: Vec::new(),
            });
        }
        if let Some(path) = env_path("QN_WAKE_REPLAY") {
            let mut g = global().lock().expect("wake-log global poisoned");
            if g.pending.is_none() {
                let text = std::fs::read_to_string(&path)
                    .map_err(|e| format!("QN_WAKE_REPLAY: cannot read {}: {e}", path.display()))?;
                g.pending = Some(parse_log(&text, crate::tuning::step_batch())?);
            }
            let section = g
                .pending
                .as_mut()
                .expect("pending sections initialized above")
                .pop_front()
                .ok_or_else(|| {
                    "QN_WAKE_REPLAY: no recorded section left for this driver run — \
                     the recording process drove the scheduler fewer times"
                        .to_string()
                })?;
            ctx.replayer = Some(Replayer {
                events: section,
                pos: 0,
            });
        }
        Ok(ctx)
    }

    /// True when events should be logged (ring or recorder active). Replay runs
    /// usually record too — that is how the divergence test gets its second stream.
    pub fn logging(&self) -> bool {
        self.ring.is_some() || self.recorder.is_some()
    }

    pub fn replaying(&self) -> bool {
        self.replayer.is_some()
    }

    /// Append an event to the ring and/or recorder. Never touches the replayer.
    pub fn log(&mut self, ev: WakeEvent) {
        if let Some(ring) = &mut self.ring {
            if ring.len() == RING_CAP {
                ring.pop_front();
            }
            ring.push_back(ev);
        }
        if let Some(rec) = &mut self.recorder {
            rec.events.push(ev);
        }
    }

    /// Replay: the next event, if any.
    pub fn peek(&self) -> Option<WakeEvent> {
        self.replayer.as_ref().and_then(|r| r.peek())
    }

    /// Replay: the next event when it is a delivery (`Io`/`Deadline`), else `None`.
    pub fn peek_delivery(&self) -> Option<WakeEvent> {
        self.peek().filter(WakeEvent::is_delivery)
    }

    /// Replay: advance past the current event (after acting on a peeked one).
    pub fn consume(&mut self) {
        if let Some(r) = &mut self.replayer {
            r.pos += 1;
        }
    }

    /// Replay: the next event must be a `Pick`; consume it and return the task id.
    pub fn expect_pick(&mut self) -> Result<usize, String> {
        match self.peek() {
            Some(WakeEvent::Pick { tid }) => {
                self.consume();
                Ok(tid)
            }
            other => Err(self.divergence_msg(&format!(
                "the scheduler is picking a ready task, but the log has {other:?}"
            ))),
        }
    }

    /// Replay: the next event must be a `Rotate`; consume it and return the decision.
    pub fn expect_rotate(&mut self) -> Result<bool, String> {
        match self.peek() {
            Some(WakeEvent::Rotate { preempt }) => {
                self.consume();
                Ok(preempt)
            }
            other => Err(self.divergence_msg(&format!(
                "the scheduler is at a yield boundary, but the log has {other:?}"
            ))),
        }
    }

    pub fn divergence_msg(&self, what: &str) -> String {
        let pos = self.replayer.as_ref().map_or(0, |r| r.pos);
        format!("wake-log replay divergence at event {pos}: {what}")
    }

    /// Dump the diagnostic ring to stderr (the `QN_WAKE_LOG` consumer — called when
    /// the driver declares a global deadlock).
    pub fn dump_ring(&self, why: &str) {
        if let Some(ring) = &self.ring {
            eprintln!("wake log ({why}; {} events, most recent last):", ring.len());
            for ev in ring {
                eprintln!("  {}", ev.to_line());
            }
        }
    }
}

impl Drop for ReplayCtx {
    fn drop(&mut self) {
        if let Some(rec) = self.recorder.take() {
            let mut g = global().lock().expect("wake-log global poisoned");
            g.recorded.push(rec.events);
            let stress = crate::tuning::sched_stress().map_or("off".to_string(), |s| s.to_string());
            let mut out = format!(
                "qn-wake-log v1 batch={} stress={stress}\n",
                crate::tuning::step_batch()
            );
            for section in &g.recorded {
                out.push_str("RUN\n");
                for ev in section {
                    out.push_str(&ev.to_line());
                    out.push('\n');
                }
            }
            if let Err(e) = std::fs::write(&rec.path, &out) {
                eprintln!(
                    "QN_WAKE_RECORD: failed to write {}: {e}",
                    rec.path.display()
                );
            }
        }
        if let Some(rep) = &self.replayer {
            let left = rep.events.len() - rep.pos;
            if left > 0 {
                eprintln!(
                    "QN_WAKE_REPLAY: driver run ended with {left} log events unconsumed \
                     (the recorded run went further)"
                );
            }
        }
    }
}

#[cfg(test)]
#[path = "replay_tests.rs"]
mod tests;
