use crate::error::QuoinError;
use crate::runtime::big_decimal::{NativeBigDecimal, make_decimal};
use crate::runtime::big_integer::{NativeBigInteger, make_bigint};
use crate::runtime::list::NativeListState;
use crate::runtime::map::NativeMapState;
use crate::value::{ObjectPayload, Value};
use crate::vm::VmState;

use gc_arena::Mutation;
use indexmap::IndexMap;
use num_bigint::BigInt;
use num_traits::ToPrimitive;
use rust_decimal::Decimal;
use serde::de::{Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::ser::{Serialize, SerializeMap, SerializeSeq, Serializer};
use std::fmt;

/// The neutral, GC-free data tree shared by the structured formats (MessagePack/TOML/YAML). It
/// carries the full Quoin scalar range — `BigInt`/`Decimal`/`Bytes` included — and hand-implements
/// serde `Serialize`/`Deserialize`, so every serde format reads/writes it directly.
///
/// (JSON does *not* go through `DataValue`: it keeps its own `serde_json::Value` path with
/// `arbitrary_precision`, the only way to emit arbitrary-precision *numbers*. Here, since serde's
/// data model caps at i128/u128/f64, an out-of-range `BigInt` and any `Decimal` fall back to a
/// string — lossless as text, the best these formats allow.)
#[derive(Debug, Clone, PartialEq)]
pub enum DataValue {
    Null,
    Bool(bool),
    Int(i64),
    BigInt(BigInt),
    Float(f64),
    Decimal(Decimal),
    Str(String),
    Bytes(Vec<u8>),
    Array(Vec<DataValue>),
    Object(Vec<(String, DataValue)>),
}

fn unrepresentable(type_name: &str) -> QuoinError {
    QuoinError::TypeError {
        expected: "a serializable value".to_string(),
        got: type_name.to_string(),
        msg: format!("cannot serialize a {type_name} (no data representation)"),
    }
}

/// Walk a Quoin value into a `DataValue` (the generate side). Errors on values with no data
/// representation (Block, Symbol, a user instance, another native type like Duration/DateTime).
/// Object pairs keep the Map's insertion order.
pub fn value_to_data(v: Value) -> Result<DataValue, QuoinError> {
    match v {
        Value::Nil => Ok(DataValue::Null),
        Value::Bool(b) => Ok(DataValue::Bool(b)),
        Value::Int(i) => Ok(DataValue::Int(i)),
        Value::Double(f) => Ok(DataValue::Float(f)),
        Value::Object(obj) => {
            {
                let borrowed = obj.borrow();
                match &borrowed.payload {
                    ObjectPayload::String(s) => return Ok(DataValue::Str((**s).clone())),
                    ObjectPayload::Bytes(b) => return Ok(DataValue::Bytes((**b).clone())),
                    ObjectPayload::Symbol(_) => return Err(unrepresentable("Symbol")),
                    ObjectPayload::Block(_) => return Err(unrepresentable("Block")),
                    ObjectPayload::Instance => return Err(unrepresentable(&borrowed.class_name())),
                    ObjectPayload::NativeState(_) => {} // dispatched below, after dropping the borrow
                }
            }
            if let Ok(items) =
                v.with_native_state::<NativeListState, _, _>(|l| l.get_vec().to_vec())
            {
                let arr = items
                    .iter()
                    .map(|e| value_to_data(*e))
                    .collect::<Result<Vec<_>, _>>()?;
                return Ok(DataValue::Array(arr));
            }
            if let Ok(map) = v.with_native_state::<NativeMapState, _, _>(|m| m.get_map().clone()) {
                let mut pairs = Vec::with_capacity(map.len());
                for (k, val) in map {
                    pairs.push((k, value_to_data(val)?));
                }
                return Ok(DataValue::Object(pairs));
            }
            if let Ok(big) = v.with_native_state::<NativeBigInteger, _, _>(|d| d.0.clone()) {
                return Ok(DataValue::BigInt(big));
            }
            if let Ok(dec) = v.with_native_state::<NativeBigDecimal, _, _>(|d| d.0) {
                return Ok(DataValue::Decimal(dec));
            }
            Err(unrepresentable(v.type_name()))
        }
        _ => Err(unrepresentable(v.type_name())),
    }
}

/// Build a Quoin value from a `DataValue` (the parse side). `Object` → `Map`, `Array` → `List`,
/// `BigInt` → `BigInteger`, `Decimal` → `BigDecimal`, `Bytes` → `Bytes`.
pub fn data_to_value<'gc>(
    dv: &DataValue,
    vm: &VmState<'gc>,
    mc: &Mutation<'gc>,
) -> Result<Value<'gc>, QuoinError> {
    Ok(match dv {
        DataValue::Null => vm.new_nil(mc),
        DataValue::Bool(b) => vm.new_bool(mc, *b),
        DataValue::Int(i) => vm.new_int(mc, *i),
        DataValue::BigInt(b) => make_bigint(vm, mc, b.clone()),
        DataValue::Float(f) => vm.new_double(mc, *f),
        DataValue::Decimal(d) => make_decimal(vm, mc, *d),
        DataValue::Str(s) => vm.new_string(mc, s.clone()),
        DataValue::Bytes(b) => vm.new_bytes(mc, b.clone()),
        DataValue::Array(items) => {
            let vals = items
                .iter()
                .map(|e| data_to_value(e, vm, mc))
                .collect::<Result<Vec<_>, _>>()?;
            vm.new_list(mc, vals)
        }
        DataValue::Object(pairs) => {
            let mut map = IndexMap::with_capacity(pairs.len());
            for (k, val) in pairs {
                map.insert(k.clone(), data_to_value(val, vm, mc)?);
            }
            vm.new_map(mc, map)
        }
    })
}

impl Serialize for DataValue {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            DataValue::Null => s.serialize_unit(),
            DataValue::Bool(b) => s.serialize_bool(*b),
            DataValue::Int(i) => s.serialize_i64(*i),
            // A native int where it fits, else the exact digits as a string (serde's data model
            // has no arbitrary-precision integer; these formats can't hold one as a number).
            DataValue::BigInt(b) => match b.to_i64() {
                Some(i) => s.serialize_i64(i),
                None => match b.to_u64() {
                    Some(u) => s.serialize_u64(u),
                    None => s.serialize_str(&b.to_string()),
                },
            },
            DataValue::Float(f) => s.serialize_f64(*f),
            // No native decimal in these formats — the exact digits as a string beat a lossy f64.
            DataValue::Decimal(d) => s.serialize_str(&d.to_string()),
            DataValue::Str(st) => s.serialize_str(st),
            DataValue::Bytes(b) => s.serialize_bytes(b),
            DataValue::Array(items) => {
                let mut seq = s.serialize_seq(Some(items.len()))?;
                for item in items {
                    seq.serialize_element(item)?;
                }
                seq.end()
            }
            DataValue::Object(pairs) => {
                let mut map = s.serialize_map(Some(pairs.len()))?;
                for (k, v) in pairs {
                    map.serialize_entry(k, v)?;
                }
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for DataValue {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(DataValueVisitor)
    }
}

struct DataValueVisitor;

impl<'de> Visitor<'de> for DataValueVisitor {
    type Value = DataValue;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("any data value")
    }

    fn visit_bool<E>(self, v: bool) -> Result<DataValue, E> {
        Ok(DataValue::Bool(v))
    }
    fn visit_i64<E>(self, v: i64) -> Result<DataValue, E> {
        Ok(DataValue::Int(v))
    }
    fn visit_i128<E>(self, v: i128) -> Result<DataValue, E> {
        Ok(i64::try_from(v).map_or_else(|_| DataValue::BigInt(BigInt::from(v)), DataValue::Int))
    }
    fn visit_u64<E>(self, v: u64) -> Result<DataValue, E> {
        Ok(i64::try_from(v).map_or_else(|_| DataValue::BigInt(BigInt::from(v)), DataValue::Int))
    }
    fn visit_u128<E>(self, v: u128) -> Result<DataValue, E> {
        Ok(i64::try_from(v).map_or_else(|_| DataValue::BigInt(BigInt::from(v)), DataValue::Int))
    }
    fn visit_f64<E>(self, v: f64) -> Result<DataValue, E> {
        Ok(DataValue::Float(v))
    }
    fn visit_str<E>(self, v: &str) -> Result<DataValue, E> {
        Ok(DataValue::Str(v.to_string()))
    }
    fn visit_string<E>(self, v: String) -> Result<DataValue, E> {
        Ok(DataValue::Str(v))
    }
    fn visit_bytes<E>(self, v: &[u8]) -> Result<DataValue, E> {
        Ok(DataValue::Bytes(v.to_vec()))
    }
    fn visit_byte_buf<E>(self, v: Vec<u8>) -> Result<DataValue, E> {
        Ok(DataValue::Bytes(v))
    }
    fn visit_none<E>(self) -> Result<DataValue, E> {
        Ok(DataValue::Null)
    }
    fn visit_unit<E>(self) -> Result<DataValue, E> {
        Ok(DataValue::Null)
    }
    fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<DataValue, D::Error> {
        d.deserialize_any(self)
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<DataValue, A::Error> {
        let mut items = Vec::new();
        while let Some(el) = seq.next_element()? {
            items.push(el);
        }
        Ok(DataValue::Array(items))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<DataValue, A::Error> {
        // Quoin Maps are string-keyed; a non-string key (e.g. a MessagePack integer key) errors.
        let mut pairs = Vec::new();
        while let Some((k, v)) = map.next_entry::<String, DataValue>()? {
            pairs.push((k, v));
        }
        Ok(DataValue::Object(pairs))
    }
}

#[cfg(test)]
#[path = "data_value_tests.rs"]
mod tests;
