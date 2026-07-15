//! Internal env-var tuning knobs — for testing and debugging the VM, not
//! user-facing. All knobs are prefixed `QN_`, read once on first use, and cached
//! for the life of the process (so they're cheap to check on hot paths).

use std::sync::OnceLock;

/// True if `name` is set to a truthy value: present and not one of
/// `""` / `"0"` / `"false"` / `"no"` (case-insensitive). This way an explicit
/// `QN_FOO=0` reads as off rather than surprise-enabling the knob.
fn env_flag(name: &str) -> bool {
    match std::env::var(name) {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "" | "0" | "false" | "no"
        ),
        Err(_) => false,
    }
}

/// `QN_GC_STRESS`: run the garbage collector as aggressively as possible to flush
/// out rooting bugs (a value reachable only via the Rust stack across a step
/// boundary gets collected and surfaces as a crash or `Nil`). Currently this makes
/// the runner collect on *every* VM step instead of every 10; future GC-stressing
/// behaviour belongs under this same flag. A separate `QN_GC_STEPS=N` interval knob
/// can be added later for finer control. Read once and cached.
pub fn gc_stress() -> bool {
    static FLAG: OnceLock<bool> = OnceLock::new();
    *FLAG.get_or_init(|| env_flag("QN_GC_STRESS"))
}

/// Default seed used when `QN_SCHED_STRESS` is enabled without an explicit one.
const SCHED_STRESS_DEFAULT_SEED: u64 = 0x5EED_5C8E_D000_0001;

/// `QN_SCHED_STRESS`: stress the task scheduler to flush ordering- and state-swap
/// bugs. When enabled, the run/test driver (a) *preempts* the running task at every
/// cooperative-yield boundary instead of running it to its next park — forcing the
/// per-task `save_task_context`/`load_task_context` round-trip on every step — and
/// (b) picks the next ready task at random rather than FIFO, which also randomizes
/// gather-child and I/O-wakeup ordering. The result is a wide sweep of interleavings
/// over the same program.
///
/// Seeded for reproducibility: `QN_SCHED_STRESS=<u64>` uses `<u64>` as the seed; any
/// other truthy value uses [`SCHED_STRESS_DEFAULT_SEED`]. Returns `Some(seed)` when
/// enabled, `None` otherwise. Read once and cached. The existing test suites are
/// expected to stay green across a sweep of seeds.
pub fn sched_stress() -> Option<u64> {
    static SEED: OnceLock<Option<u64>> = OnceLock::new();
    *SEED.get_or_init(|| match std::env::var("QN_SCHED_STRESS") {
        Ok(v) => {
            let trimmed = v.trim();
            if matches!(
                trimmed.to_ascii_lowercase().as_str(),
                "" | "0" | "false" | "no"
            ) {
                None
            } else {
                // A numeric value is taken as the seed; a non-numeric truthy value
                // (e.g. `true`/`yes`) enables stress with the default seed.
                Some(trimmed.parse::<u64>().unwrap_or(SCHED_STRESS_DEFAULT_SEED))
            }
        }
        Err(_) => None,
    })
}

/// Number of VM instructions `run_vm_loop` runs per cooperative-yield boundary. That
/// yield is a coroutine switch back to the driver (which re-enters the GC arena), so
/// paying it per instruction dominates compute-bound runtime; batching amortizes the
/// switch + GC pacing over many steps (~2x on compute-bound programs). I/O parks and
/// guest-fiber yields are unaffected — they suspend deeper in `step`, not via the
/// cooperative yield, so responsiveness is preserved. Forced to 1 under GC- or
/// scheduler-stress so those modes keep collecting/preempting at every step. Override
/// with `QN_BATCH=N`. Read once and cached.
pub fn step_batch() -> u32 {
    if gc_stress() || sched_stress().is_some() {
        return 1;
    }
    static N: OnceLock<u32> = OnceLock::new();
    *N.get_or_init(|| {
        std::env::var("QN_BATCH")
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
            .filter(|&n| n >= 1)
            .unwrap_or(256)
    })
}

/// `QN_BATCH_STATS`: have `run_vm_loop` accumulate per-batch wall time + GC bytes allocated
/// and print a one-line summary on finish — the batch-size tuning harness
/// (`profiling/batch-sweep/`). Off by default; adds two metric reads per batch when on.
pub fn batch_stats() -> bool {
    static FLAG: OnceLock<bool> = OnceLock::new();
    *FLAG.get_or_init(|| env_flag("QN_BATCH_STATS"))
}

/// `QN_EXT_HANDSHAKE_TIMEOUT_MS`: how long to wait for an extension's `GetManifest`
/// reply at spawn time before failing the spawn (default 10000). A silent extension
/// would otherwise park the spawning task forever — the handshake runs before any
/// user `Async.timeout:` can wrap it. Lowered in tests to exercise the timeout fast.
pub fn ext_handshake_timeout_ms() -> u64 {
    static MS: OnceLock<u64> = OnceLock::new();
    *MS.get_or_init(|| {
        std::env::var("QN_EXT_HANDSHAKE_TIMEOUT_MS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .filter(|&n| n >= 1)
            .unwrap_or(10_000)
    })
}

static AOT_ON: OnceLock<bool> = OnceLock::new();

/// AOT compilation of the sealed/typed subset at unit compile
/// (docs/internal/AOT_ARCH.md). Default ON as of v0.3 (the soak); `QN_AOT=0` is the
/// kill switch — the interpreter path is untouched either way (the registry
/// is a pure overlay), so disabling is always safe.
pub fn aot_enabled() -> bool {
    *AOT_ON.get_or_init(|| !std::env::var("QN_AOT").is_ok_and(|v| v == "0"))
}

/// Force AOT off for the whole process, ahead of the first `aot_enabled()` read.
/// `qn check` boots its VM only to populate the checker's class table and never
/// runs user code, so cranelift-compiling stdlib methods for it is pure waste
/// (~25% of a session boot). Must run before any compile consults the gate; a
/// later call (gate already read) is a no-op, which is fail-safe — AOT stays in
/// whatever state the process started in.
pub fn disable_aot_for_process() {
    let _ = AOT_ON.set(false);
}
