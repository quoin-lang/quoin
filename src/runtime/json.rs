use crate::arg;
use crate::error::QuoinError;
use crate::ext_sdk::Host;
use crate::runtime::big_decimal::{NativeBigDecimal, make_decimal};
use crate::runtime::big_integer::{NativeBigInteger, make_bigint};
use crate::runtime::data_value::{MAX_SERIALIZE_DEPTH, too_deep};
use crate::runtime::list::NativeListState;
use crate::runtime::map::NativeMapState;
use crate::value::{NativeClassBuilder, ObjectPayload, Value};

use indexmap::IndexMap;
use num_bigint::BigInt;
use rust_decimal::Decimal;
use serde_json::Value as Json;

/// A non-serializable Quoin value reached during `generate` — a clear TypeError naming the type.
fn unserializable(type_name: &str) -> QuoinError {
    QuoinError::TypeError {
        expected: "a JSON-serializable value".to_string(),
        got: type_name.to_string(),
        msg: format!("cannot serialize a {type_name} to JSON"),
    }
}

/// Quoin value → serde_json tree. Numbers keep full precision (`BigInteger`/`BigDecimal` serialize
/// their exact digits, via `arbitrary_precision`). `Bytes` and non-data types (Block, Duration, a
/// user instance, …) error — JSON has no representation for them. A value nested past
/// [`MAX_SERIALIZE_DEPTH`] errors too: JSON has its own `serde_json` recursion limit of 128 on the
/// way back in, and an unbounded walk of a cyclic value would abort the process.
fn value_to_json(v: Value) -> Result<Json, QuoinError> {
    value_to_json_at(v, 0)
}

fn value_to_json_at(v: Value, depth: usize) -> Result<Json, QuoinError> {
    if depth > MAX_SERIALIZE_DEPTH {
        return Err(too_deep());
    }
    match v {
        Value::Nil => Ok(Json::Null),
        Value::Bool(b) => Ok(Json::Bool(b)),
        Value::Int(i) => Ok(Json::Number(i.into())),
        Value::Double(f) => serde_json::Number::from_f64(f)
            .map(Json::Number)
            .ok_or_else(|| {
                QuoinError::ValueError("JSON cannot represent NaN or Infinity".to_string())
            }),
        Value::Object(obj) => {
            // Payload-level types first (String / Bytes / Block / Symbol / user instance).
            {
                let borrowed = obj.borrow();
                match &borrowed.payload {
                    ObjectPayload::String(s) => return Ok(Json::String((**s).clone())),
                    ObjectPayload::Bytes(_) => {
                        return Err(QuoinError::TypeError {
                            expected: "a JSON-serializable value".to_string(),
                            got: "Bytes".to_string(),
                            msg: "JSON has no bytes type; encode with Base64 first".to_string(),
                        });
                    }
                    ObjectPayload::Symbol(_) => return Err(unserializable("Symbol")),
                    ObjectPayload::Block(_) => return Err(unserializable("Block")),
                    ObjectPayload::Instance => return Err(unserializable(&borrowed.class_name())),
                    ObjectPayload::NativeState(_) => {} // dispatched below, after dropping the borrow
                }
            }
            if let Ok(items) =
                v.with_native_state::<NativeListState, _, _>(|l| l.get_vec().to_vec())
            {
                let arr = items
                    .iter()
                    .map(|e| value_to_json_at(*e, depth + 1))
                    .collect::<Result<Vec<_>, _>>()?;
                return Ok(Json::Array(arr));
            }
            if let Ok(map) = v.with_native_state::<NativeMapState, _, _>(|m| m.entries().to_vec()) {
                let mut obj_map = serde_json::Map::with_capacity(map.len());
                for (_, k, val) in map {
                    let Value::Object(kobj) = k else {
                        return Err(QuoinError::Other(
                            "JSON: Map keys must be Strings".to_string(),
                        ));
                    };
                    let crate::value::ObjectPayload::String(ks) = &kobj.borrow().payload else {
                        return Err(QuoinError::Other(
                            "JSON: Map keys must be Strings".to_string(),
                        ));
                    };
                    obj_map.insert((**ks).clone(), value_to_json_at(val, depth + 1)?);
                }
                return Ok(Json::Object(obj_map));
            }
            if let Ok(big) = v.with_native_state::<NativeBigInteger, _, _>(|d| d.0.clone()) {
                return big
                    .to_string()
                    .parse::<serde_json::Number>()
                    .map(Json::Number)
                    .map_err(|_| {
                        QuoinError::Other("BigInteger -> JSON number failed".to_string())
                    });
            }
            if let Ok(dec) = v.with_native_state::<NativeBigDecimal, _, _>(|d| d.0) {
                return dec
                    .to_string()
                    .parse::<serde_json::Number>()
                    .map(Json::Number)
                    .map_err(|_| {
                        QuoinError::Other("BigDecimal -> JSON number failed".to_string())
                    });
            }
            Err(unserializable(v.type_name()))
        }
        _ => Err(unserializable(v.type_name())),
    }
}

/// serde_json tree → Quoin value. Object → `Map`, array → `List`; numbers classified for lossless
/// representation (see `number_to_value`).
fn json_to_value<'gc>(j: &Json, host: &dyn Host<'gc>) -> Result<Value<'gc>, QuoinError> {
    match j {
        Json::Null => Ok(host.new_nil()),
        Json::Bool(b) => Ok(host.new_bool(*b)),
        Json::Number(n) => number_to_value(&n.to_string(), host),
        Json::String(s) => Ok(host.new_string(s.clone())),
        Json::Array(arr) => {
            let items = arr
                .iter()
                .map(|e| json_to_value(e, host))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(host.new_list(items))
        }
        Json::Object(obj) => {
            let mut map = IndexMap::with_capacity(obj.len());
            for (k, val) in obj {
                map.insert(k.clone(), json_to_value(val, host)?);
            }
            Ok(host.new_map(map))
        }
    }
}

/// Classify a JSON number (its raw text) into the narrowest *exact* Quoin type: an integer →
/// `Integer` if it fits i64, else `BigInteger`; a decimal → `Double` iff it round-trips exactly
/// through f64 (so `0.1`/`3.14` stay Double), else `BigDecimal`. Never lossy.
fn number_to_value<'gc>(raw: &str, host: &dyn Host<'gc>) -> Result<Value<'gc>, QuoinError> {
    let is_integer = !raw.bytes().any(|b| b == b'.' || b == b'e' || b == b'E');
    if is_integer {
        if let Ok(i) = raw.parse::<i64>() {
            return Ok(host.new_int(i));
        }
        if let Ok(big) = raw.parse::<BigInt>() {
            return Ok(make_bigint(host, big));
        }
    }
    let f = raw
        .parse::<f64>()
        .map_err(|_| QuoinError::ParseError(format!("JSON: malformed number '{raw}'")))?;
    if f.is_finite() {
        if let Ok(raw_dec) = raw.parse::<Decimal>() {
            // Double only when f64's shortest round-trip equals the literal's exact decimal value.
            let f_is_exact = format!("{f}")
                .parse::<Decimal>()
                .map(|fd| fd == raw_dec)
                .unwrap_or(false);
            if !f_is_exact {
                return Ok(make_decimal(host, raw_dec));
            }
        }
    }
    Ok(host.new_double(f))
}

pub fn build_json_class() -> NativeClassBuilder {
    NativeClassBuilder::new("JSON", Some("Object"))
        .abstract_class()
        .class_doc(
            "Parse and generate JSON text.\n\n\
             `parse:` maps JSON onto plain Quoin values — object → Map, array → List, plus \
             String / Integer / Double / Boolean / nil — and `generate:` / `generatePretty:` \
             go the other way. Numbers are never silently lossy: an integer past 64 bits \
             parses as a BigInteger, and a decimal a Double cannot represent exactly parses \
             as a BigDecimal.\n\n\
             ```\n\
             JSON.parse:'{\"a\": 1}'            \"* -> #{'a': 1}\n\
             JSON.generate:#{'a': 1}          \"* -> '{\"a\":1}'\n\
             ```",
        )
        // JSON.parse:'…' → a Quoin value (Map/List/String/Integer/Double/Bool/Nil, with
        // BigInteger/BigDecimal for out-of-range numbers). Malformed input → ParseError.
        .sdk_typed_class_method("parse:", &["String"], |host, _r, args| {
            let s = arg!(args, String, 0);
            let json: Json = serde_json::from_str(s.as_str())
                .map_err(|e| QuoinError::ParseError(format!("JSON.parse:: {e}")))?;
            json_to_value(&json, host)
        })
        .doc(
            "Parse a JSON string into Quoin values: object → Map, array → List, and String / \
             Integer / Double / Boolean / nil for the scalars (BigInteger / BigDecimal when a \
             number won't fit exactly). Malformed input throws a ParseError.\n\n\
             ```\n\
             JSON.parse:'{\"a\": 1}'      \"* -> #{'a': 1}\n\
             JSON.parse:'[1, 2, 3]'     \"* -> #(1 2 3)\n\
             ```",
        )
        // JSON.generate:value → a compact JSON string.
        .sdk_class_method("generate:", |host, _r, args| {
            let json = value_to_json(args[0])?;
            let s = serde_json::to_string(&json)
                .map_err(|e| QuoinError::Other(format!("JSON.generate:: {e}")))?;
            Ok(host.new_string(s))
        })
        .doc(
            "Generate the compact (single-line) JSON string for a value. Maps, Lists, \
             Strings, numbers (including BigInteger / BigDecimal, at full precision), \
             Booleans, and nil serialize; Map keys must be Strings; Bytes and other types \
             throw a TypeError (encode Bytes with Base64 first).\n\n\
             ```\n\
             JSON.generate:#{'a': 1 'b': #(1 2)}     \"* -> '{\"a\":1,\"b\":[1,2]}'\n\
             ```",
        )
        // JSON.generatePretty:value → an indented JSON string.
        .sdk_class_method("generatePretty:", |host, _r, args| {
            let json = value_to_json(args[0])?;
            let s = serde_json::to_string_pretty(&json)
                .map_err(|e| QuoinError::Other(format!("JSON.generatePretty:: {e}")))?;
            Ok(host.new_string(s))
        })
        .doc(
            "Like `generate:`, but indented for human eyes (two spaces, one key per \
             line).\n\n\
             ```\n\
             JSON.generatePretty:#{'a': 1}     \"* -> '{\\n  \"a\": 1\\n}'\n\
             ```",
        )
}
