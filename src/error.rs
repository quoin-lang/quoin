use crate::highlighter::{HighlightParser, HighlightSpan};
use crate::io_backend::IoError;
use crate::parser::parse_quoin_string;
use crate::value::SourceInfo;
use std::error::Error;
use std::{cmp, fmt, fs};

/// The category of a [`QuoinError::Io`], surfaced to Quoin code as `IoError.kind`
/// (a `#symbol`). A small, stable set rather than `std::io::ErrorKind` (which is
/// `#[non_exhaustive]` and large); OS kinds we don't name fold into `Other`. `Closed`
/// is synthetic — operating on a closed/consumed handle, which has no OS errno.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IoErrorKind {
    Closed,
    NotFound,
    PermissionDenied,
    ConnectionRefused,
    ConnectionReset,
    ConnectionAborted,
    BrokenPipe,
    AddrInUse,
    AddrNotAvailable,
    TimedOut,
    UnexpectedEof,
    InvalidInput,
    InvalidData,
    Other,
}

impl IoErrorKind {
    /// The camelCase name used for the Quoin `#symbol` (e.g. `#connectionRefused`).
    pub fn symbol(self) -> &'static str {
        match self {
            IoErrorKind::Closed => "closed",
            IoErrorKind::NotFound => "notFound",
            IoErrorKind::PermissionDenied => "permissionDenied",
            IoErrorKind::ConnectionRefused => "connectionRefused",
            IoErrorKind::ConnectionReset => "connectionReset",
            IoErrorKind::ConnectionAborted => "connectionAborted",
            IoErrorKind::BrokenPipe => "brokenPipe",
            IoErrorKind::AddrInUse => "addrInUse",
            IoErrorKind::AddrNotAvailable => "addrNotAvailable",
            IoErrorKind::TimedOut => "timedOut",
            IoErrorKind::UnexpectedEof => "unexpectedEof",
            IoErrorKind::InvalidInput => "invalidInput",
            IoErrorKind::InvalidData => "invalidData",
            IoErrorKind::Other => "other",
        }
    }
}

impl From<std::io::ErrorKind> for IoErrorKind {
    fn from(k: std::io::ErrorKind) -> Self {
        use std::io::ErrorKind as E;
        match k {
            E::NotFound => IoErrorKind::NotFound,
            E::PermissionDenied => IoErrorKind::PermissionDenied,
            E::ConnectionRefused => IoErrorKind::ConnectionRefused,
            E::ConnectionReset => IoErrorKind::ConnectionReset,
            E::ConnectionAborted => IoErrorKind::ConnectionAborted,
            E::BrokenPipe => IoErrorKind::BrokenPipe,
            E::AddrInUse => IoErrorKind::AddrInUse,
            E::AddrNotAvailable => IoErrorKind::AddrNotAvailable,
            E::TimedOut => IoErrorKind::TimedOut,
            E::UnexpectedEof => IoErrorKind::UnexpectedEof,
            E::InvalidInput => IoErrorKind::InvalidInput,
            E::InvalidData => IoErrorKind::InvalidData,
            _ => IoErrorKind::Other,
        }
    }
}

#[derive(Debug, Clone)]
pub enum QuoinError {
    /// Raised when a function or method receives the wrong number of arguments
    ArgumentCountMismatch {
        expected: usize,
        got: usize,
        msg: String,
    },
    /// Raised when a value has a type that is incompatible with the expected type
    TypeError {
        expected: String,
        got: String,
        msg: String,
    },
    /// Raised during illegal arithmetic operations (e.g. division by zero)
    ArithmeticError(String),
    /// Raised when method lookup fails. `candidates` holds the formatted signatures
    /// of any variants that *do* share the selector but were filtered out by
    /// dispatch (empty when the selector is genuinely absent) — a display-only hint.
    MessageNotUnderstood {
        receiver: String,
        selector: String,
        args: Vec<String>,
        candidates: Vec<String>,
    },
    /// Raised when two or more equally-specific method variants tie for a send,
    /// so dispatch can't pick one (see scored multimethod dispatch). `candidates`
    /// holds the formatted signatures of the tied variants.
    AmbiguousMethod {
        selector: String,
        msg: String,
        candidates: Vec<String>,
    },
    /// Raised when trying to execute a value that does not implement call/send dispatch
    NotCallable(String),
    /// Raised when attempting to pop or peek from an empty VM stack
    StackUnderflow(String),
    /// Generic other error
    Other(String),
    /// An I/O failure from a socket, stream, or file: a backend error (carrying the
    /// OS error kind), a closed/consumed-handle access (`Closed`), or an unexpected
    /// EOF. Mapped to a typed Quoin `IoError` (with a `kind` symbol) at the `catch:`
    /// boundary via `make_io_error`. Plain data only — holds no `Gc`.
    Io { kind: IoErrorKind, message: String },
    /// An out-of-bounds index access (e.g. `List.at:put:`, `Bytes.at:`). Mapped to a
    /// typed Quoin `IndexError` (exposing `index`/`length`) at the `catch:` boundary via
    /// `make_index_error`. Plain data only — holds no `Gc`.
    IndexError { index: i64, len: i64, msg: String },
    /// Marker that a Quoin-level exception value has been parked in
    /// `VmState.active_exception` (set by `throw`). Carries no payload — the
    /// thrown value travels in the GC-rooted `active_exception` slot, not here.
    Thrown,
    /// Raised to propagate non-local returns out of native call stacks
    NonLocalReturn,
    /// A task was cancelled (`handle.cancel`). Propagates like `Thrown` so `finally`
    /// blocks run during the unwind, but is deliberately *not* catchable by `catch:`
    /// (a task cannot swallow its own cancellation). Carries no payload.
    Cancelled,
    /// Wrapper containing source location for execution errors
    WithSourceInfo {
        error: Box<QuoinError>,
        source_info: SourceInfo,
        trace: Vec<String>,
        supports_color: bool,
    },
}

fn get_highlighted_range(
    filename: &str,
    start: usize,
    end: usize,
    fallback_text: &str,
    supports_color: bool,
) -> String {
    if !supports_color {
        return fallback_text.to_string();
    }
    if let Ok(content) = fs::read_to_string(filename) {
        let parse_and_highlight = || -> Option<String> {
            let program = parse_quoin_string(&content);
            let mut parser = HighlightParser::new(&content);
            let spans = parser.highlight_program(&program);

            let mut snippet_spans = Vec::new();
            for span in spans {
                let overlap_start = cmp::max(span.start, start);
                let overlap_end = cmp::min(span.end, end);
                if overlap_start < overlap_end {
                    snippet_spans.push(HighlightSpan {
                        start: overlap_start - start,
                        end: overlap_end - start,
                        htype: span.htype,
                        counter: span.counter,
                    });
                }
            }
            let snippet_text = content.get(start..end)?;
            Some(crate::highlighter::format_ansi(snippet_text, snippet_spans))
        };
        if let Some(res) = parse_and_highlight() {
            return res;
        }
    }
    let parse_and_highlight_fallback = || -> Option<String> {
        let program = parse_quoin_string(fallback_text);
        let mut parser = HighlightParser::new(fallback_text);
        let spans = parser.highlight_program(&program);
        Some(crate::highlighter::format_ansi(fallback_text, spans))
    };
    parse_and_highlight_fallback().unwrap_or_else(|| fallback_text.to_string())
}

impl fmt::Display for QuoinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            QuoinError::ArgumentCountMismatch { msg, .. } => {
                write!(f, "Argument count mismatch: {}", msg)
            }
            QuoinError::TypeError { msg, .. } => write!(f, "Type error: {}", msg),
            QuoinError::ArithmeticError(msg) => write!(f, "Arithmetic error: {}", msg),
            QuoinError::MessageNotUnderstood {
                receiver,
                selector,
                args,
                candidates,
            } => {
                write!(
                    f,
                    "Message not understood: receiver={}, selector='{}', args=[{}]",
                    receiver,
                    selector,
                    args.join(", ")
                )?;
                // The variants that share the selector but didn't match — one per
                // line, below the message and above the stack trace.
                for candidate in candidates {
                    write!(f, "\n  {}", candidate)?;
                }
                Ok(())
            }
            QuoinError::AmbiguousMethod {
                msg, candidates, ..
            } => {
                write!(f, "{}", msg)?;
                for candidate in candidates {
                    write!(f, "\n  {}", candidate)?;
                }
                Ok(())
            }
            QuoinError::NotCallable(msg) => write!(f, "Not callable: {}", msg),
            QuoinError::StackUnderflow(msg) => write!(f, "Stack underflow: {}", msg),
            QuoinError::Other(msg) => write!(f, "{}", msg),
            QuoinError::Io { message, .. } => write!(f, "{}", message),
            QuoinError::IndexError { msg, .. } => write!(f, "{}", msg),
            QuoinError::Thrown => write!(f, "thrown exception"),
            QuoinError::NonLocalReturn => write!(f, "Non-local return"),
            QuoinError::Cancelled => write!(f, "task cancelled"),
            QuoinError::WithSourceInfo {
                error,
                source_info,
                trace,
                supports_color,
            } => {
                writeln!(f, "{}", error)?;

                let at_str = if *supports_color {
                    crate::ansi_colorizer::colorize("$#808080[at$]")
                } else {
                    "at".to_string()
                };

                let formatted_loc = if *supports_color {
                    format!(
                        "{}$#808080[:$]$#00bfff[{}$]$#808080[:$]$#00bfff[{}$]",
                        source_info.filename,
                        source_info.line,
                        source_info.column + 1
                    )
                } else {
                    format!(
                        "{}:{}:{}",
                        source_info.filename,
                        source_info.line,
                        source_info.column + 1
                    )
                };
                let formatted_loc_colorized = if *supports_color {
                    crate::ansi_colorizer::colorize(&formatted_loc)
                } else {
                    formatted_loc
                };

                write!(f, "  {} {}", at_str, formatted_loc_colorized)?;

                if let Some(source_text) = &source_info.source_text {
                    let pipe = if *supports_color {
                        crate::ansi_colorizer::colorize("$#808080[|$]")
                    } else {
                        "|".to_string()
                    };
                    writeln!(f)?;
                    writeln!(f, "  {}", pipe)?;

                    let highlighted_text = get_highlighted_range(
                        &source_info.filename,
                        source_info.start,
                        source_info.end,
                        source_text,
                        *supports_color,
                    );

                    for line in highlighted_text.lines() {
                        writeln!(f, "  {} {}", pipe, line)?;
                    }
                    write!(f, "  {}", pipe)?;
                }
                if !trace.is_empty() {
                    for frame_str in trace {
                        writeln!(f)?;
                        write!(f, "  {}", frame_str)?;
                    }
                }
                Ok(())
            }
        }
    }
}

impl Error for QuoinError {}

impl QuoinError {
    /// A structured I/O error of the given `kind` carrying a human-readable `message`.
    /// No `vm`/`mc` needed — the typed Quoin `IoError` object is built lazily at the
    /// `catch:` boundary (`quoinerror_to_value`), which is what lets I/O raise sites
    /// stay a plain `return Err(..)` with nothing borrowed across.
    pub fn io(kind: IoErrorKind, message: impl Into<String>) -> Self {
        QuoinError::Io {
            kind,
            message: message.into(),
        }
    }

    /// An I/O error for operating on a closed/consumed handle (`kind: #closed`).
    pub fn io_closed(message: impl Into<String>) -> Self {
        QuoinError::Io {
            kind: IoErrorKind::Closed,
            message: message.into(),
        }
    }

    /// Lift a backend [`IoError`] (OS error kind + message) into a structured I/O error.
    pub fn from_io_error(e: &IoError) -> Self {
        QuoinError::Io {
            kind: IoErrorKind::from(e.kind),
            message: e.message.clone(),
        }
    }
}

impl From<String> for QuoinError {
    fn from(s: String) -> Self {
        QuoinError::Other(s)
    }
}

impl From<&str> for QuoinError {
    fn from(s: &str) -> Self {
        QuoinError::Other(s.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mnu_renders_candidates_one_per_line() {
        let err = QuoinError::MessageNotUnderstood {
            receiver: "Foo".to_string(),
            selector: "bar:".to_string(),
            args: vec!["Boolean".to_string()],
            candidates: vec![
                "bar:Integer".to_string(),
                "bar:String {x.length > 3}".to_string(),
            ],
        };
        let out = format!("{}", err);
        let lines: Vec<&str> = out.lines().collect();
        assert!(lines[0].contains("selector='bar:'"));
        assert_eq!(lines[1], "  bar:Integer");
        assert_eq!(lines[2], "  bar:String {x.length > 3}");
    }

    #[test]
    fn ambiguous_renders_candidates_one_per_line() {
        let err = QuoinError::AmbiguousMethod {
            selector: "z:".to_string(),
            msg: "ambiguous dispatch for 'z:'".to_string(),
            candidates: vec!["z:QA".to_string(), "z:QB".to_string()],
        };
        let out = format!("{}", err);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines[0], "ambiguous dispatch for 'z:'");
        assert_eq!(lines[1], "  z:QA");
        assert_eq!(lines[2], "  z:QB");
    }

    #[test]
    fn mnu_without_candidates_is_single_line() {
        let err = QuoinError::MessageNotUnderstood {
            receiver: "Integer".to_string(),
            selector: "bogus".to_string(),
            args: vec![],
            candidates: vec![],
        };
        assert_eq!(format!("{}", err).lines().count(), 1);
    }
}
