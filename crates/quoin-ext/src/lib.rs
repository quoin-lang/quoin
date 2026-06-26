//! quoin-ext — the **extension-side** SDK for out-of-process Quoin extensions
//! (Tier 1 of the extension architecture; see `docs/FUTURE_EXT_ARCH.md`).
//!
//! An extension is a separate process the Quoin VM spawns and talks to over a unix
//! domain socket. This crate is the thin per-language client an extension links
//! against — it is **not** linked into the VM. (The VM-side host API is the
//! separate in-process `ext_sdk` surface.)
//!
//! ## Wire protocol (Slice 1 — transport keystone)
//!
//! Messages are length-prefixed frames: a little-endian `u32` length followed by
//! that many payload bytes. The Slice-1 request payload is `op\0arg` (UTF-8 op
//! name, NUL, UTF-8 scalar argument); the reply payload is the result string.
//! Handles, FlatBuffers, Arrow, and batched callbacks arrive in later slices.

use std::io::{self, Read, Write};
use std::os::unix::net::UnixListener;

/// Read one length-prefixed frame. `Ok(None)` on a clean EOF (peer closed between
/// frames); `Err` on a truncated frame or other I/O error.
pub fn read_frame(r: &mut impl Read) -> io::Result<Option<Vec<u8>>> {
    let mut len_buf = [0u8; 4];
    match r.read_exact(&mut len_buf) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_le_bytes(len_buf) as usize;
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)?;
    Ok(Some(buf))
}

/// Write `payload` as one length-prefixed frame and flush.
pub fn write_frame(w: &mut impl Write, payload: &[u8]) -> io::Result<()> {
    w.write_all(&(payload.len() as u32).to_le_bytes())?;
    w.write_all(payload)?;
    w.flush()
}

/// Bind a unix socket at `path`, accept one host connection, and serve requests
/// until the host disconnects. Each request frame is `op\0arg`; `handler(op, arg)`
/// returns the reply string, sent back as one frame.
///
/// Blocking and single-connection by design: the extension is its own process, and
/// the VM holds exactly one connection to it. Returns once the host disconnects.
pub fn serve(path: &str, handler: impl Fn(&str, &str) -> String) -> io::Result<()> {
    let listener = UnixListener::bind(path)?;
    let (mut stream, _addr) = listener.accept()?;
    while let Some(frame) = read_frame(&mut stream)? {
        let text = String::from_utf8_lossy(&frame);
        let (op, arg) = text.split_once('\0').unwrap_or((text.as_ref(), ""));
        let reply = handler(op, arg);
        write_frame(&mut stream, reply.as_bytes())?;
    }
    Ok(())
}
