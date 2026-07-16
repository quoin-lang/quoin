//! The supervision policy value (SUPERVISION.md slice 3): plain data the
//! runtime interprets directly — no callback into user code on the death path.
//! Attached POST-SPAWN (`svc.serviceSupervise:` / `e.supervise:` / the
//! `quoin.toml [extension]` keys), which keeps the hosting selector matrix
//! flat and is exactly the attach shape a library strategy uses (§10.1). The
//! qnlib `Supervise` class is the user-facing constructor; this is its parse.

use crate::error::QuoinError;
use crate::value::{ObjectPayload, Value};
use crate::vm::VmState;

/// One peer's restart policy, frozen at attach. Presence means "restart on
/// every death" (§4 rule 1: only death — never errors, never stops); absence
/// is today's fail-fast. Delays double from `backoff_ms` to `cap_ms`; more
/// than `max_restarts` deaths inside `window_ms` is the give-up (§4 rule 7 —
/// the circuit breaker).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SupervisePolicy {
    pub backoff_ms: u64,
    pub cap_ms: u64,
    pub max_restarts: u32,
    pub window_ms: u64,
}

impl Default for SupervisePolicy {
    fn default() -> Self {
        SupervisePolicy {
            backoff_ms: 100,
            cap_ms: 10_000,
            max_restarts: 5,
            window_ms: 60_000,
        }
    }
}

impl SupervisePolicy {
    /// The delay before attempt N (1-based): `backoff * 2^(N-1)`, capped.
    pub fn delay_ms(&self, attempt: u32) -> u64 {
        self.backoff_ms
            .saturating_mul(1u64 << attempt.saturating_sub(1).min(30))
            .min(self.cap_ms)
    }
}

/// Parse a Quoin `Supervise` value into a policy — `None` for `#never`.
/// Reads the value's accessors through ordinary sends, so any object honoring
/// the `Supervise` protocol works (attach-time only; never on the death path).
pub fn parse_policy<'gc>(
    vm: &mut VmState<'gc>,
    mc: &gc_arena::Mutation<'gc>,
    value: Value<'gc>,
    what: &str,
) -> Result<Option<SupervisePolicy>, QuoinError> {
    let restart = vm.call_method(mc, value, "restart", vec![])?;
    let restart = symbol_text(restart).ok_or_else(|| {
        QuoinError::Other(format!(
            "{what}: expects a Supervise policy (its `restart` must be #always or #never)"
        ))
    })?;
    match restart.as_str() {
        "never" => return Ok(None),
        "always" => {}
        other => {
            return Err(QuoinError::Other(format!(
                "{what}: unknown restart mode #{other} (always|never)"
            )));
        }
    }
    let field = |vm: &mut VmState<'gc>, sel: &str, min: i64| -> Result<u64, QuoinError> {
        let v = vm.call_method(mc, value, sel, vec![])?;
        let n = v.as_i64().ok_or_else(|| {
            QuoinError::Other(format!("{what}: the policy's `{sel}` must be an Integer"))
        })?;
        if n < min {
            return Err(QuoinError::Other(format!(
                "{what}: the policy's `{sel}` must be >= {min}"
            )));
        }
        Ok(n as u64)
    };
    let backoff_ms = field(vm, "backoff", 0)?;
    let cap_ms = field(vm, "cap", 1)?;
    let max_restarts = field(vm, "max", 1)? as u32;
    let window_ms = field(vm, "window", 1)?;
    Ok(Some(SupervisePolicy {
        backoff_ms,
        cap_ms,
        max_restarts,
        window_ms,
    }))
}

/// The text of a `#symbol` value, `None` for anything else.
fn symbol_text(v: Value<'_>) -> Option<String> {
    if let Value::Object(o) = v
        && let ObjectPayload::Symbol(s) = &o.borrow().payload
    {
        return Some((**s).clone());
    }
    None
}

#[cfg(test)]
#[path = "supervise_tests.rs"]
mod tests;
