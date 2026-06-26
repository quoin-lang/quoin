use crate::arg;
use crate::error::QuoinError;
use crate::runtime::pretty::{PpChild, PpRole, PpShape, PrettyPrint};
use crate::value::{AnyCollect, NativeClassBuilder, Value};
use crate::vm::VmState;

use gc_arena::Mutation;
use gc_arena::collect::Trace;
use jiff::tz::TimeZone;
use std::any::Any;

/// Native backing state for a `TimeZone`: a jiff `TimeZone` (an IANA zone like
/// `America/New_York`, or UTC). `Clone` (Arc-backed), not `Copy`, so it's extracted by cloning.
/// No `Gc` fields / no OS resource — `trace_gc` is empty, no reap.
#[derive(Debug)]
pub struct NativeTimeZone(pub TimeZone);

impl AnyCollect for NativeTimeZone {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn trace_gc<'gc>(&self, _cc: &mut dyn Trace<'gc>) {}
}

pub fn tz_of(v: Value, who: &str) -> Result<TimeZone, QuoinError> {
    v.with_native_state::<NativeTimeZone, _, _>(|t| t.0.clone())
        .map_err(|_| QuoinError::TypeError {
            expected: "TimeZone".to_string(),
            got: "a non-TimeZone value".to_string(),
            msg: format!("{who} requires a TimeZone operand"),
        })
}

pub fn make_time_zone<'gc>(vm: &VmState<'gc>, mc: &Mutation<'gc>, tz: TimeZone) -> Value<'gc> {
    let class = vm.get_or_create_builtin_class(mc, "TimeZone");
    vm.new_native_state(mc, class, NativeTimeZone(tz))
}

/// The IANA name of a zone, or a sensible fallback for an unnamed (fixed-offset) one.
pub(crate) fn zone_name(tz: &TimeZone) -> String {
    tz.iana_name()
        .map(|s| s.to_string())
        .unwrap_or_else(|| "(fixed offset)".to_string())
}

impl PrettyPrint for NativeTimeZone {
    fn pp_shape<'gc>(&self) -> PpShape<'gc> {
        PpShape::Record {
            name: "TimeZone",
            fields: vec![(
                "name".to_string(),
                PpChild::Text(zone_name(&self.0), PpRole::Str),
            )],
        }
    }
}

pub fn build_time_zone_class() -> NativeClassBuilder {
    NativeClassBuilder::new("TimeZone", Some("Object"))
        // TimeZone.of:'America/New_York' — look up an IANA time zone (errors if unknown).
        .typed_class_method("of:", &["String"], |vm, mc, _r, args| {
            let name = arg!(args, String, 0);
            match TimeZone::get(name.as_str()) {
                Ok(tz) => Ok(make_time_zone(vm, mc, tz)),
                Err(_) => Err(QuoinError::ValueError(format!(
                    "TimeZone.of:: unknown time zone: '{}'",
                    name.as_str()
                ))),
            }
        })
        // TimeZone.utc — the UTC zone.
        .class_method("utc", |vm, mc, _r, _a| {
            Ok(make_time_zone(vm, mc, TimeZone::UTC))
        })
        // TimeZone.system — the host's configured local zone (falls back to UTC).
        .class_method("system", |vm, mc, _r, _a| {
            Ok(make_time_zone(vm, mc, TimeZone::system()))
        })
        // The IANA name (e.g. 'America/New_York', 'UTC').
        .instance_method("name", |vm, mc, receiver, _args| {
            Ok(vm.new_string(mc, zone_name(&tz_of(receiver, "name")?)))
        })
        .instance_method("s", |vm, mc, receiver, _args| {
            Ok(vm.new_string(mc, zone_name(&tz_of(receiver, "s")?)))
        })
        // `==:` accepts any argument: a non-TimeZone is simply unequal (never an error).
        .instance_method("==:", |vm, mc, receiver, args| {
            let a = tz_of(receiver, "==:")?;
            let eq = match args[0].with_native_state::<NativeTimeZone, _, _>(|t| t.0.clone()) {
                Ok(b) => a == b,
                Err(_) => false,
            };
            Ok(vm.new_bool(mc, eq))
        })
}
