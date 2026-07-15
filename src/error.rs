use crate::highlighter::{HighlightParser, HighlightSpan};
use crate::io_backend::IoError;
use crate::parser::try_parse_quoin_string_named;
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
    /// Synthetic (no OS errno): a bounded read exceeded its caller-imposed byte
    /// ceiling (e.g. `ByteStream.readUntil:limit:` with no delimiter in budget).
    LimitExceeded,
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
            IoErrorKind::LimitExceeded => "limitExceeded",
            IoErrorKind::Other => "other",
        }
    }
}

/// The cause of a [`QuoinError::PeerDied`], surfaced to Quoin code as
/// `PeerDiedError.reason` (a `#symbol`). Death is the peer *disappearing*
/// (`docs/internal/SUPERVISION.md` §2) — an error a live peer reports is
/// `ExtensionError` (or an ordinary raised error), never this. The set grows
/// with the supervision slices (`#spawnFailed`, `#gaveUp`, `#staleIncarnation`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerDeathReason {
    /// The peer's process exited, or its connection closed under a call.
    Exited,
    /// A thread-backed worker's body panicked (caught at the thread boundary).
    Panicked,
}

impl PeerDeathReason {
    /// The camelCase name used for the Quoin `#symbol`.
    pub fn symbol(self) -> &'static str {
        match self {
            PeerDeathReason::Exited => "exited",
            PeerDeathReason::Panicked => "panicked",
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
            // The backend reports an op interrupted by a handle close as NotConnected
            // ("stream/listener closed while ... in flight") — surface it as #closed
            // so accept/read loops can tell shutdown apart from transient failures.
            E::NotConnected => IoErrorKind::Closed,
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
    /// A deadline elapsed in `Async.timeout:ms do:{…}` (the bare form, no `onCancel:`).
    /// Mapped to a typed Quoin `TimeoutError` (exposing `ms`, the deadline) at the
    /// `catch:` boundary. Distinct from an OS-level I/O timeout, which is an
    /// `Io { kind: TimedOut }`. Plain data only — holds no `Gc`.
    Timeout { ms: i64 },
    /// A value of the right type but invalid content (e.g. a non-hex string to
    /// `Integer.fromHex:`, a byte outside `0..=255`, a malformed `host:port`). Mapped to a
    /// typed Quoin `ValueError` at the `catch:` boundary. Message-only.
    ValueError(String),
    /// A parse/decode failure of input data: invalid UTF-8 (`Bytes.asString`,
    /// `StringStream`), a malformed HTTP response head, or a compile error from `eval:`.
    /// Mapped to a typed Quoin `ParseError` at the `catch:` boundary. Message-only.
    ParseError(String),
    /// A class-structural violation reachable from guest code (extending a
    /// sealed class, instantiating an abstract one, …) — mapped to a typed
    /// Quoin `ClassError` so `catch:{|e:Error|}` catches it (BUGS.md
    /// Finding 12). Message-only.
    ClassError(String),
    /// A read of a name that is bound to nothing — `typo` where nothing named `typo`
    /// was ever declared. The read used to evaluate to `nil`, so a misspelling
    /// propagated silently while *assigning* to an undeclared local was rejected at
    /// compile time; the two halves of the strict `var`/`let` rule now agree.
    /// Ask whether a class exists with `Class.exists?:#Name`, not by reading it.
    NameError(String),
    /// A recoverable error an out-of-process extension raised from a call (e.g. a SQL error from
    /// the `adbc` driver) — the extension stays alive. Mapped to a catchable Quoin `Error` object
    /// at the `catch:` boundary. Distinct from the extension *crashing*, which is an `Io` of kind
    /// `#closed`. Message-only (a typed/extension-named error class is a later refinement).
    /// A recoverable error reported by an out-of-process extension. `remote_stack` is the
    /// OPAQUE cross-process stack blob (possibly multi-segment, unwind order; empty = none):
    /// displayed fenced by the printer and surfaced to Quoin as `ex.remoteStack` — never
    /// parsed (`quoin-ext-proto/PROTOCOL.md`).
    ExtensionError {
        message: String,
        remote_stack: String,
    },
    /// The isolate hosting the receiver DIED — a worker (thread or process) or an
    /// extension process. Distinct from everything a live peer can *report* (an
    /// `ExtensionError`, a hosted method raising): death is the peer disappearing
    /// (`docs/internal/SUPERVISION.md` §2). Mapped to the typed Quoin
    /// `PeerDiedError` — a root Error class deliberately distinct from `IoError`,
    /// which is too user-error-adjacent to share a catch clause with — exposing
    /// `reason` (a symbol) and `peer` (the peer's name). Plain data only — holds
    /// no `Gc`.
    PeerDied {
        /// The peer's name: the hosted class, the worker's label, or the
        /// extension's package/command.
        peer: String,
        reason: PeerDeathReason,
        message: String,
    },
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
    /// A recursion refused before it could overflow the machine stack: block re-entry with
    /// no headroom left (`execute_block`), or native → Quoin re-entry past
    /// `MAX_NATIVE_REENTRY`. Surfaces as the Quoin `StackError`, so `catch:{|e:StackError|}`
    /// and `catch:{|e:Error|}` both see it — the alternative is the process dying with an
    /// uncatchable SIGBUS.
    StackExhausted(String),
    /// `Runtime.exit:` — the guest requested process exit with this status. Unwinds
    /// like `Cancelled` (`finally` blocks run, `catch:` cannot swallow it); the
    /// driver surfaces it to the runner, which exits after normal teardown so
    /// `Drop`s (extension children, sockets) still run.
    ExitRequested(i32),
    /// Wrapper containing source location for execution errors
    WithSourceInfo {
        error: Box<QuoinError>,
        source_info: SourceInfo,
        trace: Vec<String>,
        supports_color: bool,
    },
}

impl QuoinError {
    /// Peel `WithSourceInfo` wrappers to the underlying error (identity when unwrapped).
    pub fn innermost(&self) -> &QuoinError {
        match self {
            QuoinError::WithSourceInfo { error, .. } => error.innermost(),
            other => other,
        }
    }
}

/// Strip control characters (except newline/tab) from an untrusted cross-process stack
/// blob before terminal display — foreign text must not smuggle ANSI/cursor sequences.
fn sanitize_blob(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect()
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
            // Fallible parse: this runs inside error DISPLAY — a file that no
            // longer parses (edited since, or not Quoin at all) must degrade
            // to plain text, never panic the report.
            let program = try_parse_quoin_string_named(&content, filename).ok()?;
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
        // `fallback_text` is an arbitrary EXPRESSION FRAGMENT sliced from a
        // frame's span (e.g. `^(.new:{ var message = msg }).throw` from
        // qnlib's `Error.throw:`) — it very often does not parse as a
        // standalone program. The panicking parser here crashed the whole
        // interactive REPL on ANY uncaught error with a qnlib frame once the
        // embedded stdlib made those filenames unreadable from disk (the
        // read_to_string path above used to mask this). Degrade to plain
        // text instead.
        let program = try_parse_quoin_string_named(fallback_text, filename).ok()?;
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
            QuoinError::Timeout { ms } => write!(f, "operation timed out after {}ms", ms),
            QuoinError::ValueError(msg) => write!(f, "{}", msg),
            QuoinError::ParseError(msg) => write!(f, "{}", msg),
            QuoinError::ClassError(msg) => write!(f, "{}", msg),
            QuoinError::NameError(msg) => write!(f, "{}", msg),
            QuoinError::ExtensionError { message, .. } => write!(f, "{}", message),
            QuoinError::PeerDied { message, .. } => write!(f, "{}", message),
            QuoinError::Thrown => write!(f, "thrown exception"),
            QuoinError::NonLocalReturn => write!(f, "Non-local return"),
            QuoinError::Cancelled => write!(f, "task cancelled"),
            QuoinError::StackExhausted(msg) => write!(f, "{}", msg),
            QuoinError::ExitRequested(code) => write!(f, "exit requested (status {})", code),
            QuoinError::WithSourceInfo {
                error,
                source_info,
                trace,
                supports_color,
            } => {
                writeln!(f, "{}", error)?;

                let at_str = if *supports_color {
                    crate::ansi_colorizer::colorize("[#808080]at[/]")
                } else {
                    "at".to_string()
                };

                let formatted_loc = if *supports_color {
                    format!(
                        "{}[#808080]:[/][#00bfff]{}[/][#808080]:[/][#00bfff]{}[/]",
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
                        crate::ansi_colorizer::colorize("[#808080]|[/]")
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
                // A cross-process blob sits between the failing line and the Quoin trace:
                // everything REMOTE is deeper than the raise point, so it prints first,
                // fenced and sanitized (untrusted foreign text must not inject control
                // sequences into the terminal).
                if let QuoinError::ExtensionError { remote_stack, .. } = error.innermost()
                    && !remote_stack.is_empty()
                {
                    writeln!(f)?;
                    writeln!(f, "  --- in extension ---")?;
                    for line in sanitize_blob(remote_stack).lines() {
                        writeln!(f, "  {}", line)?;
                    }
                    write!(f, "  ---")?;
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

    /// A peer-death error: the isolate named `peer` is gone
    /// (`docs/internal/SUPERVISION.md` §2). Built lazily into the typed Quoin
    /// `PeerDiedError` at the `catch:` boundary, like [`QuoinError::io`].
    pub fn peer_died(
        peer: impl Into<String>,
        reason: PeerDeathReason,
        message: impl Into<String>,
    ) -> Self {
        QuoinError::PeerDied {
            peer: peer.into(),
            reason,
            message: message.into(),
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

    #[test]
    fn io_error_kind_symbol_names() {
        use IoErrorKind::*;
        assert_eq!(Closed.symbol(), "closed");
        assert_eq!(NotFound.symbol(), "notFound");
        assert_eq!(PermissionDenied.symbol(), "permissionDenied");
        assert_eq!(ConnectionRefused.symbol(), "connectionRefused");
        assert_eq!(ConnectionReset.symbol(), "connectionReset");
        assert_eq!(ConnectionAborted.symbol(), "connectionAborted");
        assert_eq!(BrokenPipe.symbol(), "brokenPipe");
        assert_eq!(AddrInUse.symbol(), "addrInUse");
        assert_eq!(AddrNotAvailable.symbol(), "addrNotAvailable");
        assert_eq!(TimedOut.symbol(), "timedOut");
        assert_eq!(UnexpectedEof.symbol(), "unexpectedEof");
        assert_eq!(InvalidInput.symbol(), "invalidInput");
        assert_eq!(InvalidData.symbol(), "invalidData");
        assert_eq!(Other.symbol(), "other");
    }

    #[test]
    fn io_error_kind_from_std_error_kind() {
        use std::io::ErrorKind as E;
        assert_eq!(IoErrorKind::from(E::NotFound), IoErrorKind::NotFound);
        assert_eq!(
            IoErrorKind::from(E::PermissionDenied),
            IoErrorKind::PermissionDenied
        );
        assert_eq!(
            IoErrorKind::from(E::ConnectionRefused),
            IoErrorKind::ConnectionRefused
        );
        assert_eq!(
            IoErrorKind::from(E::ConnectionReset),
            IoErrorKind::ConnectionReset
        );
        assert_eq!(
            IoErrorKind::from(E::ConnectionAborted),
            IoErrorKind::ConnectionAborted
        );
        assert_eq!(IoErrorKind::from(E::BrokenPipe), IoErrorKind::BrokenPipe);
        assert_eq!(IoErrorKind::from(E::AddrInUse), IoErrorKind::AddrInUse);
        assert_eq!(
            IoErrorKind::from(E::AddrNotAvailable),
            IoErrorKind::AddrNotAvailable
        );
        assert_eq!(IoErrorKind::from(E::TimedOut), IoErrorKind::TimedOut);
        assert_eq!(
            IoErrorKind::from(E::UnexpectedEof),
            IoErrorKind::UnexpectedEof
        );
        assert_eq!(
            IoErrorKind::from(E::InvalidInput),
            IoErrorKind::InvalidInput
        );
        assert_eq!(IoErrorKind::from(E::InvalidData), IoErrorKind::InvalidData);
        // Kinds we don't name fold into Other.
        assert_eq!(IoErrorKind::from(E::WouldBlock), IoErrorKind::Other);
    }

    #[test]
    fn display_simple_variants() {
        let cases: Vec<(QuoinError, &str)> = vec![
            (
                QuoinError::ArgumentCountMismatch {
                    expected: 1,
                    got: 2,
                    msg: "too many".to_string(),
                },
                "Argument count mismatch: too many",
            ),
            (
                QuoinError::TypeError {
                    expected: "Integer".to_string(),
                    got: "String".to_string(),
                    msg: "nope".to_string(),
                },
                "Type error: nope",
            ),
            (
                QuoinError::ArithmeticError("div by zero".to_string()),
                "Arithmetic error: div by zero",
            ),
            (
                QuoinError::NotCallable("nope".to_string()),
                "Not callable: nope",
            ),
            (
                QuoinError::StackUnderflow("empty".to_string()),
                "Stack underflow: empty",
            ),
            (QuoinError::Other("plain".to_string()), "plain"),
            (
                QuoinError::Io {
                    kind: IoErrorKind::NotFound,
                    message: "missing".to_string(),
                },
                "missing",
            ),
            (
                QuoinError::IndexError {
                    index: 5,
                    len: 3,
                    msg: "out of range".to_string(),
                },
                "out of range",
            ),
            (
                QuoinError::Timeout { ms: 250 },
                "operation timed out after 250ms",
            ),
            (QuoinError::ValueError("bad".to_string()), "bad"),
            (QuoinError::ParseError("malformed".to_string()), "malformed"),
            (QuoinError::Thrown, "thrown exception"),
            (QuoinError::NonLocalReturn, "Non-local return"),
            (QuoinError::Cancelled, "task cancelled"),
            (QuoinError::ExitRequested(3), "exit requested (status 3)"),
        ];
        for (err, expected) in cases {
            assert_eq!(format!("{}", err), expected, "variant {:?}", err);
        }
    }

    #[test]
    fn from_string_and_str_make_other() {
        let a: QuoinError = "boom".into();
        assert!(matches!(a, QuoinError::Other(ref s) if s == "boom"));
        let b: QuoinError = String::from("bang").into();
        assert!(matches!(b, QuoinError::Other(ref s) if s == "bang"));
    }

    fn src_info(filename: &str, source_text: Option<&str>, end: usize) -> SourceInfo {
        SourceInfo {
            filename: filename.to_string(),
            line: 3,
            column: 4,
            start: 0,
            end,
            source_text: source_text.map(|s| s.to_string()),
        }
    }

    #[test]
    fn display_with_source_info_plain() {
        let err = QuoinError::WithSourceInfo {
            error: Box::new(QuoinError::TypeError {
                expected: "Integer".to_string(),
                got: "String".to_string(),
                msg: "bad arg".to_string(),
            }),
            source_info: src_info("fixture.qn", None, 0),
            trace: vec![],
            supports_color: false,
        };
        let out = format!("{}", err);
        assert!(out.contains("Type error: bad arg"), "got: {out:?}");
        // column is rendered 1-based (column + 1).
        assert!(out.contains("at fixture.qn:3:5"), "got: {out:?}");
    }

    #[test]
    fn display_with_source_info_shows_source_block() {
        let err = QuoinError::WithSourceInfo {
            error: Box::new(QuoinError::Other("boom".to_string())),
            source_info: src_info("fixture.qn", Some("nil.bogusMethod"), 5),
            trace: vec![],
            supports_color: false,
        };
        let out = format!("{}", err);
        assert!(out.contains("boom"), "got: {out:?}");
        assert!(out.contains('|'), "expected the source pipe block: {out:?}");
        assert!(out.contains("nil.bogusMethod"), "got: {out:?}");
    }

    #[test]
    fn display_with_source_info_shows_trace() {
        let err = QuoinError::WithSourceInfo {
            error: Box::new(QuoinError::Other("boom".to_string())),
            source_info: src_info("f.qn", None, 0),
            trace: vec!["at frame one".to_string(), "at frame two".to_string()],
            supports_color: false,
        };
        let out = format!("{}", err);
        assert!(out.contains("at frame one"), "got: {out:?}");
        assert!(out.contains("at frame two"), "got: {out:?}");
    }

    #[test]
    fn display_with_source_info_highlights_from_file() {
        use std::io::Write as _;

        // The colorized trace highlights the failing range read back from the source
        // file, so this path needs supports_color = true and a real readable file.
        let path = std::env::temp_dir().join(format!("quoin_err_hl_{}.qn", std::process::id()));
        let src = "x = 42\n";
        std::fs::File::create(&path)
            .unwrap()
            .write_all(src.as_bytes())
            .unwrap();

        let err = QuoinError::WithSourceInfo {
            error: Box::new(QuoinError::Other("boom".to_string())),
            source_info: SourceInfo {
                filename: path.to_string_lossy().into_owned(),
                line: 1,
                column: 0,
                start: 0,
                end: 6, // "x = 42"
                source_text: Some(src[..6].to_string()),
            },
            trace: vec![],
            supports_color: true,
        };
        let out = format!("{}", err);
        std::fs::remove_file(&path).ok();
        // Color-on output carries ANSI escapes (location + highlighted snippet).
        assert!(
            out.contains('\u{1b}'),
            "expected ANSI escapes in colorized output: {out:?}"
        );
    }

    #[test]
    fn display_degrades_when_the_snippet_is_an_unparseable_fragment() {
        // A frame's `source_text` is an arbitrary expression FRAGMENT sliced
        // from its span — qnlib's `Error.throw:` yields
        // fragments like `var message = msg }).throw` that do not parse as
        // standalone programs. With the frame's filename unreadable (the
        // embedded stdlib's `std:…` names, `<repl>`, `<string>`), the
        // colorized Display used to re-parse that fragment with the PANICKING
        // parser: every uncaught error with a qnlib frame crashed the whole
        // interactive REPL. It must degrade to plain text instead.
        let frag = "var message = msg }).throw";
        let err = QuoinError::WithSourceInfo {
            error: Box::new(QuoinError::Other("boom".to_string())),
            source_info: SourceInfo {
                filename: "std:core/00-bootstrap.qn".to_string(),
                line: 1,
                column: 0,
                start: 0,
                end: frag.len(),
                source_text: Some(frag.to_string()),
            },
            trace: vec![],
            supports_color: true,
        };
        let out = format!("{}", err); // the whole point: this must not panic
        assert!(out.contains(frag), "the raw snippet still renders: {out:?}");
    }
}
