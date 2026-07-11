//! Stream-codec factories — THE extension point for streaming (de)compression.
//!
//! A codec is a pure function from stream to stream; everything async lives in
//! the one generic `IoRequest::WrapStream` op (io_backend.rs), written once.
//! Adding a codec means adding a table entry + factory HERE (plus a qnlib sugar
//! method over `codecWrap:'name'`) — the request enum, labels, scheduler, and
//! backend never change again. Parameterized wraps (TLS, with its SNI domain)
//! stay bespoke ops; this table is for nullary transforms.

use crate::io_backend::{AsyncStream, IoError, ReadOnlyStream};

use async_compression::futures::bufread::GzipDecoder;
use futures_lite::io::BufReader;

pub type WrapFn = fn(Box<dyn AsyncStream>) -> Box<dyn AsyncStream>;

const CODECS: &[(&str, WrapFn)] = &[
    // Streaming gunzip for reads: `[IO]File.open:'x.tar.gz' … byteStream.gunzip`. The
    // write side ("gzip", compress-on-write) is deliberately absent until the
    // close path can FINISH the encoder — the gzip trailer is written by the
    // encoder's close, and the reap path only drops fds, which would corrupt
    // every archive written through a collected handle.
    ("gunzip", wrap_gunzip),
];

/// The factory for the named codec — looked up BEFORE the stream is taken from
/// the registry, so an unknown codec (the one place a typo'd `codecWrap:`
/// surfaces) leaves the stream untouched instead of dropping its fd.
pub fn lookup(codec: &str) -> Result<WrapFn, IoError> {
    CODECS
        .iter()
        .find(|(name, _)| *name == codec)
        .map(|(_, f)| *f)
        .ok_or_else(|| IoError {
            kind: std::io::ErrorKind::InvalidInput,
            message: format!("unknown stream codec '{codec}'"),
        })
}

fn wrap_gunzip(stream: Box<dyn AsyncStream>) -> Box<dyn AsyncStream> {
    // multiple_members: real .gz files (and .tar.gz from some producers) are
    // sometimes several concatenated members; decode them all, not just the
    // first. The decoder is read-only — writes answer Unsupported.
    let mut decoder = GzipDecoder::new(BufReader::new(stream));
    decoder.multiple_members(true);
    Box::new(ReadOnlyStream(decoder))
}
