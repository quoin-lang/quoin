use crate::arg;
use crate::error::QuoinError;
use crate::ext_sdk::Host;
use crate::runtime::list::NativeListState;
use crate::runtime::map::NativeMapState;
use crate::value::{NativeClassBuilder, ObjectPayload, Value};

use indexmap::IndexMap;

/// `CSV` — tabular text (RFC 4180) via the `csv` crate. CSV is untyped, so `parse` yields strings;
/// `generate` stringifies each field via its `.s`. Both positional rows (List of Lists) and
/// header-keyed rows (List of Maps) are supported.
pub fn build_csv_class() -> NativeClassBuilder {
    NativeClassBuilder::new("CSV", Some("Object"))
        // CSV.parse:'a,b\n1,2' -> #( #('a' 'b') #('1' '2') ) — every field a String.
        .sdk_typed_class_method("parse:", &["String"], |host, _r, args| {
            let s = arg!(args, String, 0);
            let mut reader = csv::ReaderBuilder::new()
                .has_headers(false)
                .flexible(true)
                .from_reader(s.as_str().as_bytes());
            let mut rows = Vec::new();
            for result in reader.records() {
                let record =
                    result.map_err(|e| QuoinError::ParseError(format!("CSV.parse:: {e}")))?;
                let fields: Vec<Value> = record
                    .iter()
                    .map(|f| host.new_string(f.to_string()))
                    .collect();
                rows.push(host.new_list(fields));
            }
            Ok(host.new_list(rows))
        })
        // CSV.parseWithHeaders:'name,age\nAlice,30' -> #( #{'name': 'Alice' 'age': '30'} ). The
        // header row keys each data row (column order preserved — Maps are insertion-ordered).
        .sdk_typed_class_method("parseWithHeaders:", &["String"], |host, _r, args| {
            let s = arg!(args, String, 0);
            let mut reader = csv::ReaderBuilder::new()
                .has_headers(true)
                .flexible(true)
                .from_reader(s.as_str().as_bytes());
            let headers: Vec<String> = reader
                .headers()
                .map_err(|e| QuoinError::ParseError(format!("CSV.parseWithHeaders:: {e}")))?
                .iter()
                .map(String::from)
                .collect();
            let mut rows = Vec::new();
            for result in reader.records() {
                let record = result
                    .map_err(|e| QuoinError::ParseError(format!("CSV.parseWithHeaders:: {e}")))?;
                let mut map = IndexMap::with_capacity(headers.len());
                for (i, header) in headers.iter().enumerate() {
                    let field = record.get(i).unwrap_or("");
                    map.insert(header.clone(), host.new_string(field.to_string()));
                }
                rows.push(host.new_map(map));
            }
            Ok(host.new_list(rows))
        })
        // CSV.generate:#( #('id' 'n') #(1 2) ) -> 'id,n\n1,2\n' (fields stringified via .s).
        .sdk_typed_class_method("generate:", &["List"], |host, _r, args| {
            let rows = list_of(args[0], "CSV.generate:")?;
            let mut wtr = csv::WriterBuilder::new().from_writer(Vec::new());
            for row in rows {
                let fields = list_of(row, "CSV.generate: (each row)")?;
                let mut record: Vec<String> = Vec::with_capacity(fields.len());
                for field in fields {
                    record.push(field_to_string(host, field)?);
                }
                wtr.write_record(&record)
                    .map_err(|e| QuoinError::Other(format!("CSV.generate:: {e}")))?;
            }
            finish(host, wtr)
        })
        // CSV.generateWithHeaders:#( #{'id': 1 'n': 2} ) -> 'id,n\n1,2\n'. The header comes from
        // the first row's keys (in order); a key missing from a later row is an empty field.
        .sdk_typed_class_method("generateWithHeaders:", &["List"], |host, _r, args| {
            let rows = list_of(args[0], "CSV.generateWithHeaders:")?;
            let mut wtr = csv::WriterBuilder::new().from_writer(Vec::new());
            let first = match rows.first() {
                Some(r) => *r,
                None => return finish(host, wtr), // no rows -> empty document
            };
            let headers: Vec<String> = first
                .with_native_state::<NativeMapState, _, _>(|m| {
                    m.entries()
                        .iter()
                        .filter_map(|(_, k, _)| {
                            if let Value::Object(kobj) = k
                                && let crate::value::ObjectPayload::String(s) =
                                    &kobj.borrow().payload
                            {
                                Some((**s).clone())
                            } else {
                                None
                            }
                        })
                        .collect()
                })
                .map_err(|_| row_not_map())?;
            wtr.write_record(&headers)
                .map_err(|e| QuoinError::Other(format!("CSV.generateWithHeaders:: {e}")))?;
            for row in rows {
                let mut record: Vec<String> = Vec::with_capacity(headers.len());
                for header in &headers {
                    let field_val = row
                        .with_native_state::<NativeMapState, _, _>(|m| m.get_str(header))
                        .map_err(|_| row_not_map())?;
                    record.push(match field_val {
                        Some(v) => field_to_string(host, v)?,
                        None => String::new(),
                    });
                }
                wtr.write_record(&record)
                    .map_err(|e| QuoinError::Other(format!("CSV.generateWithHeaders:: {e}")))?;
            }
            finish(host, wtr)
        })
}

fn list_of<'gc>(v: Value<'gc>, who: &str) -> Result<Vec<Value<'gc>>, QuoinError> {
    v.with_native_state::<NativeListState, _, _>(|l| l.get_vec().to_vec())
        .map_err(|_| QuoinError::TypeError {
            expected: "List".to_string(),
            got: v.type_name().to_string(),
            msg: format!("{who} expects a List"),
        })
}

fn row_not_map() -> QuoinError {
    QuoinError::TypeError {
        expected: "Map".to_string(),
        got: "a non-Map row".to_string(),
        msg: "CSV.generateWithHeaders: expects a List of Maps".to_string(),
    }
}

/// A field as its CSV text — the value's `.s` (so numbers/bools/etc. become text).
fn field_to_string<'gc>(host: &mut dyn Host<'gc>, field: Value<'gc>) -> Result<String, QuoinError> {
    let s_val = host.call_method(field, "s", vec![])?;
    if let Value::Object(obj) = s_val
        && let ObjectPayload::String(s) = &obj.borrow().payload
    {
        return Ok((**s).clone());
    }
    Ok(format!("{s_val}"))
}

fn finish<'gc>(host: &dyn Host<'gc>, wtr: csv::Writer<Vec<u8>>) -> Result<Value<'gc>, QuoinError> {
    let bytes = wtr
        .into_inner()
        .map_err(|e| QuoinError::Other(format!("CSV generate: {e}")))?;
    let s =
        String::from_utf8(bytes).map_err(|e| QuoinError::Other(format!("CSV generate: {e}")))?;
    Ok(host.new_string(s))
}
