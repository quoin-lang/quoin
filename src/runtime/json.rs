use crate::arg;
use crate::error::QuoinError;
use crate::runtime::big_decimal::{NativeBigDecimal, make_decimal};
use crate::runtime::big_integer::{NativeBigInteger, make_bigint};
use crate::runtime::list::NativeListState;
use crate::runtime::map::NativeMapState;
use crate::value::{NativeClassBuilder, ObjectPayload, Value};
use crate::vm::VmState;

use gc_arena::Mutation;
use num_bigint::BigInt;
use rust_decimal::Decimal;
use serde_json::Value as Json;
use std::collections::HashMap;

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
/// user instance, …) error — JSON has no representation for them.
fn value_to_json(v: Value) -> Result<Json, QuoinError> {
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
                    .map(|e| value_to_json(*e))
                    .collect::<Result<Vec<_>, _>>()?;
                return Ok(Json::Array(arr));
            }
            if let Ok(map) = v.with_native_state::<NativeMapState, _, _>(|m| m.get_map().clone()) {
                let mut obj_map = serde_json::Map::with_capacity(map.len());
                for (k, val) in map {
                    obj_map.insert(k, value_to_json(val)?);
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
fn json_to_value<'gc>(
    j: &Json,
    vm: &VmState<'gc>,
    mc: &Mutation<'gc>,
) -> Result<Value<'gc>, QuoinError> {
    match j {
        Json::Null => Ok(vm.new_nil(mc)),
        Json::Bool(b) => Ok(vm.new_bool(mc, *b)),
        Json::Number(n) => number_to_value(&n.to_string(), vm, mc),
        Json::String(s) => Ok(vm.new_string(mc, s.clone())),
        Json::Array(arr) => {
            let items = arr
                .iter()
                .map(|e| json_to_value(e, vm, mc))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(vm.new_list(mc, items))
        }
        Json::Object(obj) => {
            let mut map = HashMap::with_capacity(obj.len());
            for (k, val) in obj {
                map.insert(k.clone(), json_to_value(val, vm, mc)?);
            }
            Ok(vm.new_map(mc, map))
        }
    }
}

/// Classify a JSON number (its raw text) into the narrowest *exact* Quoin type: an integer →
/// `Integer` if it fits i64, else `BigInteger`; a decimal → `Double` iff it round-trips exactly
/// through f64 (so `0.1`/`3.14` stay Double), else `BigDecimal`. Never lossy.
fn number_to_value<'gc>(
    raw: &str,
    vm: &VmState<'gc>,
    mc: &Mutation<'gc>,
) -> Result<Value<'gc>, QuoinError> {
    let is_integer = !raw.bytes().any(|b| b == b'.' || b == b'e' || b == b'E');
    if is_integer {
        if let Ok(i) = raw.parse::<i64>() {
            return Ok(vm.new_int(mc, i));
        }
        if let Ok(big) = raw.parse::<BigInt>() {
            return Ok(make_bigint(vm, mc, big));
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
                return Ok(make_decimal(vm, mc, raw_dec));
            }
        }
    }
    Ok(vm.new_double(mc, f))
}

pub fn build_json_class() -> NativeClassBuilder {
    NativeClassBuilder::new("JSON", Some("Object"))
        // JSON.parse:'…' → a Quoin value (Map/List/String/Integer/Double/Bool/Nil, with
        // BigInteger/BigDecimal for out-of-range numbers). Malformed input → ParseError.
        .typed_class_method("parse:", &["String"], |vm, mc, _r, args| {
            let s = arg!(args, String, 0);
            let json: Json = serde_json::from_str(s.as_str())
                .map_err(|e| QuoinError::ParseError(format!("JSON.parse:: {e}")))?;
            json_to_value(&json, vm, mc)
        })
        // JSON.generate:value → a compact JSON string.
        .class_method("generate:", |vm, mc, _r, args| {
            let json = value_to_json(args[0])?;
            let s = serde_json::to_string(&json)
                .map_err(|e| QuoinError::Other(format!("JSON.generate:: {e}")))?;
            Ok(vm.new_string(mc, s))
        })
        // JSON.generatePretty:value → an indented JSON string.
        .class_method("generatePretty:", |vm, mc, _r, args| {
            let json = value_to_json(args[0])?;
            let s = serde_json::to_string_pretty(&json)
                .map_err(|e| QuoinError::Other(format!("JSON.generatePretty:: {e}")))?;
            Ok(vm.new_string(mc, s))
        })
}
