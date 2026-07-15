//! Why-interpreted bookkeeping: refusal kinds and the process-lifetime
//! compile/refusal counters behind `VM.stats`' aotRefusals view.

use super::*;

/// Coarse buckets for WHY a member stayed interpreted — stable keys for the
/// `VM.stats` counters (the free-form `why` string carries the details, and
/// stays free-form precisely so these keys can be stable). The `Precheck*`
/// kinds are candidacy skips: the member never reached the translator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RefusalKind {
    UnsupportedInstruction,
    UnsupportedConstant,
    /// `^^` inside a compiled block template (the recorded "template-^^" gap).
    NlrTemplate,
    /// `^^` meeting a catch-family send.
    NlrCatch,
    /// A `^^`-carrying closure escaping the compiled scope.
    NlrEscape,
    /// Per-iteration / guarded-nest materialization heuristics.
    MaterializationGate,
    /// Own-selector (recursive) materialization — the makeTree gate.
    RecursionGate,
    /// Write-capturing closure shapes (shared siblings, escapes, param/self writes).
    WriteCapture,
    /// A fused-combinator receiver or element shape that can't be proven.
    UnprovenReceiver,
    /// A value that must be slot-resident but isn't (self/nil at boundaries, cold stubs).
    SlotResidency,
    /// Local/return typing: unknown local, kind change, unprovable scalar.
    LocalTyping,
    /// The compiled ABI's 8-wide argument / list-literal caps.
    ArityCap,
    /// Structural bytecode limits (jump range, merges, underflow) — and the
    /// default for untagged helper errors.
    Structural,
    /// Candidacy skip: multi-variant (typed multimethod) selector.
    PrecheckMultiVariant,
    /// Candidacy skip: guard/decl-block member.
    PrecheckDeclBlock,
    /// Candidacy skip: a parameter/return shape with no scalar/Obj mapping.
    PrecheckSignature,
    /// Candidacy skip: block shape (>1 param, named, init-literal config,
    /// nested block literal, `^^` inside).
    PrecheckBlockShape,
}

impl RefusalKind {
    /// The stable camelCase key this bucket counts under in `VM.stats`.
    pub fn name(self) -> &'static str {
        match self {
            RefusalKind::UnsupportedInstruction => "unsupportedInstruction",
            RefusalKind::UnsupportedConstant => "unsupportedConstant",
            RefusalKind::NlrTemplate => "nlrTemplate",
            RefusalKind::NlrCatch => "nlrCatch",
            RefusalKind::NlrEscape => "nlrEscape",
            RefusalKind::MaterializationGate => "materializationGate",
            RefusalKind::RecursionGate => "recursionGate",
            RefusalKind::WriteCapture => "writeCapture",
            RefusalKind::UnprovenReceiver => "unprovenReceiver",
            RefusalKind::SlotResidency => "slotResidency",
            RefusalKind::LocalTyping => "localTyping",
            RefusalKind::ArityCap => "arityCap",
            RefusalKind::Structural => "structural",
            RefusalKind::PrecheckMultiVariant => "precheckMultiVariant",
            RefusalKind::PrecheckDeclBlock => "precheckDeclBlock",
            RefusalKind::PrecheckSignature => "precheckSignature",
            RefusalKind::PrecheckBlockShape => "precheckBlockShape",
        }
    }

    /// True for candidacy skips (`VM.stats` counts them as 'skipped', not 'refused').
    pub fn is_precheck(self) -> bool {
        matches!(
            self,
            RefusalKind::PrecheckMultiVariant
                | RefusalKind::PrecheckDeclBlock
                | RefusalKind::PrecheckSignature
                | RefusalKind::PrecheckBlockShape
        )
    }
}

/// A translation refusal traveling out of the bytecode walk: the coarse bucket
/// plus the human-readable detail. `From<String>`/`From<&str>` default to
/// [`RefusalKind::Structural`] so incidental helper errors (`ok_or("stack
/// underflow")?`) keep composing; every deliberate refusal site tags its kind.
#[derive(Debug, Clone)]
pub struct Refusal {
    pub kind: RefusalKind,
    pub why: String,
}

impl From<String> for Refusal {
    fn from(why: String) -> Self {
        Refusal {
            kind: RefusalKind::Structural,
            why,
        }
    }
}

impl From<&str> for Refusal {
    fn from(why: &str) -> Self {
        Refusal {
            kind: RefusalKind::Structural,
            why: why.to_string(),
        }
    }
}

/// One recorded refusal or candidacy skip, for `VM.stats` / `VM.aotRefusals`.
#[derive(Debug, Clone)]
pub struct RefusalRecord {
    pub selector: String,
    pub kind: RefusalKind,
    pub why: String,
}

/// The process-lifetime refusal/skip log behind `VM.stats`. Bounded (a
/// pathological compile loop must not grow it without limit); appended on
/// final outcomes only — demote-retries that eventually compile never land
/// here. Reads dedup by (selector, kind, why): units recompile (REPL lines,
/// speculative re-attempts), and "distinct members refused" is the honest
/// statistic.
static REFUSAL_LOG: std::sync::Mutex<Vec<RefusalRecord>> = std::sync::Mutex::new(Vec::new());
const REFUSAL_LOG_CAP: usize = 4096;

/// Record one refusal (translator) or skip (candidacy pre-check).
pub fn record_refusal(selector: &str, kind: RefusalKind, why: &str) {
    let mut log = REFUSAL_LOG.lock().unwrap();
    if log.len() < REFUSAL_LOG_CAP {
        log.push(RefusalRecord {
            selector: selector.to_string(),
            kind,
            why: why.to_string(),
        });
    }
}

/// A deduplicated snapshot of the refusal/skip log (see [`REFUSAL_LOG`]).
/// The currently-registered compiled entries as `(selector, role)` — the
/// positive mirror of `refusal_snapshot`, behind `VM.aotCompiled`. Reflects
/// LIVE state: a tombstoned entry drops out, exactly as it stopped being
/// dispatched to.
pub fn compiled_snapshot() -> Vec<(String, AotRole)> {
    let reg = registry().read().unwrap();
    let mut out: Vec<(String, AotRole)> =
        reg.values().map(|e| (e.selector.clone(), e.role)).collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

pub fn refusal_snapshot() -> Vec<RefusalRecord> {
    let log = REFUSAL_LOG.lock().unwrap();
    let mut seen = HashSet::new();
    log.iter()
        .filter(|r| seen.insert((r.selector.clone(), r.kind, r.why.clone())))
        .cloned()
        .collect()
}

/// How `compile_candidates` fared, for logs/tests (`VM.stats` reads the
/// process-lifetime aggregates instead: `compile_totals` + `refusal_snapshot`).
#[derive(Default, Debug)]
pub struct CompileStats {
    pub compiled: usize,
    pub refused: Vec<RefusalRecord>,
}

/// Process-lifetime compile/refusal counters. Every `compile_candidates`
/// caller used to drop its `CompileStats`, so the ONLY record that a
/// candidate silently fell out of compilation was an env-gated eprintln —
/// no way to notice a coverage regression. `QN_AOT_STATS=1` surfaces these.
pub(crate) static TOTAL_COMPILED: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);
pub(crate) static TOTAL_REFUSED: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// `(compiled, refused)` across the process so far.
pub fn compile_totals() -> (usize, usize) {
    use std::sync::atomic::Ordering;
    (
        TOTAL_COMPILED.load(Ordering::Relaxed),
        TOTAL_REFUSED.load(Ordering::Relaxed),
    )
}
