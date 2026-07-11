use crate::arg;
use crate::error::QuoinError;
use crate::runtime::data_value::{DataValue, data_to_value, value_to_data};
use crate::value::{NativeClassBuilder, Value};

/// `YAML` — over the DataValue bridge (`serde_yaml_ng`, the maintained fork of the archived
/// `serde_yaml`). YAML allows any top-level value and has a native null, so no extra constraints
/// (unlike TOML). Big numbers beyond 64 bits and any BigDecimal serialize as strings (see DataValue).
pub fn build_yaml_class() -> NativeClassBuilder {
    NativeClassBuilder::new("YAML", Some("Object"))
        .abstract_class()
        .class_doc(
            "Parse and generate YAML documents.\n\n\
             The same value mapping as `JSON` (mapping → Map, sequence → List, plus the \
             scalars), and YAML has a native null, so `nil` round-trips. Numbers beyond the \
             64-bit range and any BigDecimal serialize as strings.\n\n\
             ```\n\
             YAML.parse:'a: 1'          \"* -> #{'a': 1}\n\
             YAML.generate:#{'a': 1}    \"* -> 'a: 1\\n'\n\
             ```",
        )
        // YAML.parse:'…' -> a Quoin value.
        .sdk_typed_class_method("parse:", &["String"], |host, _r, args| {
            let s = arg!(args, String, 0);
            let data: DataValue = serde_yaml_ng::from_str(s.as_str())
                .map_err(|e| QuoinError::ParseError(format!("YAML.parse:: {e}")))?;
            data_to_value(&data, host)
        })
        .doc(
            "Parse a YAML document into Quoin values — mapping → Map, sequence → List, plus \
             String / Integer / Double / Boolean / nil. Malformed input throws a \
             ParseError.\n\n\
             ```\n\
             (YAML.parse:'servers:\\n  - web\\n  - db').s     \"* -> #{'servers': #('web' 'db')}\n\
             ```",
        )
        // YAML.generate:value -> a YAML document.
        .class_method("generate:", |vm, mc, _r, args| {
            let data = value_to_data(vm, mc, args[0])?;
            let s = serde_yaml_ng::to_string(&data)
                .map_err(|e| QuoinError::ValueError(format!("YAML.generate:: {e}")))?;
            Ok(vm.new_string(mc, s))
        })
        .doc(
            "Generate the YAML document for a value (any top-level value is allowed, unlike \
             TOML). Serializable types match `JSON.generate:`, with `nil` allowed — YAML has \
             a native null.\n\n\
             ```\n\
             YAML.generate:#{'name': 'quoin' 'tags': #('vm' 'lang')}\n\
             \"* -> 'name: quoin\\ntags:\\n- vm\\n- lang\\n'\n\
             ```",
        )
}
