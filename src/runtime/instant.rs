use crate::error::QuoinError;
use crate::ext_sdk::{Host, HostExt};
use crate::runtime::duration::make_duration;
use crate::value::{AnyCollect, NativeClassBuilder, Value};

use gc_arena::collect::Trace;
use jiff::SignedDuration;
use std::any::Any;
use std::time::Instant as StdInstant;

/// Native backing state for an `Instant`: a point on the **monotonic** clock
/// (`std::time::Instant`) — forward-only and unaffected by wall-clock adjustments, for measuring
/// elapsed time. Distinct from a wall-clock `Timestamp` (Phase 2). Copy, no `Gc`, no reap.
#[derive(Debug)]
pub struct NativeInstant(pub StdInstant);

impl AnyCollect for NativeInstant {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn trace_gc<'gc>(&self, _cc: &mut dyn Trace<'gc>) {}
}

fn instant_of(v: Value, who: &str) -> Result<StdInstant, QuoinError> {
    v.with_native_state::<NativeInstant, _, _>(|i| i.0)
        .map_err(|_| QuoinError::TypeError {
            expected: "Instant".to_string(),
            got: "a non-Instant value".to_string(),
            msg: format!("{who} requires an Instant operand"),
        })
}

fn make_instant<'gc>(host: &dyn Host<'gc>, i: StdInstant) -> Value<'gc> {
    let class = host.get_or_create_builtin_class("Instant");
    host.new_native_state(class, NativeInstant(i))
}

/// A std `Duration` (unsigned) as a jiff `SignedDuration`. `subsec_nanos` is always < 1e9 (fits
/// i32); a `secs` overflowing i64 would take ~292 billion years.
fn signed(d: std::time::Duration) -> SignedDuration {
    SignedDuration::new(d.as_secs() as i64, d.subsec_nanos() as i32)
}

pub fn build_instant_class() -> NativeClassBuilder {
    NativeClassBuilder::new("Instant", Some("Object"))
        .construct_with("use Instant.now")
        .class_doc(
            "A point on the monotonic clock, for measuring elapsed time.\n\n\
             Monotonic means forward-only and immune to wall-clock adjustments (NTP steps, \
             manual changes), which makes an Instant the right tool for timing and the wrong \
             one for calendars — it has no date, cannot be serialized, and is only meaningful \
             within the current process. For wall-clock time use `Timestamp`; for zone-aware \
             dates use `DateTime`.\n\n\
             ```\n\
             var t0 = Instant.now\n\
             \"* ... work ...\n\
             t0.elapsed     \"* -> the Duration since t0\n\
             ```",
        )
        // Instant.now -> the current monotonic instant.
        .sdk_class_method("now", |host, _r, _a| {
            Ok(make_instant(host, StdInstant::now()))
        })
        .doc("The current monotonic instant.")
        // Time since this instant (now - self), as a Duration.
        .sdk_instance_method("elapsed", |host, receiver, _args| {
            Ok(make_duration(
                host,
                signed(instant_of(receiver, "elapsed")?.elapsed()),
            ))
        })
        .doc(
            "The time since this instant (now minus the receiver), as a Duration — the \
             everyday stopwatch read.",
        )
        // Instant - Instant -> a signed Duration (positive when the receiver is the later one).
        .sdk_typed_instance_method("-:", &["Instant"], |host, receiver, args| {
            let a = instant_of(receiver, "-:")?;
            let b = instant_of(args[0], "-:")?;
            let sd = match a.checked_duration_since(b) {
                Some(d) => signed(d),
                None => -signed(b.checked_duration_since(a).unwrap_or_default()),
            };
            Ok(make_duration(host, sd))
        })
        .doc(
            "The signed Duration between two Instants — positive when the receiver is the \
             later one.",
        )
        // Only `<:` is native; `>:`/`<=:`/`>=:` derive from it as shared Quoin on Object.
        .sdk_typed_instance_method("<:", &["Instant"], |host, receiver, args| {
            Ok(host.new_bool(instant_of(receiver, "<:")? < instant_of(args[0], "<:")?))
        })
        .doc(
            "Whether the receiver is the earlier instant. Only `<:` is native; `>:` / `<=:` \
             / `>=:` derive from it on Object.",
        )
        // `==:` accepts any argument: a non-Instant is simply unequal (never an error).
        .sdk_instance_method("==:", |host, receiver, args| {
            let a = instant_of(receiver, "==:")?;
            let eq = match args[0].with_native_state::<NativeInstant, _, _>(|i| i.0) {
                Ok(b) => a == b,
                Err(_) => false,
            };
            Ok(host.new_bool(eq))
        })
        .doc(
            "Whether the argument is the very same instant. A non-Instant argument is simply \
             unequal — never an error.",
        )
}
