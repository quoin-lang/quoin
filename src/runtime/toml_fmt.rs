use crate::arg;
use crate::error::QuoinError;
use crate::runtime::data_value::{DataValue, data_to_value, value_to_data};
use crate::value::{NativeClassBuilder, Value};

/// `TOML` — config-file format over the DataValue bridge (the `toml` crate). TOML's top level must
/// be a table, so `generate:` requires a `Map`; TOML has no null, so a `nil` anywhere errors.
pub fn build_toml_class() -> NativeClassBuilder {
    NativeClassBuilder::new("TOML", Some("Object"))
        // TOML.parse:'…' -> a Quoin value (a Map at the top level).
        .typed_class_method("parse:", &["String"], |vm, mc, _r, args| {
            let s = arg!(args, String, 0);
            let data: DataValue = toml::from_str(s.as_str())
                .map_err(|e| QuoinError::ParseError(format!("TOML.parse:: {e}")))?;
            data_to_value(&data, vm, mc)
        })
        // TOML.generate:aMap -> a TOML document.
        .class_method("generate:", |vm, mc, _r, args| {
            let data = value_to_data(args[0])?;
            if !matches!(data, DataValue::Object(_)) {
                return Err(QuoinError::ValueError(
                    "TOML.generate:: the top-level value must be a Map (a TOML table)".to_string(),
                ));
            }
            if contains_null(&data) {
                return Err(QuoinError::ValueError(
                    "TOML.generate:: TOML has no null; remove nil values".to_string(),
                ));
            }
            let s = toml::to_string(&data)
                .map_err(|e| QuoinError::ValueError(format!("TOML.generate:: {e}")))?;
            Ok(vm.new_string(mc, s))
        })
}

/// TOML has no null — detect a `nil` anywhere so `generate:` reports it clearly.
fn contains_null(dv: &DataValue) -> bool {
    match dv {
        DataValue::Null => true,
        DataValue::Array(items) => items.iter().any(contains_null),
        DataValue::Object(pairs) => pairs.iter().any(|(_, v)| contains_null(v)),
        _ => false,
    }
}
