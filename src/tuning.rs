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
