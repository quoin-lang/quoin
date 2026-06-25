use crate::arg;
use crate::error::QuoinError;
use crate::value::{AnyCollect, NativeClassBuilder, Value};
use crate::vm::VmState;

use gc_arena::Mutation;
use gc_arena::collect::Trace;
use std::any::Any;
use ulid::Ulid;
use uuid::Uuid;

// ----- UUID -----

/// Native backing state for a `UUID` (128-bit, `Copy`). No `Gc` fields / no OS resource.
#[derive(Debug)]
pub struct NativeUuid(pub Uuid);

impl AnyCollect for NativeUuid {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn trace_gc<'gc>(&self, _cc: &mut dyn Trace<'gc>) {}
}

fn uuid_of(v: Value, who: &str) -> Result<Uuid, QuoinError> {
    v.with_native_state::<NativeUuid, _, _>(|u| u.0)
        .map_err(|_| QuoinError::TypeError {
            expected: "UUID".to_string(),
            got: "a non-UUID value".to_string(),
            msg: format!("{who} requires a UUID operand"),
        })
}

fn make_uuid<'gc>(vm: &VmState<'gc>, mc: &Mutation<'gc>, u: Uuid) -> Value<'gc> {
    let class = vm.get_or_create_builtin_class(mc, "UUID");
    vm.new_native_state(mc, class, NativeUuid(u))
}

pub fn build_uuid_class() -> NativeClassBuilder {
    let b = NativeClassBuilder::new("UUID", Some("Object"))
        // UUID.generateV4 -> a random (v4) UUID.
        .class_method("generateV4", |vm, mc, _r, _a| {
            Ok(make_uuid(vm, mc, Uuid::new_v4()))
        })
        // UUID.generateV7 -> a time-ordered (v7) UUID.
        .class_method("generateV7", |vm, mc, _r, _a| {
            Ok(make_uuid(vm, mc, Uuid::now_v7()))
        })
        // UUID.parse:'…' -> parse a hyphenated UUID string.
        .typed_class_method("parse:", &["String"], |vm, mc, _r, args| {
            let s = arg!(args, String, 0);
            match Uuid::parse_str(s.as_str()) {
                Ok(u) => Ok(make_uuid(vm, mc, u)),
                Err(_) => Err(QuoinError::ValueError(format!(
                    "UUID.parse:: not a UUID: '{}'",
                    s.as_str()
                ))),
            }
        });
    let b = b
        // Only `<:` is native; the other comparisons derive from it on Object.
        .typed_instance_method("<:", &["UUID"], |vm, mc, r, args| {
            Ok(vm.new_bool(mc, uuid_of(r, "<:")? < uuid_of(args[0], "<:")?))
        })
        .instance_method("==:", |vm, mc, r, args| {
            let a = uuid_of(r, "==:")?;
            let eq = match args[0].with_native_state::<NativeUuid, _, _>(|u| u.0) {
                Ok(b) => a == b,
                Err(_) => false,
            };
            Ok(vm.new_bool(mc, eq))
        });
    b.instance_method("s", |vm, mc, r, _a| {
        Ok(vm.new_string(mc, uuid_of(r, "s")?.to_string()))
    })
    .instance_method("asBytes", |vm, mc, r, _a| {
        Ok(vm.new_bytes(mc, uuid_of(r, "asBytes")?.as_bytes().to_vec()))
    })
    .instance_method("version", |vm, mc, r, _a| {
        Ok(vm.new_int(mc, uuid_of(r, "version")?.get_version_num() as i64))
    })
}

// ----- ULID -----

/// Native backing state for a `ULID` (128-bit, `Copy`) — a sortable, base32 identifier.
#[derive(Debug)]
pub struct NativeUlid(pub Ulid);

impl AnyCollect for NativeUlid {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
    fn trace_gc<'gc>(&self, _cc: &mut dyn Trace<'gc>) {}
}

fn ulid_of(v: Value, who: &str) -> Result<Ulid, QuoinError> {
    v.with_native_state::<NativeUlid, _, _>(|u| u.0)
        .map_err(|_| QuoinError::TypeError {
            expected: "ULID".to_string(),
            got: "a non-ULID value".to_string(),
            msg: format!("{who} requires a ULID operand"),
        })
}

fn make_ulid<'gc>(vm: &VmState<'gc>, mc: &Mutation<'gc>, u: Ulid) -> Value<'gc> {
    let class = vm.get_or_create_builtin_class(mc, "ULID");
    vm.new_native_state(mc, class, NativeUlid(u))
}

pub fn build_ulid_class() -> NativeClassBuilder {
    let b = NativeClassBuilder::new("ULID", Some("Object"))
        // ULID.generate -> a new ULID (current time + randomness).
        .class_method("generate", |vm, mc, _r, _a| {
            Ok(make_ulid(vm, mc, Ulid::new()))
        })
        // ULID.parse:'…' -> parse a 26-char Crockford base32 ULID string.
        .typed_class_method("parse:", &["String"], |vm, mc, _r, args| {
            let s = arg!(args, String, 0);
            match Ulid::from_string(s.as_str()) {
                Ok(u) => Ok(make_ulid(vm, mc, u)),
                Err(_) => Err(QuoinError::ValueError(format!(
                    "ULID.parse:: not a ULID: '{}'",
                    s.as_str()
                ))),
            }
        });
    let b = b
        // ULID `<:` is lexicographic = chronological order (the timestamp is the high bits).
        .typed_instance_method("<:", &["ULID"], |vm, mc, r, args| {
            Ok(vm.new_bool(mc, ulid_of(r, "<:")? < ulid_of(args[0], "<:")?))
        })
        .instance_method("==:", |vm, mc, r, args| {
            let a = ulid_of(r, "==:")?;
            let eq = match args[0].with_native_state::<NativeUlid, _, _>(|u| u.0) {
                Ok(b) => a == b,
                Err(_) => false,
            };
            Ok(vm.new_bool(mc, eq))
        });
    b.instance_method("s", |vm, mc, r, _a| {
        Ok(vm.new_string(mc, ulid_of(r, "s")?.to_string()))
    })
    .instance_method("asBytes", |vm, mc, r, _a| {
        Ok(vm.new_bytes(mc, ulid_of(r, "asBytes")?.to_bytes().to_vec()))
    })
    // The embedded Unix-millisecond timestamp.
    .instance_method("timestampMillis", |vm, mc, r, _a| {
        Ok(vm.new_int(mc, ulid_of(r, "timestampMillis")?.timestamp_ms() as i64))
    })
}
