use crate::arg;
use crate::error::QuoinError;
use crate::ext_sdk::{Host, HostExt};
use crate::runtime::pretty::{PpChild, PpRole, PpShape, PrettyPrint};
use crate::value::{AnyCollect, NativeClassBuilder, Value};

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

impl PrettyPrint for NativeUuid {
    fn pp_shape<'gc>(&self) -> PpShape<'gc> {
        PpShape::Record {
            name: "UUID",
            fields: vec![
                (
                    "version".to_string(),
                    PpChild::Val(Value::Int(self.0.get_version_num() as i64)),
                ),
                (
                    "hex".to_string(),
                    PpChild::Text(self.0.as_simple().to_string(), PpRole::Str),
                ),
            ],
        }
    }
}

fn uuid_of(v: Value, who: &str) -> Result<Uuid, QuoinError> {
    v.with_native_state::<NativeUuid, _, _>(|u| u.0)
        .map_err(|_| QuoinError::TypeError {
            expected: "UUID".to_string(),
            got: "a non-UUID value".to_string(),
            msg: format!("{who} requires a UUID operand"),
        })
}

fn make_uuid<'gc>(host: &dyn Host<'gc>, u: Uuid) -> Value<'gc> {
    let class = host.get_or_create_builtin_class("UUID");
    host.new_native_state(class, NativeUuid(u))
}

pub fn build_uuid_class() -> NativeClassBuilder {
    let b = NativeClassBuilder::new("UUID", Some("Object"))
        .construct_with("use UUID.generateV4 / UUID.generateV7 / UUID.parse:")
        .class_doc(
            "A 128-bit universally unique identifier. Generate with `UUID.generateV4` \
             (random) or `UUID.generateV7` (time-ordered, so fresh IDs sort by creation \
             time), or parse the standard hyphenated form with `UUID.parse:`. Renders via \
             `s` as the 36-character hyphenated lowercase form.\n\n\
             ```\n\
             UUID.generateV4.s.length     \"* -> 36\n\
             ```",
        )
        // UUID.generateV4 -> a random (v4) UUID.
        .sdk_class_method("generateV4", |host, _r, _a| {
            Ok(make_uuid(host, Uuid::new_v4()))
        })
        .doc(
            "A new random (version 4) UUID -- 122 random bits.\n\n\
             ```\n\
             UUID.generateV4.version     \"* -> 4\n\
             ```",
        )
        // UUID.generateV7 -> a time-ordered (v7) UUID.
        .sdk_class_method("generateV7", |host, _r, _a| {
            Ok(make_uuid(host, Uuid::now_v7()))
        })
        .doc(
            "A new time-ordered (version 7) UUID: a millisecond timestamp in the high \
             bits plus randomness, so IDs generated later sort greater -- a good fit for \
             database keys.\n\n\
             ```\n\
             UUID.generateV7.version     \"* -> 7\n\
             ```",
        )
        // UUID.parse:'…' -> parse a hyphenated UUID string.
        .sdk_typed_class_method("parse:", &["String"], |host, _r, args| {
            let s = arg!(args, String, 0);
            match Uuid::parse_str(s.as_str()) {
                Ok(u) => Ok(make_uuid(host, u)),
                Err(_) => Err(QuoinError::ValueError(format!(
                    "UUID.parse:: not a UUID: '{}'",
                    s.as_str()
                ))),
            }
        })
        .doc(
            "The UUID written in the standard hyphenated String form; raises a ValueError \
             if the argument is not one.\n\n\
             ```\n\
             UUID.parse:'67e55044-10b1-426f-9247-bb680e5fe0c8'     \"* -> \
             67e55044-10b1-426f-9247-bb680e5fe0c8\n\
             ```",
        );
    let b = b
        // Only `<:` is native; the other comparisons derive from it on Object.
        .sdk_typed_instance_method("<:", &["UUID"], |host, r, args| {
            Ok(host.new_bool(uuid_of(r, "<:")? < uuid_of(args[0], "<:")?))
        })
        .doc(
            "Byte-order less-than against another UUID; for version-7 UUIDs this is \
             creation order. The one native comparison -- the others derive from it.",
        )
        .sdk_instance_method("==:", |host, r, args| {
            let a = uuid_of(r, "==:")?;
            let eq = match args[0].with_native_state::<NativeUuid, _, _>(|u| u.0) {
                Ok(b) => a == b,
                Err(_) => false,
            };
            Ok(host.new_bool(eq))
        })
        .doc(
            "Whether the argument is the same UUID (by value); a non-UUID is simply \
             unequal, never an error.",
        );
    b.sdk_instance_method("s", |host, r, _a| {
        Ok(host.new_string(uuid_of(r, "s")?.to_string()))
    })
    .doc("The 36-character hyphenated lowercase form.")
    .sdk_instance_method("asBytes", |host, r, _a| {
        Ok(host.new_bytes(uuid_of(r, "asBytes")?.as_bytes().to_vec()))
    })
    .doc(
        "The 16 raw bytes, big-endian.\n\n\
         ```\n\
         (UUID.parse:'67e55044-10b1-426f-9247-bb680e5fe0c8').asBytes\n\
         \"* -> Bytes[16] 67 e5 50 44 10 b1 42 6f 92 47 bb 68 0e 5f e0 c8\n\
         ```",
    )
    .sdk_instance_method("version", |host, r, _a| {
        Ok(host.new_int(uuid_of(r, "version")?.get_version_num() as i64))
    })
    .doc(
        "The UUID's version number -- 4 for random, 7 for time-ordered.\n\n\
         ```\n\
         UUID.generateV7.version     \"* -> 7\n\
         ```",
    )
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

impl PrettyPrint for NativeUlid {
    fn pp_shape<'gc>(&self) -> PpShape<'gc> {
        // ULID = 48-bit millisecond timestamp + 80-bit randomness (20 hex digits).
        PpShape::Record {
            name: "ULID",
            fields: vec![
                (
                    "timestampMillis".to_string(),
                    PpChild::Val(Value::Int(self.0.timestamp_ms() as i64)),
                ),
                (
                    "random".to_string(),
                    PpChild::Text(format!("{:020x}", self.0.random()), PpRole::Str),
                ),
            ],
        }
    }
}

fn ulid_of(v: Value, who: &str) -> Result<Ulid, QuoinError> {
    v.with_native_state::<NativeUlid, _, _>(|u| u.0)
        .map_err(|_| QuoinError::TypeError {
            expected: "ULID".to_string(),
            got: "a non-ULID value".to_string(),
            msg: format!("{who} requires a ULID operand"),
        })
}

fn make_ulid<'gc>(host: &dyn Host<'gc>, u: Ulid) -> Value<'gc> {
    let class = host.get_or_create_builtin_class("ULID");
    host.new_native_state(class, NativeUlid(u))
}

pub fn build_ulid_class() -> NativeClassBuilder {
    let b = NativeClassBuilder::new("ULID", Some("Object"))
        .construct_with("use ULID.generate / ULID.parse:")
        .class_doc(
            "A 128-bit sortable identifier: a 48-bit millisecond timestamp plus 80 random \
             bits, rendered as 26 characters of Crockford base32. String order equals \
             creation order, so ULIDs make good keys where insertion order matters. \
             Generate with `ULID.generate`, parse with `ULID.parse:`.\n\n\
             ```\n\
             ULID.generate.s.length     \"* -> 26\n\
             ```",
        )
        // ULID.generate -> a new ULID (current time + randomness).
        .sdk_class_method("generate", |host, _r, _a| Ok(make_ulid(host, Ulid::new())))
        .doc(
            "A new ULID stamped with the current time plus randomness.\n\n\
             ```\n\
             ULID.generate.s.length     \"* -> 26\n\
             ```",
        )
        // ULID.parse:'…' -> parse a 26-char Crockford base32 ULID string.
        .sdk_typed_class_method("parse:", &["String"], |host, _r, args| {
            let s = arg!(args, String, 0);
            match Ulid::from_string(s.as_str()) {
                Ok(u) => Ok(make_ulid(host, u)),
                Err(_) => Err(QuoinError::ValueError(format!(
                    "ULID.parse:: not a ULID: '{}'",
                    s.as_str()
                ))),
            }
        })
        .doc(
            "The ULID for a 26-character Crockford-base32 String; raises a ValueError \
             otherwise.\n\n\
             ```\n\
             ULID.parse:'01ARZ3NDEKTSV4RRFFQ69G5FAV'     \"* -> \
             01ARZ3NDEKTSV4RRFFQ69G5FAV\n\
             ```",
        );
    let b = b
        // ULID `<:` is lexicographic = chronological order (the timestamp is the high bits).
        .sdk_typed_instance_method("<:", &["ULID"], |host, r, args| {
            Ok(host.new_bool(ulid_of(r, "<:")? < ulid_of(args[0], "<:")?))
        })
        .doc(
            "Less-than against another ULID -- lexicographic on the canonical string, \
             which is chronological order (the timestamp occupies the high bits). The one \
             native comparison -- the others derive from it.",
        )
        .sdk_instance_method("==:", |host, r, args| {
            let a = ulid_of(r, "==:")?;
            let eq = match args[0].with_native_state::<NativeUlid, _, _>(|u| u.0) {
                Ok(b) => a == b,
                Err(_) => false,
            };
            Ok(host.new_bool(eq))
        })
        .doc(
            "Whether the argument is the same ULID (by value); a non-ULID is simply \
             unequal, never an error.",
        );
    b.sdk_instance_method("s", |host, r, _a| {
        Ok(host.new_string(ulid_of(r, "s")?.to_string()))
    })
    .doc("The canonical 26-character uppercase Crockford-base32 form.")
    .sdk_instance_method("asBytes", |host, r, _a| {
        Ok(host.new_bytes(ulid_of(r, "asBytes")?.to_bytes().to_vec()))
    })
    .doc("The 16 raw bytes, timestamp first (big-endian).")
    // The embedded Unix-millisecond timestamp.
    .sdk_instance_method("timestampMillis", |host, r, _a| {
        Ok(host.new_int(ulid_of(r, "timestampMillis")?.timestamp_ms() as i64))
    })
    .doc(
        "The embedded creation time, as Unix milliseconds.\n\n\
         ```\n\
         (ULID.parse:'01ARZ3NDEKTSV4RRFFQ69G5FAV').timestampMillis     \"* -> \
         1469922850259\n\
         ```",
    )
}
