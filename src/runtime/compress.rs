//! Pure-Rust (de)compression for HTTP `Content-Encoding` and the `Bytes` codec methods.
//!
//! gzip + deflate go through `flate2`'s `miniz_oxide` backend (`rust_backend`); zstd is
//! *decode-only* via `ruzstd` (a pure-Rust decoder — compressing zstd would need the C
//! `libzstd`, which we avoid, same as the ring/jiff choices elsewhere). Every function is
//! a plain `&[u8] -> Result<Vec<u8>, String>`; the `Bytes` wrappers in `bytes.rs` turn an
//! `Err` into a catchable `ParseError`.

use std::io::{Read, Write};

use flate2::Compression;
use flate2::read::{DeflateDecoder, GzDecoder, ZlibDecoder};
use flate2::write::{GzEncoder, ZlibEncoder};

pub fn gzip_decode(input: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    GzDecoder::new(input)
        .read_to_end(&mut out)
        .map_err(|e| e.to_string())?;
    Ok(out)
}

pub fn gzip_encode(input: &[u8]) -> Result<Vec<u8>, String> {
    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
    enc.write_all(input).map_err(|e| e.to_string())?;
    enc.finish().map_err(|e| e.to_string())
}

/// `Content-Encoding: deflate`. Per RFC 7230 this is zlib-wrapped deflate, but many
/// servers send a *raw* deflate stream, so fall back to raw if the zlib framing fails.
pub fn deflate_decode(input: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    if ZlibDecoder::new(input).read_to_end(&mut out).is_ok() {
        return Ok(out);
    }
    out.clear();
    DeflateDecoder::new(input)
        .read_to_end(&mut out)
        .map_err(|e| e.to_string())?;
    Ok(out)
}

/// Encode as zlib-wrapped deflate (the RFC-correct form of `Content-Encoding: deflate`).
pub fn deflate_encode(input: &[u8]) -> Result<Vec<u8>, String> {
    let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
    enc.write_all(input).map_err(|e| e.to_string())?;
    enc.finish().map_err(|e| e.to_string())
}

pub fn zstd_decode(input: &[u8]) -> Result<Vec<u8>, String> {
    let mut decoder = ruzstd::decoding::StreamingDecoder::new(input).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    decoder.read_to_end(&mut out).map_err(|e| e.to_string())?;
    Ok(out)
}

#[cfg(test)]
#[path = "compress_tests.rs"]
mod tests;
