//! Internal env-var tuning knobs — for testing and debugging the VM, not
//! user-facing. All knobs are prefixed `QN_`, read once on first use, and cached
//! for the life of the process (so they're cheap to check on hot paths).

use std::sync::OnceLock;

/// True if `name` is set to a truthy value: present and not one of
/// `""` / `"0"` / `"false"` / `"no"` (case-insensitive). This way an explicit
/// `QN_FOO=0` reads as off rather than surprise-enabling the knob.
fn env_flag(name: &str) -> bool {
    match std::env::var(name) {
        Ok(v) => !matches!(v.trim().to_ascii_lowercase().as_str(), "" | "0" | "false" | "no"),
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
