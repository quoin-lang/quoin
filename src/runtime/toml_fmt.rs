use crate::arg;
use crate::error::QuoinError;
use crate::runtime::data_value::{DataValue, data_to_value, value_to_data};
use crate::value::{NativeClassBuilder, Value};

/// `TOML` — config-file format over the DataValue bridge (the `toml` crate). TOML's top level must
/// be a table, so `generate:` requires a `Map`; TOML has no null, so a `nil` anywhere errors.
pub fn build_toml_class() -> NativeClassBuilder {
    NativeClassBuilder::new("TOML", Some("Object"))
        .abstract_class()
        .class_doc(
            "Parse and generate TOML, the config-file format.\n\n\
             The same value mapping as `JSON` (table → Map, array → List, plus the scalars), \
             with TOML's two format constraints enforced: the top level must be a table (a \
             Map), and TOML has no null, so a `nil` anywhere in the value errors on \
             `generate:`.\n\n\
             ```\n\
             TOML.parse:'x = 1'               \"* -> #{'x': 1}\n\
             TOML.generate:#{'port': 8080}    \"* -> 'port = 8080\\n'\n\
             ```",
        )
        // TOML.parse:'…' -> a Quoin value (a Map at the top level).
        .sdk_typed_class_method("parse:", &["String"], |host, _r, args| {
            let s = arg!(args, String, 0);
            let data: DataValue = toml::from_str(s.as_str())
                .map_err(|e| QuoinError::ParseError(format!("TOML.parse:: {e}")))?;
            data_to_value(&data, host)
        })
        .doc(
            "Parse a TOML document into Quoin values — always a Map at the top level, with \
             tables as nested Maps. Malformed input throws a ParseError.\n\n\
             ```\n\
             TOML.parse:'[server]\\nhost = \"localhost\"'     \"* -> #{'server': #{'host': 'localhost'}}\n\
             ```",
        )
        // TOML.generate:aMap -> a TOML document.
        .sdk_class_method("generate:", |host, _r, args| {
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
            Ok(host.new_string(s))
        })
        .doc(
            "Generate the TOML document for a Map (nested Maps become tables). The top-level \
             value must be a Map, and no `nil` may appear anywhere — TOML has no null; either \
             throws a ValueError.\n\n\
             ```\n\
             TOML.generate:#{'server': #{'host': 'localhost' 'port': 8080}}\n\
             \"* -> '[server]\\nhost = \"localhost\"\\nport = 8080\\n'\n\
             ```",
        )
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
