use crate::arg;
use crate::error::QuoinError;
use crate::runtime::data_value::{DataValue, data_to_value, value_to_data};
use crate::value::{NativeClassBuilder, Value};

/// `YAML` — over the DataValue bridge (`serde_yaml_ng`, the maintained fork of the archived
/// `serde_yaml`). YAML allows any top-level value and has a native null, so no extra constraints
/// (unlike TOML). Big numbers beyond 64 bits and any BigDecimal serialize as strings (see DataValue).
pub fn build_yaml_class() -> NativeClassBuilder {
    NativeClassBuilder::new("YAML", Some("Object"))
        // YAML.parse:'…' -> a Quoin value.
        .typed_class_method("parse:", &["String"], |vm, mc, _r, args| {
            let s = arg!(args, String, 0);
            let data: DataValue = serde_yaml_ng::from_str(s.as_str())
                .map_err(|e| QuoinError::ParseError(format!("YAML.parse:: {e}")))?;
            data_to_value(&data, vm, mc)
        })
        // YAML.generate:value -> a YAML document.
        .class_method("generate:", |vm, mc, _r, args| {
            let data = value_to_data(args[0])?;
            let s = serde_yaml_ng::to_string(&data)
                .map_err(|e| QuoinError::ValueError(format!("YAML.generate:: {e}")))?;
            Ok(vm.new_string(mc, s))
        })
}
