//! PACKED transport of a [`DataValue`]: the whole structured value as one MessagePack blob,
//! negotiated via `GetManifest.packed_ok` / `ManifestReturn.packed_ok` (see `schema/ext.fbs`).
//!
//! Why: the boxed `DataValueBox` tree costs two table objects per node on every peer — measured
//! at ~2.5µs/element/direction in the pure-Python SDK and ~0.16µs/element even in planus
//! (profiling/wire-encoding/notes.md). One contiguous MessagePack pass replaces all of it, and
//! every mainstream language has a C-speed MessagePack codec.
//!
//! Wire mapping (the contract other SDKs implement):
//! - `Null`/`Bool`/`Int`/`Float`/`Str`/`Bytes`/`List`/`Map` are native MessagePack
//!   (nil / bool / int64 / float64 / str / bin / array / map-with-str-keys).
//! - `BigInt` is ext type 1, payload = ASCII decimal digits.
//! - `Decimal` is ext type 2, payload = ASCII decimal string.
//!
//! Hand-rolled (both directions) rather than pulling a MessagePack crate: the subset is small,
//! the format is frozen by the spec, and this keeps the protocol crate dependency-free.

use crate::{DataValue, MAX_DV_DEPTH};

// ---------------------------------------------------------------------------------------------
// Encode
// ---------------------------------------------------------------------------------------------

/// Serialize one [`DataValue`] as a MessagePack blob.
pub fn pack_dv(dv: &DataValue) -> Vec<u8> {
    let mut out = Vec::with_capacity(64);
    write_dv(&mut out, dv);
    out
}

fn write_dv(out: &mut Vec<u8>, dv: &DataValue) {
    match dv {
        DataValue::Null => out.push(0xc0),
        DataValue::Bool(false) => out.push(0xc2),
        DataValue::Bool(true) => out.push(0xc3),
        DataValue::Int(i) => write_int(out, *i),
        DataValue::Float(f) => {
            out.push(0xcb);
            out.extend_from_slice(&f.to_be_bytes());
        }
        DataValue::Str(s) => {
            write_str(out, s);
        }
        DataValue::Bytes(b) => {
            match b.len() {
                n if n < 0x100 => {
                    out.push(0xc4);
                    out.push(n as u8);
                }
                n if n < 0x1_0000 => {
                    out.push(0xc5);
                    out.extend_from_slice(&(n as u16).to_be_bytes());
                }
                n => {
                    out.push(0xc6);
                    out.extend_from_slice(&(n as u32).to_be_bytes());
                }
            }
            out.extend_from_slice(b);
        }
        DataValue::BigInt(s) => write_ext(out, 1, s.as_bytes()),
        DataValue::Decimal(s) => write_ext(out, 2, s.as_bytes()),
        DataValue::List(items) => {
            match items.len() {
                n if n < 16 => out.push(0x90 | n as u8),
                n if n < 0x1_0000 => {
                    out.push(0xdc);
                    out.extend_from_slice(&(n as u16).to_be_bytes());
                }
                n => {
                    out.push(0xdd);
                    out.extend_from_slice(&(n as u32).to_be_bytes());
                }
            }
            for it in items {
                write_dv(out, it);
            }
        }
        DataValue::Map(entries) => {
            match entries.len() {
                n if n < 16 => out.push(0x80 | n as u8),
                n if n < 0x1_0000 => {
                    out.push(0xde);
                    out.extend_from_slice(&(n as u16).to_be_bytes());
                }
                n => {
                    out.push(0xdf);
                    out.extend_from_slice(&(n as u32).to_be_bytes());
                }
            }
            for (k, v) in entries {
                write_str(out, k);
                write_dv(out, v);
            }
        }
    }
}

/// Smallest-form MessagePack integer.
fn write_int(out: &mut Vec<u8>, i: i64) {
    if (0..=0x7f).contains(&i) {
        out.push(i as u8);
    } else if (-32..0).contains(&i) {
        out.push(i as u8); // 111xxxxx negative fixint
    } else if i >= 0 {
        match i {
            _ if i <= 0xff => {
                out.push(0xcc);
                out.push(i as u8);
            }
            _ if i <= 0xffff => {
                out.push(0xcd);
                out.extend_from_slice(&(i as u16).to_be_bytes());
            }
            _ if i <= 0xffff_ffff => {
                out.push(0xce);
                out.extend_from_slice(&(i as u32).to_be_bytes());
            }
            _ => {
                out.push(0xcf);
                out.extend_from_slice(&(i as u64).to_be_bytes());
            }
        }
    } else {
        match i {
            _ if i >= -0x80 => {
                out.push(0xd0);
                out.push(i as i8 as u8);
            }
            _ if i >= -0x8000 => {
                out.push(0xd1);
                out.extend_from_slice(&(i as i16).to_be_bytes());
            }
            _ if i >= -0x8000_0000 => {
                out.push(0xd2);
                out.extend_from_slice(&(i as i32).to_be_bytes());
            }
            _ => {
                out.push(0xd3);
                out.extend_from_slice(&i.to_be_bytes());
            }
        }
    }
}

fn write_str(out: &mut Vec<u8>, s: &str) {
    let b = s.as_bytes();
    match b.len() {
        n if n < 32 => out.push(0xa0 | n as u8),
        n if n < 0x100 => {
            out.push(0xd9);
            out.push(n as u8);
        }
        n if n < 0x1_0000 => {
            out.push(0xda);
            out.extend_from_slice(&(n as u16).to_be_bytes());
        }
        n => {
            out.push(0xdb);
            out.extend_from_slice(&(n as u32).to_be_bytes());
        }
    }
    out.extend_from_slice(b);
}

fn write_ext(out: &mut Vec<u8>, ty: i8, payload: &[u8]) {
    match payload.len() {
        1 => out.push(0xd4),
        2 => out.push(0xd5),
        4 => out.push(0xd6),
        8 => out.push(0xd7),
        16 => out.push(0xd8),
        n if n < 0x100 => {
            out.push(0xc7);
            out.push(n as u8);
        }
        n if n < 0x1_0000 => {
            out.push(0xc8);
            out.extend_from_slice(&(n as u16).to_be_bytes());
        }
        n => {
            out.push(0xc9);
            out.extend_from_slice(&(n as u32).to_be_bytes());
        }
    }
    out.push(ty as u8);
    out.extend_from_slice(payload);
}

// ---------------------------------------------------------------------------------------------
// Decode
// ---------------------------------------------------------------------------------------------

/// Deserialize one MessagePack blob back into a [`DataValue`]. Enforces the same nesting-depth
/// cap as the boxed decoder (a deep value from a buggy peer must not overflow the host stack)
/// and rejects trailing garbage.
pub fn unpack_dv(bytes: &[u8]) -> Result<DataValue, String> {
    let mut rd = bytes;
    let v = read_dv(&mut rd, 0)?;
    if !rd.is_empty() {
        return Err(format!(
            "extension protocol: packed DataValue has {} trailing bytes",
            rd.len()
        ));
    }
    Ok(v)
}

fn take<'a>(rd: &mut &'a [u8], n: usize) -> Result<&'a [u8], String> {
    if rd.len() < n {
        return Err("extension protocol: truncated packed DataValue".to_string());
    }
    let (head, rest) = rd.split_at(n);
    *rd = rest;
    Ok(head)
}

fn take_u8(rd: &mut &[u8]) -> Result<u8, String> {
    Ok(take(rd, 1)?[0])
}

fn take_u16(rd: &mut &[u8]) -> Result<u16, String> {
    Ok(u16::from_be_bytes(take(rd, 2)?.try_into().unwrap()))
}

fn take_u32(rd: &mut &[u8]) -> Result<u32, String> {
    Ok(u32::from_be_bytes(take(rd, 4)?.try_into().unwrap()))
}

fn read_dv(rd: &mut &[u8], depth: usize) -> Result<DataValue, String> {
    if depth > MAX_DV_DEPTH {
        return Err(format!(
            "extension protocol: packed DataValue nesting exceeds the {MAX_DV_DEPTH}-level decode limit"
        ));
    }
    let m = take_u8(rd)?;
    Ok(match m {
        0x00..=0x7f => DataValue::Int(m as i64),
        0xe0..=0xff => DataValue::Int(m as i8 as i64),
        0xc0 => DataValue::Null,
        0xc2 => DataValue::Bool(false),
        0xc3 => DataValue::Bool(true),
        0xcc => DataValue::Int(take_u8(rd)? as i64),
        0xcd => DataValue::Int(take_u16(rd)? as i64),
        0xce => DataValue::Int(take_u32(rd)? as i64),
        0xcf => {
            let v = u64::from_be_bytes(take(rd, 8)?.try_into().unwrap());
            i64::try_from(v)
                .map(DataValue::Int)
                // A uint64 beyond i64 is out of DataValue's Int range; keep the value (as BigInt)
                // rather than reject — a C-side packer may emit it for large positive ints.
                .unwrap_or_else(|_| DataValue::BigInt(v.to_string()))
        }
        0xd0 => DataValue::Int(take_u8(rd)? as i8 as i64),
        0xd1 => DataValue::Int(take_u16(rd)? as i16 as i64),
        0xd2 => DataValue::Int(take_u32(rd)? as i32 as i64),
        0xd3 => DataValue::Int(i64::from_be_bytes(take(rd, 8)?.try_into().unwrap())),
        0xca => DataValue::Float(f32::from_be_bytes(take(rd, 4)?.try_into().unwrap()) as f64),
        0xcb => DataValue::Float(f64::from_be_bytes(take(rd, 8)?.try_into().unwrap())),
        0xa0..=0xbf => read_str(rd, (m & 0x1f) as usize)?,
        0xd9 => {
            let n = take_u8(rd)? as usize;
            read_str(rd, n)?
        }
        0xda => {
            let n = take_u16(rd)? as usize;
            read_str(rd, n)?
        }
        0xdb => {
            let n = take_u32(rd)? as usize;
            read_str(rd, n)?
        }
        0xc4 => {
            let n = take_u8(rd)? as usize;
            DataValue::Bytes(take(rd, n)?.to_vec())
        }
        0xc5 => {
            let n = take_u16(rd)? as usize;
            DataValue::Bytes(take(rd, n)?.to_vec())
        }
        0xc6 => {
            let n = take_u32(rd)? as usize;
            DataValue::Bytes(take(rd, n)?.to_vec())
        }
        0x90..=0x9f => read_list(rd, (m & 0x0f) as usize, depth)?,
        0xdc => {
            let n = take_u16(rd)? as usize;
            read_list(rd, n, depth)?
        }
        0xdd => {
            let n = take_u32(rd)? as usize;
            read_list(rd, n, depth)?
        }
        0x80..=0x8f => read_map(rd, (m & 0x0f) as usize, depth)?,
        0xde => {
            let n = take_u16(rd)? as usize;
            read_map(rd, n, depth)?
        }
        0xdf => {
            let n = take_u32(rd)? as usize;
            read_map(rd, n, depth)?
        }
        0xd4 => read_ext(rd, 1)?,
        0xd5 => read_ext(rd, 2)?,
        0xd6 => read_ext(rd, 4)?,
        0xd7 => read_ext(rd, 8)?,
        0xd8 => read_ext(rd, 16)?,
        0xc7 => {
            let n = take_u8(rd)? as usize;
            read_ext(rd, n)?
        }
        0xc8 => {
            let n = take_u16(rd)? as usize;
            read_ext(rd, n)?
        }
        0xc9 => {
            let n = take_u32(rd)? as usize;
            read_ext(rd, n)?
        }
        other => {
            return Err(format!(
                "extension protocol: unsupported MessagePack marker 0x{other:02x} in packed DataValue"
            ));
        }
    })
}

fn read_str(rd: &mut &[u8], n: usize) -> Result<DataValue, String> {
    let b = take(rd, n)?;
    std::str::from_utf8(b)
        .map(|s| DataValue::Str(s.to_string()))
        .map_err(|_| "extension protocol: packed DataValue string is not UTF-8".to_string())
}

fn read_list(rd: &mut &[u8], n: usize, depth: usize) -> Result<DataValue, String> {
    // Cap the pre-allocation by what the remaining buffer could possibly hold (1 byte/element
    // minimum) so a lying length prefix can't drive a huge allocation.
    let mut items = Vec::with_capacity(n.min(rd.len()));
    for _ in 0..n {
        items.push(read_dv(rd, depth + 1)?);
    }
    Ok(DataValue::List(items))
}

fn read_map(rd: &mut &[u8], n: usize, depth: usize) -> Result<DataValue, String> {
    let mut entries = Vec::with_capacity(n.min(rd.len()));
    for _ in 0..n {
        let key = match read_dv(rd, depth + 1)? {
            DataValue::Str(s) => s,
            other => {
                return Err(format!(
                    "extension protocol: packed DataValue map key must be a string (got {other:?})"
                ));
            }
        };
        entries.push((key, read_dv(rd, depth + 1)?));
    }
    Ok(DataValue::Map(entries))
}

fn read_ext(rd: &mut &[u8], n: usize) -> Result<DataValue, String> {
    let ty = take_u8(rd)? as i8;
    let payload = take(rd, n)?;
    let digits = |what: &str| {
        std::str::from_utf8(payload)
            .map(str::to_string)
            .map_err(|_| format!("extension protocol: packed {what} payload is not UTF-8"))
    };
    match ty {
        1 => Ok(DataValue::BigInt(digits("BigInt")?)),
        2 => Ok(DataValue::Decimal(digits("Decimal")?)),
        other => Err(format!(
            "extension protocol: unknown packed DataValue ext type {other}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::{pack_dv, unpack_dv};
    use crate::DataValue as Dv;

    fn round_trip(dv: Dv) {
        assert_eq!(unpack_dv(&pack_dv(&dv)).unwrap(), dv, "round trip failed");
    }

    #[test]
    fn scalars_round_trip() {
        for dv in [
            Dv::Null,
            Dv::Bool(true),
            Dv::Bool(false),
            Dv::Int(0),
            Dv::Int(127),
            Dv::Int(128),
            Dv::Int(-1),
            Dv::Int(-32),
            Dv::Int(-33),
            Dv::Int(65536),
            Dv::Int(i64::MAX),
            Dv::Int(i64::MIN),
            Dv::Float(1.5),
            Dv::Float(f64::NAN.copysign(1.0)).clone(),
            Dv::Str(String::new()),
            Dv::Str("hello".into()),
            Dv::Str("x".repeat(40)),
            Dv::Str("y".repeat(70000)),
            Dv::Bytes(vec![]),
            Dv::Bytes(vec![0, 255, 7]),
            Dv::Bytes(vec![9; 70000]),
            Dv::BigInt("123456789012345678901234567890".into()),
            Dv::Decimal("-1.500".into()),
        ] {
            if let Dv::Float(f) = dv {
                // NaN != NaN; check the bits instead.
                let back = unpack_dv(&pack_dv(&Dv::Float(f))).unwrap();
                match back {
                    Dv::Float(g) => assert_eq!(f.to_bits(), g.to_bits()),
                    other => panic!("unexpected: {other:?}"),
                }
                continue;
            }
            round_trip(dv);
        }
    }

    #[test]
    fn structures_round_trip() {
        round_trip(Dv::List(vec![]));
        round_trip(Dv::List((0..300).map(Dv::Int).collect()));
        round_trip(Dv::Map(vec![
            ("a".into(), Dv::Int(1)),
            (
                "nested".into(),
                Dv::List(vec![Dv::Str("x".into()), Dv::Null]),
            ),
        ]));
        round_trip(Dv::Map(
            (0..40)
                .map(|i| (format!("k{i}"), Dv::Float(i as f64)))
                .collect(),
        ));
    }

    #[test]
    fn depth_cap_rejects_deep_values() {
        let mut dv = Dv::Int(1);
        for _ in 0..300 {
            dv = Dv::List(vec![dv]);
        }
        let err = unpack_dv(&pack_dv(&dv)).expect_err("deep packed value must be rejected");
        assert!(err.contains("nesting"), "unexpected error: {err}");
    }

    #[test]
    fn trailing_garbage_rejected() {
        let mut b = pack_dv(&Dv::Int(1));
        b.push(0xc0);
        assert!(unpack_dv(&b).is_err());
    }

    #[test]
    fn uint64_beyond_i64_becomes_bigint() {
        // 0xcf marker with a value above i64::MAX (a foreign packer may emit this).
        let mut b = vec![0xcf];
        b.extend_from_slice(&u64::MAX.to_be_bytes());
        assert_eq!(unpack_dv(&b).unwrap(), Dv::BigInt(u64::MAX.to_string()));
    }
}
