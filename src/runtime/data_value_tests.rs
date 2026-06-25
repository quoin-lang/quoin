use super::*;

// Serialize is format-generic and deterministic; the full Value <-> DataValue <-> bytes round-trip
// is exercised end-to-end by the MessagePack/TOML/YAML qnlib suites (which need a VM + a format
// without serde_json's arbitrary_precision quirk).

#[test]
fn serialize_scalars_and_structure() {
    let dv = DataValue::Object(vec![
        ("b".to_string(), DataValue::Bool(true)),
        ("n".to_string(), DataValue::Int(42)),
        ("f".to_string(), DataValue::Float(1.5)),
        ("s".to_string(), DataValue::Str("hi".to_string())),
        ("nil".to_string(), DataValue::Null),
        (
            "arr".to_string(),
            DataValue::Array(vec![DataValue::Int(1), DataValue::Int(2)]),
        ),
    ]);
    assert_eq!(
        serde_json::to_string(&dv).unwrap(),
        r#"{"b":true,"n":42,"f":1.5,"s":"hi","nil":null,"arr":[1,2]}"#
    );
}

#[test]
fn bigint_uses_int_when_it_fits_else_a_string() {
    assert_eq!(
        serde_json::to_string(&DataValue::BigInt(BigInt::from(42))).unwrap(),
        "42"
    );
    let big: BigInt = "99999999999999999999999999".parse().unwrap();
    assert_eq!(
        serde_json::to_string(&DataValue::BigInt(big)).unwrap(),
        r#""99999999999999999999999999""#
    );
}

#[test]
fn decimal_serializes_as_its_exact_string() {
    let d: Decimal = "1.50".parse().unwrap();
    assert_eq!(
        serde_json::to_string(&DataValue::Decimal(d)).unwrap(),
        r#""1.50""#
    );
}
