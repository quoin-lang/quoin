//! Stream-codec factories — THE extension point for streaming (de)compression.
//!
//! A codec is a pure function from stream to stream; everything async lives in
//! the one generic `IoRequest::WrapStream` op (io_backend.rs), written once.
//! Adding a codec means adding a table entry + factory HERE (plus a qnlib sugar
//! method over `codecWrap:'name'`) — the request enum, labels, scheduler, and
//! backend never change again. Parameterized wraps (TLS, with its SNI domain)
//! stay bespoke ops; this table is for nullary transforms.

use crate::io_backend::{AsyncStream, IoError, ReadOnlyStream, WriteOnlyStream};

use async_compression::futures::bufread::GzipDecoder;
use async_compression::futures::write::GzipEncoder;
use futures_lite::io::BufReader;

pub type WrapFn = fn(Box<dyn AsyncStream>) -> Box<dyn AsyncStream>;

/// Which half of the stream a codec transforms — and therefore which close
/// discipline the wrapped stream needs. `Read` decoders hold no unwritten
/// state: dropping the fd is a complete close, so the ordinary reap path
/// suffices. `Write` encoders finish on `poll_close` (gzip writes its final
/// deflate block + trailer there), so the backend records the id and every
/// close path routes it through `IoRequest::FinishStream` instead of a drop.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    Read,
    Write,
}

const CODECS: &[(&str, Side, WrapFn)] = &[
    // Streaming gunzip for reads: `[IO]File.open:'x.tar.gz' … byteStream.gunzip`.
    ("gunzip", Side::Read, wrap_gunzip),
    // Streaming gzip for writes: `[IO]File.create:'x.tar.gz' … byteStream.gzip`.
    ("gzip", Side::Write, wrap_gzip),
];

/// The named codec's side + factory — looked up BEFORE the stream is taken from
/// the registry, so an unknown codec (the one place a typo'd `codecWrap:`
/// surfaces) leaves the stream untouched instead of dropping its fd. The
/// runtime's `codec_wrap` also calls this early, to validate the receiver
/// against the side (read codecs wrap unread read streams, write codecs wrap
/// unwritten file write streams).
pub fn lookup(codec: &str) -> Result<(Side, WrapFn), IoError> {
    CODECS
        .iter()
        .find(|(name, _, _)| *name == codec)
        .map(|(_, side, f)| (*side, *f))
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

fn wrap_gzip(stream: Box<dyn AsyncStream>) -> Box<dyn AsyncStream> {
    // The encoder is write-only — reads answer Unsupported. Its `poll_close`
    // finishes the deflate stream (final block + trailer) into the inner
    // stream and closes that too; `FinishStream` is what drives it.
    Box::new(WriteOnlyStream(GzipEncoder::new(stream)))
}
