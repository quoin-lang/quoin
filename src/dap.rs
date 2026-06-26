//! DAP (Debug Adapter Protocol) wire layer for `qn debug --dap`.
//!
//! Hand-rolled over `serde_json` (no DAP crate): the `Content-Length` framing, the
//! request/response/event envelopes, an outgoing-sequence counter, and the protocol-stdout
//! redirect. This is the transport only — request dispatch and the run loop land in a later phase.
//!
//! The editor → adapter direction is always *requests*; the adapter → editor direction is
//! *responses* (one per request) and *events* (unsolicited, e.g. `stopped`/`output`).

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use std::fs::File;
use std::io::{self, BufRead, Write};

/// Redirect process stdout (fd 1) to stderr (fd 2) and return a `File` over the *original* stdout
/// — the private channel for DAP protocol writes. After this, a stray `println!`/`print!` lands
/// on stderr, never the protocol stream. Call once, before anything writes to stdout. Unix-only
/// (the DAP adapter targets the same platforms the VM runs on).
#[cfg(unix)]
pub fn redirect_protocol_stdout() -> io::Result<File> {
    use std::os::fd::FromRawFd;
    // SAFETY: fds 1 and 2 are open for the process lifetime. We dup fd 1 to a fresh fd (the saved
    // protocol channel) *before* repointing fd 1 at fd 2, so no protocol writes are lost; on the
    // dup2 failure path we close the saved fd to avoid leaking it.
    let saved = unsafe { libc::dup(libc::STDOUT_FILENO) };
    if saved < 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::dup2(libc::STDERR_FILENO, libc::STDOUT_FILENO) } < 0 {
        let err = io::Error::last_os_error();
        unsafe { libc::close(saved) };
        return Err(err);
    }
    Ok(unsafe { File::from_raw_fd(saved) })
}

// ---- message envelopes ----

/// An incoming DAP request (editor → adapter). Its `type` is always `"request"`.
#[derive(Debug, Clone, Deserialize)]
pub struct Request {
    pub seq: i64,
    pub command: String,
    /// Command arguments; `Null` when the request carries none.
    #[serde(default)]
    pub arguments: Json,
}

/// An outgoing response to a [`Request`].
#[derive(Debug, Clone, Serialize)]
pub struct Response {
    pub seq: i64,
    #[serde(rename = "type")]
    pub msg_type: &'static str, // always "response"
    pub request_seq: i64,
    pub success: bool,
    pub command: String,
    /// A short error message when `success` is false (shown by the client).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<Json>,
}

/// An outgoing, unsolicited event (adapter → editor).
#[derive(Debug, Clone, Serialize)]
pub struct Event {
    pub seq: i64,
    #[serde(rename = "type")]
    pub msg_type: &'static str, // always "event"
    pub event: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<Json>,
}

// ---- Content-Length framing ----

/// Read one `Content-Length`-framed message body off `reader`. Returns `None` at a clean EOF
/// (client disconnected between messages). Other headers are tolerated and ignored.
fn read_frame(reader: &mut impl BufRead) -> io::Result<Option<Vec<u8>>> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            // EOF: clean only if it lands on a message boundary (no header seen yet).
            return if content_length.is_none() {
                Ok(None)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "EOF in the middle of a DAP message header",
                ))
            };
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break; // blank line terminates the header block
        }
        if let Some(v) = trimmed.strip_prefix("Content-Length:") {
            content_length = Some(v.trim().parse().map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "invalid Content-Length header")
            })?);
        }
    }
    let len = content_length.ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "missing Content-Length header")
    })?;
    let mut body = vec![0u8; len];
    io::Read::read_exact(reader, &mut body)?;
    Ok(Some(body))
}

/// Write one `Content-Length`-framed message and flush.
fn write_frame(writer: &mut impl Write, body: &[u8]) -> io::Result<()> {
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
    writer.write_all(body)?;
    writer.flush()
}

/// The DAP wire connection: reads requests from `reader`, writes responses/events to `writer`
/// (the redirected protocol channel), assigning outgoing `seq` numbers.
pub struct Connection<R: BufRead, W: Write> {
    reader: R,
    writer: W,
    next_seq: i64,
}

impl<R: BufRead, W: Write> Connection<R, W> {
    pub fn new(reader: R, writer: W) -> Self {
        Self {
            reader,
            writer,
            next_seq: 1,
        }
    }

    fn next_seq(&mut self) -> i64 {
        let s = self.next_seq;
        self.next_seq += 1;
        s
    }

    /// Read the next request, or `None` at clean EOF (client disconnected).
    pub fn read_request(&mut self) -> io::Result<Option<Request>> {
        match read_frame(&mut self.reader)? {
            Some(body) => serde_json::from_slice(&body)
                .map(Some)
                .map_err(io::Error::other),
            None => Ok(None),
        }
    }

    /// Send a response to `req`. `body` is the command-specific payload (omitted when `None`);
    /// `message` is a short error string for an unsuccessful response.
    pub fn respond(
        &mut self,
        req: &Request,
        success: bool,
        body: Option<Json>,
        message: Option<String>,
    ) -> io::Result<()> {
        let resp = Response {
            seq: self.next_seq(),
            msg_type: "response",
            request_seq: req.seq,
            success,
            command: req.command.clone(),
            message,
            body,
        };
        write_frame(
            &mut self.writer,
            &serde_json::to_vec(&resp).map_err(io::Error::other)?,
        )
    }

    /// A successful response with an optional body.
    pub fn ok(&mut self, req: &Request, body: Option<Json>) -> io::Result<()> {
        self.respond(req, true, body, None)
    }

    /// An unsuccessful response carrying an error message.
    pub fn fail(&mut self, req: &Request, message: impl Into<String>) -> io::Result<()> {
        self.respond(req, false, None, Some(message.into()))
    }

    /// Send an unsolicited event (e.g. `stopped`, `output`, `terminated`).
    pub fn event(&mut self, event: &str, body: Option<Json>) -> io::Result<()> {
        let ev = Event {
            seq: self.next_seq(),
            msg_type: "event",
            event: event.to_string(),
            body,
        };
        write_frame(
            &mut self.writer,
            &serde_json::to_vec(&ev).map_err(io::Error::other)?,
        )
    }

    /// Consume the connection and return its writer (used by tests to inspect output).
    pub fn into_writer(self) -> W {
        self.writer
    }
}

#[cfg(test)]
#[path = "dap_tests.rs"]
mod tests;
