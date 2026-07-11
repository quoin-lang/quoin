use crate::arg;
use crate::error::QuoinError;
use crate::ext_sdk::{Host, HostExt};
use crate::runtime::duration::make_duration;
use crate::runtime::pretty::{PpChild, PpShape, PrettyPrint};
use crate::value::{AnyCollect, NativeClassBuilder, Value};

use gc_arena::collect::Trace;
use jiff::{SignedDuration, Span};
use std::any::Any;

/// Native backing state for a `Span`: a mixed-unit, calendar-aware duration (jiff `Span` —
/// each unit is a separate field, so "1 month" stays "1 month" until it meets a date).
/// Plain `Copy` data — no `Gc` fields, no OS resource — so `trace_gc` is empty.
#[derive(Debug)]
pub struct NativeSpan(pub Span);

impl AnyCollect for NativeSpan {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn trace_gc<'gc>(&self, _cc: &mut dyn Trace<'gc>) {}
}

impl PrettyPrint for NativeSpan {
    fn pp_shape<'gc>(&self) -> PpShape<'gc> {
        // Only the units the span actually carries; the zero span shows none.
        let s = self.0;
        let units: [(&str, i64); 10] = [
            ("years", s.get_years() as i64),
            ("months", s.get_months() as i64),
            ("weeks", s.get_weeks() as i64),
            ("days", s.get_days() as i64),
            ("hours", s.get_hours() as i64),
            ("minutes", s.get_minutes()),
            ("seconds", s.get_seconds()),
            ("milliseconds", s.get_milliseconds()),
            ("microseconds", s.get_microseconds()),
            ("nanoseconds", s.get_nanoseconds()),
        ];
        PpShape::Record {
            name: "Span",
            fields: units
                .into_iter()
                .filter(|(_, n)| *n != 0)
                .map(|(u, n)| (u.to_string(), PpChild::Val(Value::Int(n))))
                .collect(),
        }
    }
}

/// The jiff `Span` behind a `Span` value (the receiver, or — for the typed operators — a
/// same-typed operand). Errors clearly if `v` is not a `Span`.
pub fn span_of(v: Value, who: &str) -> Result<Span, QuoinError> {
    v.with_native_state::<NativeSpan, _, _>(|s| s.0)
        .map_err(|_| QuoinError::TypeError {
            expected: "Span".to_string(),
            got: "a non-Span value".to_string(),
            msg: format!("{who} requires a Span operand"),
        })
}

pub fn make_span<'gc>(host: &dyn Host<'gc>, s: Span) -> Value<'gc> {
    let class = host.get_or_create_builtin_class("Span");
    host.new_native_state(class, NativeSpan(s))
}

/// A single-unit constructor body: build the span through jiff's fallible setter, mapping an
/// out-of-range count to a clean ArithmeticError.
fn unit_span(
    set: impl FnOnce(Span, i64) -> Result<Span, jiff::Error>,
    n: i64,
    who: &str,
) -> Result<Span, QuoinError> {
    set(Span::new(), n).map_err(|e| QuoinError::ArithmeticError(format!("Span {who} {e}")))
}

/// Field-wise combination — the `+:`/`-:` semantics. Each unit adds (or subtracts)
/// independently: `1y + 2mo = 1y 2mo`, and `1h + 60m` stays `1h 60m` — no unit ever converts
/// into another, because without a calendar there is no correct conversion. Per-unit
/// overflow of jiff's span limits is an ArithmeticError.
fn fieldwise(a: Span, b: Span, sub: bool, who: &str) -> Result<Span, QuoinError> {
    let combine = |x: i64, y: i64| -> Result<i64, QuoinError> {
        let y = if sub { y.checked_neg() } else { Some(y) }
            .ok_or_else(|| QuoinError::ArithmeticError(format!("Span {who} overflow")))?;
        x.checked_add(y)
            .ok_or_else(|| QuoinError::ArithmeticError(format!("Span {who} overflow")))
    };
    let years = combine(a.get_years() as i64, b.get_years() as i64)?;
    let months = combine(a.get_months() as i64, b.get_months() as i64)?;
    let weeks = combine(a.get_weeks() as i64, b.get_weeks() as i64)?;
    let days = combine(a.get_days() as i64, b.get_days() as i64)?;
    let hours = combine(a.get_hours() as i64, b.get_hours() as i64)?;
    let minutes = combine(a.get_minutes(), b.get_minutes())?;
    let seconds = combine(a.get_seconds(), b.get_seconds())?;
    let millis = combine(a.get_milliseconds(), b.get_milliseconds())?;
    let micros = combine(a.get_microseconds(), b.get_microseconds())?;
    let nanos = combine(a.get_nanoseconds(), b.get_nanoseconds())?;
    Span::new()
        .try_years(years)
        .and_then(|s| s.try_months(months))
        .and_then(|s| s.try_weeks(weeks))
        .and_then(|s| s.try_days(days))
        .and_then(|s| s.try_hours(hours))
        .and_then(|s| s.try_minutes(minutes))
        .and_then(|s| s.try_seconds(seconds))
        .and_then(|s| s.try_milliseconds(millis))
        .and_then(|s| s.try_microseconds(micros))
        .and_then(|s| s.try_nanoseconds(nanos))
        .map_err(|e| QuoinError::ArithmeticError(format!("Span {who} {e}")))
}

pub fn build_span_class() -> NativeClassBuilder {
    let b = NativeClassBuilder::new("Span", Some("Object"))
        .construct_with("use Span.parse: / Span.years: / Span.days: / …")
        .class_doc(
            "A mixed-unit, calendar-aware duration: years, months, weeks, days, and time \
             units held as SEPARATE fields — \"1 month\" stays 1 month until it meets a \
             date, where `Date`/`DateTime` arithmetic applies it correctly (end-of-month \
             clamping, DST). Contrast `Duration`, which is a fixed length of time.\n\n\
             Parse ISO 8601 (`'P1Y2M3D'`) or the friendly form (`'1y 2mo'`); combine with \
             `+:` / `-:` field-wise. Equality is FIELDWISE: `1h` ≠ `60m` — whether they're \
             the same depends on a calendar, so Span refuses to guess (and has no `<:` for \
             the same reason).\n\n\
             ```\n\
             (Span.parse:'P1Y2M').s                     \"* -> 1y 2mo\n\
             ((Span.years:1) + (Span.months:2)).iso8601 \"* -> 'P1Y2M'\n\
             ```",
        )
        .sdk_class_method("zero", |host, _r, _a| Ok(make_span(host, Span::new())))
        .doc("The empty Span — every unit zero; the identity for `+:`.")
        .sdk_typed_class_method("parse:", &["String"], |host, _r, args| {
            let s = arg!(args, String, 0);
            s.parse::<Span>()
                .map(|sp| make_span(host, sp))
                .map_err(|e| QuoinError::ValueError(format!("Span parse: {e}")))
        })
        .doc(
            "The Span an ISO 8601 duration (`'P1Y2M3DT4H'`) or friendly form (`'1y 2mo'`) \
             denotes. Not parseable → ValueError.\n\n\
             ```\n\
             (Span.parse:'P1Y2M3D').days       \"* -> 3\n\
             (Span.parse:'1h 30m').minutes     \"* -> 30\n\
             ```",
        )
        .sdk_typed_class_method("years:", &["Integer"], |host, _r, args| {
            Ok(make_span(
                host,
                unit_span(|s, n| s.try_years(n), arg!(args, Int, 0), "years:")?,
            ))
        })
        .doc("A Span of exactly n calendar years (may be negative).")
        .sdk_typed_class_method("months:", &["Integer"], |host, _r, args| {
            Ok(make_span(
                host,
                unit_span(|s, n| s.try_months(n), arg!(args, Int, 0), "months:")?,
            ))
        })
        .doc("A Span of exactly n calendar months (may be negative).")
        .sdk_typed_class_method("weeks:", &["Integer"], |host, _r, args| {
            Ok(make_span(
                host,
                unit_span(|s, n| s.try_weeks(n), arg!(args, Int, 0), "weeks:")?,
            ))
        })
        .doc("A Span of exactly n weeks (may be negative).")
        .sdk_typed_class_method("days:", &["Integer"], |host, _r, args| {
            Ok(make_span(
                host,
                unit_span(|s, n| s.try_days(n), arg!(args, Int, 0), "days:")?,
            ))
        })
        .doc(
            "A Span of exactly n calendar days (may be negative). A calendar day is not \
             always 24 hours — applying it across a DST change keeps the wall-clock \
             time.\n\n\
             ```\n\
             (Span.days:3).iso8601     \"* -> 'P3D'\n\
             ```",
        )
        .sdk_typed_class_method("hours:", &["Integer"], |host, _r, args| {
            Ok(make_span(
                host,
                unit_span(|s, n| s.try_hours(n), arg!(args, Int, 0), "hours:")?,
            ))
        })
        .doc("A Span of exactly n hours (may be negative).")
        .sdk_typed_class_method("minutes:", &["Integer"], |host, _r, args| {
            Ok(make_span(
                host,
                unit_span(|s, n| s.try_minutes(n), arg!(args, Int, 0), "minutes:")?,
            ))
        })
        .doc("A Span of exactly n minutes (may be negative).")
        .sdk_typed_class_method("seconds:", &["Integer"], |host, _r, args| {
            Ok(make_span(
                host,
                unit_span(|s, n| s.try_seconds(n), arg!(args, Int, 0), "seconds:")?,
            ))
        })
        .doc("A Span of exactly n seconds (may be negative).");
    // Field-wise arithmetic and fieldwise equality (see the class doc for why).
    let b = b
        .sdk_typed_instance_method("+:", &["Span"], |host, receiver, args| {
            let s = fieldwise(
                span_of(receiver, "+:")?,
                span_of(args[0], "+:")?,
                false,
                "+:",
            )?;
            Ok(make_span(host, s))
        })
        .doc(
            "The field-wise sum: each unit adds independently — `1y + 2mo` is `1y 2mo`, and \
             `1h + 60m` stays `1h 60m` (units never convert; without a calendar there is no \
             correct conversion). Per-unit overflow throws an ArithmeticError.\n\n\
             ```\n\
             ((Span.years:1) + (Span.months:2)).s     \"* -> 1y 2mo\n\
             ```",
        )
        .sdk_typed_instance_method("-:", &["Span"], |host, receiver, args| {
            let s = fieldwise(
                span_of(receiver, "-:")?,
                span_of(args[0], "-:")?,
                true,
                "-:",
            )?;
            Ok(make_span(host, s))
        })
        .doc("The field-wise difference — `+:` with every unit of the argument negated.")
        .sdk_instance_method("negate", |host, receiver, _args| {
            Ok(make_span(host, span_of(receiver, "negate")?.negate()))
        })
        .doc(
            "The same units with every sign flipped.\n\n\
             ```\n\
             (Span.parse:'P1Y2M').negate.iso8601     \"* -> '-P1Y2M'\n\
             ```",
        )
        .sdk_instance_method("==:", |host, receiver, args| {
            let a = span_of(receiver, "==:")?;
            let eq = match args[0].with_native_state::<NativeSpan, _, _>(|s| s.0) {
                Ok(b) => a.fieldwise() == b.fieldwise(),
                Err(_) => false,
            };
            Ok(host.new_bool(eq))
        })
        .doc(
            "FIELDWISE equality: every unit must match — `1h` is not `60m` (whether they \
             name the same length depends on a calendar, so Span refuses to guess). A \
             non-Span argument is simply unequal.\n\n\
             ```\n\
             (Span.hours:1) == (Span.minutes:60)     \"* -> false\n\
             (Span.hours:1) == (Span.hours:1)        \"* -> true\n\
             ```",
        );
    // Unit getters — each answers ONLY its own field (nothing converts).
    let b = b
        .sdk_instance_method("years", |host, receiver, _args| {
            Ok(host.new_int(span_of(receiver, "years")?.get_years() as i64))
        })
        .doc("The years field alone (no conversion from other units).")
        .sdk_instance_method("months", |host, receiver, _args| {
            Ok(host.new_int(span_of(receiver, "months")?.get_months() as i64))
        })
        .doc("The months field alone.")
        .sdk_instance_method("weeks", |host, receiver, _args| {
            Ok(host.new_int(span_of(receiver, "weeks")?.get_weeks() as i64))
        })
        .doc("The weeks field alone.")
        .sdk_instance_method("days", |host, receiver, _args| {
            Ok(host.new_int(span_of(receiver, "days")?.get_days() as i64))
        })
        .doc("The days field alone.")
        .sdk_instance_method("hours", |host, receiver, _args| {
            Ok(host.new_int(span_of(receiver, "hours")?.get_hours() as i64))
        })
        .doc("The hours field alone.")
        .sdk_instance_method("minutes", |host, receiver, _args| {
            Ok(host.new_int(span_of(receiver, "minutes")?.get_minutes()))
        })
        .doc("The minutes field alone.")
        .sdk_instance_method("seconds", |host, receiver, _args| {
            Ok(host.new_int(span_of(receiver, "seconds")?.get_seconds()))
        })
        .doc(
            "The seconds field alone — fractional seconds parse into the millisecond/\
             microsecond/nanosecond fields, not here.",
        )
        .sdk_instance_method("milliseconds", |host, receiver, _args| {
            Ok(host.new_int(span_of(receiver, "milliseconds")?.get_milliseconds()))
        })
        .doc("The milliseconds field alone.")
        .sdk_instance_method("microseconds", |host, receiver, _args| {
            Ok(host.new_int(span_of(receiver, "microseconds")?.get_microseconds()))
        })
        .doc("The microseconds field alone.")
        .sdk_instance_method("nanoseconds", |host, receiver, _args| {
            Ok(host.new_int(span_of(receiver, "nanoseconds")?.get_nanoseconds()))
        })
        .doc("The nanoseconds field alone.");
    b.sdk_instance_method("asDuration", |host, receiver, _args| {
        let s = span_of(receiver, "asDuration")?;
        SignedDuration::try_from(s)
            .map(|d| make_duration(host, d))
            .map_err(|_| {
                QuoinError::ValueError(
                    "asDuration: the span has calendar units (years/months/weeks/days), which \
                     have no fixed length — apply it to a Date or DateTime instead"
                        .to_string(),
                )
            })
    })
    .doc(
        "The equivalent fixed Duration — defined only for a span of pure time units. A span \
         with calendar units (years/months/weeks/days) has no fixed length, so it throws a \
         ValueError rather than guessing.\n\n\
         ```\n\
         (Span.parse:'PT1H30M').asDuration.asSeconds     \"* -> 5400.0\n\
         ```",
    )
    .sdk_instance_method("s", |host, receiver, _args| {
        Ok(host.new_string(format!("{:#}", span_of(receiver, "s")?)))
    })
    .doc(
        "A human-readable rendering (jiff's friendly form). For a machine-readable form \
         use `iso8601`.\n\n\
         ```\n\
         (Span.parse:'P1Y2M3D').s     \"* -> 1y 2mo 3d\n\
         ```",
    )
    .sdk_instance_method("iso8601", |host, receiver, _args| {
        Ok(host.new_string(span_of(receiver, "iso8601")?.to_string()))
    })
    .doc(
        "The canonical ISO 8601 duration string, for serialization.\n\n\
         ```\n\
         ((Span.years:1) + (Span.hours:2)).iso8601     \"* -> 'P1YT2H'\n\
         ```",
    )
}
