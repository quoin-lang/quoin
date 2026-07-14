//! The hand-rolled MessagePack wire codec: [`encode`] / [`decode_frame`] for whole frames,
//! [`pack_dv`] / [`unpack_dv`] for a bare [`DataValue`] (the value-level subset, kept public
//! for tests and tools).
//!
//! One frame is one MessagePack array `[type, field, ...]` — the layouts live in
//! `PROTOCOL.md`. Value mapping: `Null`/`Bool`/`Int`/`Float`/`Str`/`Bytes`/`List`/`Map` are
//! native MessagePack (nil / bool / int64 / float64 / str / bin / array / map-with-str-keys);
//! `BigInt` is ext type 1 (ASCII decimal digits); `Decimal` is ext type 2 (ASCII decimal
//! string).
//!
//! Evolution contract: message fields are append-only. A decoder reads the fields it knows
//! and *skips* any well-formed trailing extras ([`skip_value`]), so a newer peer can add
//! fields without breaking an older one; an unknown frame *type* is a hard error (the
//! version handshake in `GetManifest`/`ManifestReturn` catches real mismatches first).
//!
//! Hand-rolled rather than pulling a MessagePack crate: the subset is small, the format is
//! frozen by the spec, and this keeps the protocol crate dependency-free.

use crate::{Arg, ArrowArray, ArrowDType, ClassDecl, DataValue, MAX_DV_DEPTH, Msg, ReplyMeta};

// Frame type tags (`[tag, ...]`), grouped: the call, its terminal returns, the manifest
// handshake, then re-entrant host-ops and their replies. Append new types; never renumber.
const T_CALL: u64 = 0;
const T_CALL_RETURN: u64 = 1;
const T_CALL_RETURN_ERROR: u64 = 2;
const T_CALL_RETURN_RESOURCE: u64 = 3;
const T_CALL_RETURN_ARRAY: u64 = 4;
const T_CALL_RETURN_DATA: u64 = 5;
const T_CALL_RETURN_HANDLE: u64 = 6;
const T_GET_MANIFEST: u64 = 7;
const T_MANIFEST_RETURN: u64 = 8;
const T_MAKE_STRING: u64 = 9;
const T_HANDLE_TO_STRING: u64 = 10;
const T_RETAIN: u64 = 11;
const T_RELEASE: u64 = 12;
const T_CALL_METHOD_ON_HANDLE: u64 = 13;
const T_INVOKE_BLOCK: u64 = 14;
const T_INVOKE_BLOCK_RETURN: u64 = 15;
const T_GET_GLOBAL: u64 = 16;
const T_MAKE_VALUE: u64 = 17;
const T_READ_HANDLE: u64 = 18;
const T_READ_HANDLE_RETURN: u64 = 19;
const T_HOST_OP_RETURN: u64 = 20;
const T_CALL_RETURN_CHANNEL: u64 = 21;
const T_CHAN: u64 = 22;

// ---------------------------------------------------------------------------------------------
// Encode
// ---------------------------------------------------------------------------------------------

/// Encode one [`Msg`] as a complete MessagePack frame (no length prefix — the transport
/// frames it).
pub fn encode(msg: &Msg) -> Vec<u8> {
    encode_with_meta(msg, None)
}

/// [`encode`], plus appended [`ReplyMeta`] on the `CallReturn*` terminals (the field is
/// simply not written when `meta` is `None` or the message is not a Call terminal —
/// append-only evolution, older decoders skip it).
pub fn encode_with_meta(msg: &Msg, meta: Option<&ReplyMeta>) -> Vec<u8> {
    // The meta field count this frame will carry: 1 on a Call terminal with meta given.
    let meta_n = match msg {
        Msg::CallReturn { .. }
        | Msg::CallReturnError { .. }
        | Msg::CallReturnResource { .. }
        | Msg::CallReturnArray { .. }
        | Msg::CallReturnData { .. }
        | Msg::CallReturnHandle { .. }
        | Msg::CallReturnChannel { .. } => usize::from(meta.is_some()),
        _ => 0,
    };
    let mut out = Vec::with_capacity(64);
    match msg {
        Msg::Call {
            op,
            arg,
            handles,
            resources,
            releases,
            arrays,
            data,
            class_name,
            recv,
            method_args,
        } => {
            write_array_header(&mut out, 11);
            write_uint(&mut out, T_CALL);
            write_str(&mut out, op);
            write_str(&mut out, arg);
            write_u64_vec(&mut out, handles);
            write_u64_vec(&mut out, resources);
            write_u64_vec(&mut out, releases);
            write_array_header(&mut out, arrays.len());
            for a in arrays {
                write_arrow(&mut out, a);
            }
            // `None` is nil on the wire; `Some(Null)` also encodes as nil (the two are
            // indistinguishable to every SDK surface, so the wire does not distinguish them).
            match data {
                Some(dv) if !matches!(dv, DataValue::Null) => write_dv(&mut out, dv),
                _ => out.push(0xc0),
            }
            write_str(&mut out, class_name);
            write_uint(&mut out, *recv);
            write_array_header(&mut out, method_args.len());
            for a in method_args {
                write_arg(&mut out, a);
            }
        }
        Msg::CallReturn { result } => {
            write_array_header(&mut out, 2 + meta_n);
            write_uint(&mut out, T_CALL_RETURN);
            write_str(&mut out, result);
        }
        Msg::CallReturnError {
            message,
            remote_stack,
        } => {
            write_array_header(&mut out, 3 + meta_n);
            write_uint(&mut out, T_CALL_RETURN_ERROR);
            write_str(&mut out, message);
            write_str(&mut out, remote_stack);
        }
        Msg::CallReturnResource {
            resource,
            class_name,
        } => {
            write_array_header(&mut out, 3 + meta_n);
            write_uint(&mut out, T_CALL_RETURN_RESOURCE);
            write_uint(&mut out, *resource);
            write_str(&mut out, class_name);
        }
        Msg::CallReturnChannel { chan } => {
            write_array_header(&mut out, 2 + meta_n);
            write_uint(&mut out, T_CALL_RETURN_CHANNEL);
            write_uint(&mut out, *chan);
        }
        Msg::Chan {
            kind,
            chan,
            corr,
            value,
            message,
        } => {
            write_array_header(&mut out, 6);
            write_uint(&mut out, T_CHAN);
            write_uint(&mut out, *kind as u64);
            write_uint(&mut out, *chan);
            write_uint(&mut out, *corr);
            match value {
                Some(dv) if !matches!(dv, DataValue::Null) => write_dv(&mut out, dv),
                _ => out.push(0xc0),
            }
            write_str(&mut out, message);
        }
        Msg::CallReturnArray { array } => {
            write_array_header(&mut out, 2 + meta_n);
            write_uint(&mut out, T_CALL_RETURN_ARRAY);
            write_arrow(&mut out, array);
        }
        Msg::CallReturnData { value } => {
            write_array_header(&mut out, 2 + meta_n);
            write_uint(&mut out, T_CALL_RETURN_DATA);
            write_dv(&mut out, value);
        }
        Msg::CallReturnHandle { handle } => {
            write_array_header(&mut out, 2 + meta_n);
            write_uint(&mut out, T_CALL_RETURN_HANDLE);
            write_uint(&mut out, *handle);
        }
        Msg::GetManifest { version } => {
            write_array_header(&mut out, 2);
            write_uint(&mut out, T_GET_MANIFEST);
            write_uint(&mut out, *version as u64);
        }
        Msg::ManifestReturn { classes, version } => {
            write_array_header(&mut out, 3);
            write_uint(&mut out, T_MANIFEST_RETURN);
            write_uint(&mut out, *version as u64);
            write_array_header(&mut out, classes.len());
            for c in classes {
                write_class_decl(&mut out, c);
            }
        }
        Msg::MakeString { value } => {
            write_array_header(&mut out, 2);
            write_uint(&mut out, T_MAKE_STRING);
            write_str(&mut out, value);
        }
        Msg::HandleToString { handle } => {
            write_array_header(&mut out, 2);
            write_uint(&mut out, T_HANDLE_TO_STRING);
            write_uint(&mut out, *handle);
        }
        Msg::Retain { handle } => {
            write_array_header(&mut out, 2);
            write_uint(&mut out, T_RETAIN);
            write_uint(&mut out, *handle);
        }
        Msg::Release { handles } => {
            write_array_header(&mut out, 2);
            write_uint(&mut out, T_RELEASE);
            write_u64_vec(&mut out, handles);
        }
        Msg::CallMethodOnHandle {
            receiver,
            selector,
            args,
        } => {
            write_array_header(&mut out, 4);
            write_uint(&mut out, T_CALL_METHOD_ON_HANDLE);
            write_uint(&mut out, *receiver);
            write_str(&mut out, selector);
            write_u64_vec(&mut out, args);
        }
        Msg::InvokeBlock { block, batches } => {
            write_array_header(&mut out, 3);
            write_uint(&mut out, T_INVOKE_BLOCK);
            write_uint(&mut out, *block);
            write_array_header(&mut out, batches.len());
            for tuple in batches {
                write_u64_vec(&mut out, tuple);
            }
        }
        Msg::InvokeBlockReturn {
            results,
            error,
            remote_stack,
        } => {
            write_array_header(&mut out, 4);
            write_uint(&mut out, T_INVOKE_BLOCK_RETURN);
            write_u64_vec(&mut out, results);
            write_opt_str(&mut out, error);
            write_str(&mut out, remote_stack);
        }
        Msg::GetGlobal { name } => {
            write_array_header(&mut out, 2);
            write_uint(&mut out, T_GET_GLOBAL);
            write_str(&mut out, name);
        }
        Msg::MakeValue { value } => {
            write_array_header(&mut out, 2);
            write_uint(&mut out, T_MAKE_VALUE);
            write_dv(&mut out, value);
        }
        Msg::ReadHandle { handle } => {
            write_array_header(&mut out, 2);
            write_uint(&mut out, T_READ_HANDLE);
            write_uint(&mut out, *handle);
        }
        Msg::ReadHandleReturn {
            value,
            error,
            remote_stack,
        } => {
            write_array_header(&mut out, 4);
            write_uint(&mut out, T_READ_HANDLE_RETURN);
            write_dv(&mut out, value);
            write_opt_str(&mut out, error);
            write_str(&mut out, remote_stack);
        }
        Msg::HostOpReturn {
            handle,
            str,
            error,
            remote_stack,
        } => {
            write_array_header(&mut out, 5);
            write_uint(&mut out, T_HOST_OP_RETURN);
            write_uint(&mut out, *handle);
            write_opt_str(&mut out, str);
            write_opt_str(&mut out, error);
            write_str(&mut out, remote_stack);
        }
    }
    // The appended meta field, counted into the terminal arms' headers above.
    if meta_n == 1 {
        write_uint(
            &mut out,
            meta.expect("meta_n == 1 implies meta").handler_micros,
        );
    }
    out
}

/// Serialize one [`DataValue`] as a bare MessagePack blob (no frame around it).
pub fn pack_dv(dv: &DataValue) -> Vec<u8> {
    let mut out = Vec::with_capacity(64);
    write_dv(&mut out, dv);
    out
}

/// Recursive and infallible by design: `encode`/`pack_dv` are used on paths that cannot report an
/// error. That is safe because every `DataValue` the *host* packs is produced by
/// `runtime::data_value::value_to_data`, which refuses anything deeper than `MAX_SERIALIZE_DEPTH`
/// (128) — so a cyclic or enormous Quoin value can never reach here. An extension building a deep
/// `DataValue` by hand can still overflow, but only its own process, which the host already
/// isolates. Bytes arriving *from* a peer are bounded separately by `MAX_DV_DEPTH` in `read_dv`.
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
        DataValue::Str(s) => write_str(out, s),
        DataValue::Bytes(b) => write_bin(out, b),
        DataValue::BigInt(s) => write_ext(out, 1, s.as_bytes()),
        DataValue::Decimal(s) => write_ext(out, 2, s.as_bytes()),
        // Ext type 3: 8-byte little-endian object-table id, then the UTF-8 class name.
        DataValue::Resource { id, class_name } => {
            let mut payload = Vec::with_capacity(8 + class_name.len());
            payload.extend_from_slice(&id.to_le_bytes());
            payload.extend_from_slice(class_name.as_bytes());
            write_ext(out, 3, &payload);
        }
        DataValue::List(items) => {
            write_array_header(out, items.len());
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
    if i >= 0 {
        write_uint(out, i as u64);
    } else if i >= -32 {
        out.push(i as u8); // 111xxxxx negative fixint
    } else if i >= -0x80 {
        out.push(0xd0);
        out.push(i as i8 as u8);
    } else if i >= -0x8000 {
        out.push(0xd1);
        out.extend_from_slice(&(i as i16).to_be_bytes());
    } else if i >= -0x8000_0000 {
        out.push(0xd2);
        out.extend_from_slice(&(i as i32).to_be_bytes());
    } else {
        out.push(0xd3);
        out.extend_from_slice(&i.to_be_bytes());
    }
}

/// Smallest-form MessagePack unsigned integer (ids, counts, versions).
fn write_uint(out: &mut Vec<u8>, v: u64) {
    if v <= 0x7f {
        out.push(v as u8);
    } else if v <= 0xff {
        out.push(0xcc);
        out.push(v as u8);
    } else if v <= 0xffff {
        out.push(0xcd);
        out.extend_from_slice(&(v as u16).to_be_bytes());
    } else if v <= 0xffff_ffff {
        out.push(0xce);
        out.extend_from_slice(&(v as u32).to_be_bytes());
    } else {
        out.push(0xcf);
        out.extend_from_slice(&v.to_be_bytes());
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

fn write_bin(out: &mut Vec<u8>, b: &[u8]) {
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

fn write_array_header(out: &mut Vec<u8>, n: usize) {
    match n {
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
}

fn write_u64_vec(out: &mut Vec<u8>, xs: &[u64]) {
    write_array_header(out, xs.len());
    for x in xs {
        write_uint(out, *x);
    }
}

fn write_opt_str(out: &mut Vec<u8>, s: &Option<String>) {
    match s {
        Some(s) => write_str(out, s),
        None => out.push(0xc0),
    }
}

/// `ArrowArray` = `[dtype, length, data]`.
fn write_arrow(out: &mut Vec<u8>, a: &ArrowArray) {
    write_array_header(out, 3);
    write_uint(out, a.dtype as u64);
    write_uint(out, a.length);
    write_bin(out, &a.data);
}

/// `Arg` = `[kind, payload]` — kind 0 = Data (payload is a DataValue), 1 = Resource
/// (payload is the ext-side object-table id), 2 = Handle (payload is a host-value handle),
/// 3 = Array (payload is an inline `ArrowArray` — the data plane as a method argument).
fn write_arg(out: &mut Vec<u8>, a: &Arg) {
    write_array_header(out, 2);
    match a {
        Arg::Data(d) => {
            write_uint(out, 0);
            write_dv(out, d);
        }
        Arg::Resource(id) => {
            write_uint(out, 1);
            write_uint(out, *id);
        }
        Arg::Handle(h) => {
            write_uint(out, 2);
            write_uint(out, *h);
        }
        Arg::Array(a) => {
            write_uint(out, 3);
            write_arrow(out, a);
        }
        Arg::Chan(c) => {
            write_uint(out, 4);
            write_uint(out, *c);
        }
    }
}

/// `ClassDecl` = `[name, instance_selectors, class_selectors]`.
fn write_class_decl(out: &mut Vec<u8>, c: &ClassDecl) {
    write_array_header(out, 3);
    write_str(out, &c.name);
    write_array_header(out, c.instance_selectors.len());
    for s in &c.instance_selectors {
        write_str(out, s);
    }
    write_array_header(out, c.class_selectors.len());
    for s in &c.class_selectors {
        write_str(out, s);
    }
}

// ---------------------------------------------------------------------------------------------
// Decode
// ---------------------------------------------------------------------------------------------

/// Decode one received frame into an owned [`Msg`]. The error is a human-readable string
/// (a malformed buffer, an unknown frame type, or a structural limit); both the host and
/// the extension SDK wrap it in their own error type.
pub fn decode_frame(bytes: &[u8]) -> Result<Msg, String> {
    decode_frame_with_meta(bytes).map(|(msg, _)| msg)
}

/// [`decode_frame`], plus the appended [`ReplyMeta`] from `CallReturn*` terminals
/// (default — `handler_micros: 0` — for other frames and for older peers that don't
/// send the field).
pub fn decode_frame_with_meta(bytes: &[u8]) -> Result<(Msg, ReplyMeta), String> {
    let mut rd = bytes;
    let rd = &mut rd;
    let n = read_array_header(rd)?;
    if n == 0 {
        return Err("extension protocol: empty frame array".to_string());
    }
    let tag = read_uint(rd)?;
    // Fields beyond the ones this decoder knows are skipped (append-only evolution).
    let fields = n - 1;
    let mut meta = ReplyMeta::default();
    let msg = match tag {
        T_CALL => {
            let extra = need(fields, 10, "Call")?;
            let msg = Msg::Call {
                op: read_str(rd)?,
                arg: read_str(rd)?,
                handles: read_u64_vec(rd)?,
                resources: read_u64_vec(rd)?,
                releases: read_u64_vec(rd)?,
                arrays: {
                    let k = read_array_header(rd)?;
                    let mut arrays = Vec::with_capacity(k.min(rd.len()));
                    for _ in 0..k {
                        arrays.push(read_arrow(rd)?);
                    }
                    arrays
                },
                data: read_opt_dv(rd)?,
                class_name: read_str(rd)?,
                recv: read_uint(rd)?,
                method_args: {
                    let k = read_array_header(rd)?;
                    let mut method_args = Vec::with_capacity(k.min(rd.len()));
                    for _ in 0..k {
                        method_args.push(read_arg(rd)?);
                    }
                    method_args
                },
            };
            skip_extra(rd, extra)?;
            msg
        }
        T_CALL_RETURN => {
            let mut extra = need(fields, 1, "CallReturn")?;
            let msg = Msg::CallReturn {
                result: read_str(rd)?,
            };
            meta.handler_micros = read_appended_u64(rd, &mut extra)?;
            skip_extra(rd, extra)?;
            msg
        }
        T_CALL_RETURN_ERROR => {
            let mut extra = need(fields, 1, "CallReturnError")?;
            let msg = Msg::CallReturnError {
                message: read_str(rd)?,
                remote_stack: read_appended_str(rd, &mut extra)?,
            };
            meta.handler_micros = read_appended_u64(rd, &mut extra)?;
            skip_extra(rd, extra)?;
            msg
        }
        T_CALL_RETURN_RESOURCE => {
            let mut extra = need(fields, 2, "CallReturnResource")?;
            let msg = Msg::CallReturnResource {
                resource: read_uint(rd)?,
                class_name: read_str(rd)?,
            };
            meta.handler_micros = read_appended_u64(rd, &mut extra)?;
            skip_extra(rd, extra)?;
            msg
        }
        T_CALL_RETURN_CHANNEL => {
            let mut extra = need(fields, 1, "CallReturnChannel")?;
            let msg = Msg::CallReturnChannel {
                chan: read_uint(rd)?,
            };
            meta.handler_micros = read_appended_u64(rd, &mut extra)?;
            skip_extra(rd, extra)?;
            msg
        }
        T_CHAN => {
            let extra = need(fields, 5, "Chan")?;
            let msg = Msg::Chan {
                kind: read_uint(rd)? as u8,
                chan: read_uint(rd)?,
                corr: read_uint(rd)?,
                value: read_opt_dv(rd)?,
                message: read_str(rd)?,
            };
            skip_extra(rd, extra)?;
            msg
        }
        T_CALL_RETURN_ARRAY => {
            let mut extra = need(fields, 1, "CallReturnArray")?;
            let msg = Msg::CallReturnArray {
                array: read_arrow(rd)?,
            };
            meta.handler_micros = read_appended_u64(rd, &mut extra)?;
            skip_extra(rd, extra)?;
            msg
        }
        T_CALL_RETURN_DATA => {
            let mut extra = need(fields, 1, "CallReturnData")?;
            let msg = Msg::CallReturnData {
                value: read_dv(rd, 0)?,
            };
            meta.handler_micros = read_appended_u64(rd, &mut extra)?;
            skip_extra(rd, extra)?;
            msg
        }
        T_CALL_RETURN_HANDLE => {
            let mut extra = need(fields, 1, "CallReturnHandle")?;
            let msg = Msg::CallReturnHandle {
                handle: read_uint(rd)?,
            };
            meta.handler_micros = read_appended_u64(rd, &mut extra)?;
            skip_extra(rd, extra)?;
            msg
        }
        T_GET_MANIFEST => {
            let extra = need(fields, 1, "GetManifest")?;
            let msg = Msg::GetManifest {
                version: read_version(rd)?,
            };
            skip_extra(rd, extra)?;
            msg
        }
        T_MANIFEST_RETURN => {
            let extra = need(fields, 2, "ManifestReturn")?;
            let msg = Msg::ManifestReturn {
                version: read_version(rd)?,
                classes: {
                    let k = read_array_header(rd)?;
                    let mut classes = Vec::with_capacity(k.min(rd.len()));
                    for _ in 0..k {
                        classes.push(read_class_decl(rd)?);
                    }
                    classes
                },
            };
            skip_extra(rd, extra)?;
            msg
        }
        T_MAKE_STRING => {
            let extra = need(fields, 1, "MakeString")?;
            let msg = Msg::MakeString {
                value: read_str(rd)?,
            };
            skip_extra(rd, extra)?;
            msg
        }
        T_HANDLE_TO_STRING => {
            let extra = need(fields, 1, "HandleToString")?;
            let msg = Msg::HandleToString {
                handle: read_uint(rd)?,
            };
            skip_extra(rd, extra)?;
            msg
        }
        T_RETAIN => {
            let extra = need(fields, 1, "Retain")?;
            let msg = Msg::Retain {
                handle: read_uint(rd)?,
            };
            skip_extra(rd, extra)?;
            msg
        }
        T_RELEASE => {
            let extra = need(fields, 1, "Release")?;
            let msg = Msg::Release {
                handles: read_u64_vec(rd)?,
            };
            skip_extra(rd, extra)?;
            msg
        }
        T_CALL_METHOD_ON_HANDLE => {
            let extra = need(fields, 3, "CallMethodOnHandle")?;
            let msg = Msg::CallMethodOnHandle {
                receiver: read_uint(rd)?,
                selector: read_str(rd)?,
                args: read_u64_vec(rd)?,
            };
            skip_extra(rd, extra)?;
            msg
        }
        T_INVOKE_BLOCK => {
            let extra = need(fields, 2, "InvokeBlock")?;
            let msg = Msg::InvokeBlock {
                block: read_uint(rd)?,
                batches: {
                    let k = read_array_header(rd)?;
                    let mut batches = Vec::with_capacity(k.min(rd.len()));
                    for _ in 0..k {
                        batches.push(read_u64_vec(rd)?);
                    }
                    batches
                },
            };
            skip_extra(rd, extra)?;
            msg
        }
        T_INVOKE_BLOCK_RETURN => {
            let mut extra = need(fields, 2, "InvokeBlockReturn")?;
            let msg = Msg::InvokeBlockReturn {
                results: read_u64_vec(rd)?,
                error: read_opt_str(rd)?,
                remote_stack: read_appended_str(rd, &mut extra)?,
            };
            skip_extra(rd, extra)?;
            msg
        }
        T_GET_GLOBAL => {
            let extra = need(fields, 1, "GetGlobal")?;
            let msg = Msg::GetGlobal {
                name: read_str(rd)?,
            };
            skip_extra(rd, extra)?;
            msg
        }
        T_MAKE_VALUE => {
            let extra = need(fields, 1, "MakeValue")?;
            let msg = Msg::MakeValue {
                value: read_dv(rd, 0)?,
            };
            skip_extra(rd, extra)?;
            msg
        }
        T_READ_HANDLE => {
            let extra = need(fields, 1, "ReadHandle")?;
            let msg = Msg::ReadHandle {
                handle: read_uint(rd)?,
            };
            skip_extra(rd, extra)?;
            msg
        }
        T_READ_HANDLE_RETURN => {
            let mut extra = need(fields, 2, "ReadHandleReturn")?;
            let msg = Msg::ReadHandleReturn {
                value: read_dv(rd, 0)?,
                error: read_opt_str(rd)?,
                remote_stack: read_appended_str(rd, &mut extra)?,
            };
            skip_extra(rd, extra)?;
            msg
        }
        T_HOST_OP_RETURN => {
            let mut extra = need(fields, 3, "HostOpReturn")?;
            let msg = Msg::HostOpReturn {
                handle: read_uint(rd)?,
                str: read_opt_str(rd)?,
                error: read_opt_str(rd)?,
                remote_stack: read_appended_str(rd, &mut extra)?,
            };
            skip_extra(rd, extra)?;
            msg
        }
        other => {
            return Err(format!(
                "extension protocol: unknown frame type {other} — is the peer speaking a \
                 different protocol version?"
            ));
        }
    };
    if !rd.is_empty() {
        return Err(format!(
            "extension protocol: {} trailing byte(s) after frame",
            rd.len()
        ));
    }
    Ok((msg, meta))
}

/// Deserialize one bare MessagePack blob back into a [`DataValue`]. Enforces the nesting-
/// depth cap (a deep value from a buggy peer must not overflow the host stack) and rejects
/// trailing garbage.
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

/// `fields.checked_sub(expected)` with a clear too-few-fields error; the surplus is what
/// the caller must [`skip_extra`] after reading its known fields.
fn need(fields: usize, expected: usize, what: &str) -> Result<usize, String> {
    fields.checked_sub(expected).ok_or_else(|| {
        format!("extension protocol: {what} frame has {fields} field(s), needs {expected}")
    })
}

/// Read an APPENDED optional String field: present when the peer is new enough to send
/// it, an empty default when it isn't (PROTOCOL.md §Evolution — append-only fields).
fn read_appended_str(rd: &mut &[u8], extra: &mut usize) -> Result<String, String> {
    if *extra == 0 {
        return Ok(String::new());
    }
    *extra -= 1;
    read_str(rd)
}

/// Read an APPENDED optional u64 field: present when the peer is new enough to send
/// it, 0 when it isn't (PROTOCOL.md §Evolution — append-only fields).
fn read_appended_u64(rd: &mut &[u8], extra: &mut usize) -> Result<u64, String> {
    if *extra == 0 {
        return Ok(0);
    }
    *extra -= 1;
    read_uint(rd)
}

fn skip_extra(rd: &mut &[u8], extra: usize) -> Result<(), String> {
    for _ in 0..extra {
        skip_value(rd, 0)?;
    }
    Ok(())
}

fn take<'a>(rd: &mut &'a [u8], n: usize) -> Result<&'a [u8], String> {
    if rd.len() < n {
        return Err("extension protocol: truncated frame".to_string());
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

fn read_array_header(rd: &mut &[u8]) -> Result<usize, String> {
    let m = take_u8(rd)?;
    Ok(match m {
        0x90..=0x9f => (m & 0x0f) as usize,
        0xdc => take_u16(rd)? as usize,
        0xdd => take_u32(rd)? as usize,
        other => {
            return Err(format!(
                "extension protocol: expected an array, got marker 0x{other:02x}"
            ));
        }
    })
}

/// An unsigned integer (ids, counts, tags). Accepts the signed encodings too when the
/// value is non-negative — our writers never emit them here, but a foreign packer may.
fn read_uint(rd: &mut &[u8]) -> Result<u64, String> {
    let m = take_u8(rd)?;
    let signed = |v: i64| {
        u64::try_from(v).map_err(|_| {
            "extension protocol: expected an unsigned integer, got a negative".to_string()
        })
    };
    Ok(match m {
        0x00..=0x7f => m as u64,
        0xcc => take_u8(rd)? as u64,
        0xcd => take_u16(rd)? as u64,
        0xce => take_u32(rd)? as u64,
        0xcf => u64::from_be_bytes(take(rd, 8)?.try_into().unwrap()),
        0xd0 => signed(take_u8(rd)? as i8 as i64)?,
        0xd1 => signed(take_u16(rd)? as i16 as i64)?,
        0xd2 => signed(take_u32(rd)? as i32 as i64)?,
        0xd3 => signed(i64::from_be_bytes(take(rd, 8)?.try_into().unwrap()))?,
        other => {
            return Err(format!(
                "extension protocol: expected an unsigned integer, got marker 0x{other:02x}"
            ));
        }
    })
}

fn read_version(rd: &mut &[u8]) -> Result<u32, String> {
    u32::try_from(read_uint(rd)?)
        .map_err(|_| "extension protocol: protocol version out of range".to_string())
}

fn read_str_raw(rd: &mut &[u8]) -> Result<String, String> {
    let m = take_u8(rd)?;
    let n = match m {
        0xa0..=0xbf => (m & 0x1f) as usize,
        0xd9 => take_u8(rd)? as usize,
        0xda => take_u16(rd)? as usize,
        0xdb => take_u32(rd)? as usize,
        other => {
            return Err(format!(
                "extension protocol: expected a string, got marker 0x{other:02x}"
            ));
        }
    };
    let b = take(rd, n)?;
    std::str::from_utf8(b)
        .map(str::to_string)
        .map_err(|_| "extension protocol: string is not UTF-8".to_string())
}

fn read_opt_str(rd: &mut &[u8]) -> Result<Option<String>, String> {
    if rd.first() == Some(&0xc0) {
        *rd = &rd[1..];
        return Ok(None);
    }
    read_str_raw(rd).map(Some)
}

/// Field position that carries `nil | DataValue` (`Call.data`). Nil decodes to `None`.
fn read_opt_dv(rd: &mut &[u8]) -> Result<Option<DataValue>, String> {
    if rd.first() == Some(&0xc0) {
        *rd = &rd[1..];
        return Ok(None);
    }
    read_dv(rd, 0).map(Some)
}

fn read_bin(rd: &mut &[u8]) -> Result<Vec<u8>, String> {
    let m = take_u8(rd)?;
    let n = match m {
        0xc4 => take_u8(rd)? as usize,
        0xc5 => take_u16(rd)? as usize,
        0xc6 => take_u32(rd)? as usize,
        other => {
            return Err(format!(
                "extension protocol: expected binary data, got marker 0x{other:02x}"
            ));
        }
    };
    Ok(take(rd, n)?.to_vec())
}

fn read_u64_vec(rd: &mut &[u8]) -> Result<Vec<u64>, String> {
    let k = read_array_header(rd)?;
    // Cap the pre-allocation by what the remaining buffer could possibly hold (1 byte per
    // element minimum) so a lying length prefix can't drive a huge allocation.
    let mut xs = Vec::with_capacity(k.min(rd.len()));
    for _ in 0..k {
        xs.push(read_uint(rd)?);
    }
    Ok(xs)
}

fn read_str_vec(rd: &mut &[u8]) -> Result<Vec<String>, String> {
    let k = read_array_header(rd)?;
    let mut xs = Vec::with_capacity(k.min(rd.len()));
    for _ in 0..k {
        xs.push(read_str_raw(rd)?);
    }
    Ok(xs)
}

fn read_str(rd: &mut &[u8]) -> Result<String, String> {
    read_str_raw(rd)
}

fn read_arrow(rd: &mut &[u8]) -> Result<ArrowArray, String> {
    let extra = need(read_array_header(rd)?, 3, "ArrowArray")?;
    let dtype = match read_uint(rd)? {
        0 => ArrowDType::Float64,
        1 => ArrowDType::Int64,
        other => {
            return Err(format!(
                "extension protocol: unknown ArrowArray dtype {other}"
            ));
        }
    };
    let length = read_uint(rd)?;
    let data = read_bin(rd)?;
    skip_extra(rd, extra)?;
    Ok(ArrowArray {
        dtype,
        length,
        data,
    })
}

fn read_arg(rd: &mut &[u8]) -> Result<Arg, String> {
    let extra = need(read_array_header(rd)?, 2, "Arg")?;
    let arg = match read_uint(rd)? {
        0 => Arg::Data(read_dv(rd, 0)?),
        1 => Arg::Resource(read_uint(rd)?),
        2 => Arg::Handle(read_uint(rd)?),
        3 => Arg::Array(read_arrow(rd)?),
        4 => Arg::Chan(read_uint(rd)?),
        other => return Err(format!("extension protocol: unknown Arg kind {other}")),
    };
    skip_extra(rd, extra)?;
    Ok(arg)
}

fn read_class_decl(rd: &mut &[u8]) -> Result<ClassDecl, String> {
    let extra = need(read_array_header(rd)?, 3, "ClassDecl")?;
    let decl = ClassDecl {
        name: read_str_raw(rd)?,
        instance_selectors: read_str_vec(rd)?,
        class_selectors: read_str_vec(rd)?,
    };
    skip_extra(rd, extra)?;
    Ok(decl)
}

fn read_dv(rd: &mut &[u8], depth: usize) -> Result<DataValue, String> {
    if depth > MAX_DV_DEPTH {
        return Err(format!(
            "extension protocol: value nesting exceeds the {MAX_DV_DEPTH}-level decode limit"
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
        0xa0..=0xbf => read_str_dv(rd, (m & 0x1f) as usize)?,
        0xd9 => {
            let n = take_u8(rd)? as usize;
            read_str_dv(rd, n)?
        }
        0xda => {
            let n = take_u16(rd)? as usize;
            read_str_dv(rd, n)?
        }
        0xdb => {
            let n = take_u32(rd)? as usize;
            read_str_dv(rd, n)?
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
                "extension protocol: unsupported MessagePack marker 0x{other:02x} in a value"
            ));
        }
    })
}

fn read_str_dv(rd: &mut &[u8], n: usize) -> Result<DataValue, String> {
    let b = take(rd, n)?;
    std::str::from_utf8(b)
        .map(|s| DataValue::Str(s.to_string()))
        .map_err(|_| "extension protocol: string is not UTF-8".to_string())
}

fn read_list(rd: &mut &[u8], n: usize, depth: usize) -> Result<DataValue, String> {
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
                    "extension protocol: map key must be a string (got {other:?})"
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
        3 => {
            let (id, name) = payload.split_at_checked(8).ok_or_else(|| {
                "extension protocol: Resource payload shorter than its 8-byte id".to_string()
            })?;
            Ok(DataValue::Resource {
                id: u64::from_le_bytes(id.try_into().unwrap()),
                class_name: std::str::from_utf8(name)
                    .map_err(|_| {
                        "extension protocol: Resource class name is not UTF-8".to_string()
                    })?
                    .to_string(),
            })
        }
        other => Err(format!(
            "extension protocol: unknown value ext type {other}"
        )),
    }
}

/// Walk past one well-formed MessagePack value of ANY shape (including ext types this
/// decoder doesn't know) — the skipper behind append-only field evolution. Depth-capped
/// like [`read_dv`] so a hostile nest can't overflow the stack via the skip path either.
fn skip_value(rd: &mut &[u8], depth: usize) -> Result<(), String> {
    if depth > MAX_DV_DEPTH {
        return Err(format!(
            "extension protocol: value nesting exceeds the {MAX_DV_DEPTH}-level decode limit"
        ));
    }
    let m = take_u8(rd)?;
    match m {
        0x00..=0x7f | 0xe0..=0xff | 0xc0 | 0xc2 | 0xc3 => {}
        0xcc | 0xd0 => {
            take(rd, 1)?;
        }
        0xcd | 0xd1 => {
            take(rd, 2)?;
        }
        0xce | 0xd2 | 0xca => {
            take(rd, 4)?;
        }
        0xcf | 0xd3 | 0xcb => {
            take(rd, 8)?;
        }
        0xa0..=0xbf => {
            take(rd, (m & 0x1f) as usize)?;
        }
        0xd9 | 0xc4 => {
            let n = take_u8(rd)? as usize;
            take(rd, n)?;
        }
        0xda | 0xc5 => {
            let n = take_u16(rd)? as usize;
            take(rd, n)?;
        }
        0xdb | 0xc6 => {
            let n = take_u32(rd)? as usize;
            take(rd, n)?;
        }
        0xd4..=0xd8 => {
            take(rd, 1 + (1usize << (m - 0xd4)))?;
        }
        0xc7 => {
            let n = take_u8(rd)? as usize;
            take(rd, 1 + n)?;
        }
        0xc8 => {
            let n = take_u16(rd)? as usize;
            take(rd, 1 + n)?;
        }
        0xc9 => {
            let n = take_u32(rd)? as usize;
            take(rd, 1 + n)?;
        }
        0x90..=0x9f => {
            for _ in 0..(m & 0x0f) {
                skip_value(rd, depth + 1)?;
            }
        }
        0xdc => {
            let n = take_u16(rd)?;
            for _ in 0..n {
                skip_value(rd, depth + 1)?;
            }
        }
        0xdd => {
            let n = take_u32(rd)?;
            for _ in 0..n {
                skip_value(rd, depth + 1)?;
            }
        }
        0x80..=0x8f => {
            for _ in 0..(2 * (m & 0x0f) as usize) {
                skip_value(rd, depth + 1)?;
            }
        }
        0xde => {
            let n = take_u16(rd)? as usize;
            for _ in 0..(2 * n) {
                skip_value(rd, depth + 1)?;
            }
        }
        0xdf => {
            let n = take_u32(rd)? as usize;
            for _ in 0..(2 * n) {
                skip_value(rd, depth + 1)?;
            }
        }
        0xc1 => {
            return Err("extension protocol: reserved MessagePack marker 0xc1".to_string());
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "codec_tests.rs"]
mod codec_tests;
