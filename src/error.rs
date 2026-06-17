use std::error::Error;
use std::fmt;

use crate::value::SourceInfo;

#[derive(Debug, Clone)]
pub enum BBError {
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
    /// Raised when method lookup fails
    MessageNotUnderstood {
        receiver: String,
        selector: String,
        args: Vec<String>,
    },
    /// Raised when trying to execute a value that does not implement call/send dispatch
    NotCallable(String),
    /// Raised when attempting to pop or peek from an empty VM stack
    StackUnderflow(String),
    /// Generic other error
    Other(String),
    /// Raised to propagate non-local returns out of native call stacks
    NonLocalReturn,
    /// Wrapper containing source location for execution errors
    WithSourceInfo {
        error: Box<BBError>,
        source_info: SourceInfo,
        trace: Vec<String>,
    },
}

impl fmt::Display for BBError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BBError::ArgumentCountMismatch { msg, .. } => {
                write!(f, "Argument count mismatch: {}", msg)
            }
            BBError::TypeError { msg, .. } => write!(f, "Type error: {}", msg),
            BBError::ArithmeticError(msg) => write!(f, "Arithmetic error: {}", msg),
            BBError::MessageNotUnderstood {
                receiver,
                selector,
                args,
            } => {
                write!(
                    f,
                    "Message not understood: receiver={}, selector='{}', args=[{}]",
                    receiver,
                    selector,
                    args.join(", ")
                )
            }
            BBError::NotCallable(msg) => write!(f, "Not callable: {}", msg),
            BBError::StackUnderflow(msg) => write!(f, "Stack underflow: {}", msg),
            BBError::Other(msg) => write!(f, "{}", msg),
            BBError::NonLocalReturn => write!(f, "Non-local return"),
            BBError::WithSourceInfo {
                error,
                source_info,
                trace,
            } => {
                writeln!(f, "{}", error)?;
                write!(
                    f,
                    "  at {}:{}:{}",
                    source_info.filename,
                    source_info.line,
                    source_info.column + 1
                )?;
                if let Some(source_text) = &source_info.source_text {
                    writeln!(f)?;
                    writeln!(f, "  |")?;
                    for line in source_text.lines() {
                        writeln!(f, "  | {}", line)?;
                    }
                    write!(f, "  |")?;
                }
                if !trace.is_empty() {
                    writeln!(f)?;
                    writeln!(f, "VM Stack Trace:")?;
                    for (i, frame_str) in trace.iter().enumerate() {
                        if i > 0 {
                            writeln!(f)?;
                        }
                        write!(f, "    {}", frame_str)?;
                    }
                }
                Ok(())
            }
        }
    }
}

impl Error for BBError {}

impl From<String> for BBError {
    fn from(s: String) -> Self {
        BBError::Other(s)
    }
}

impl From<&str> for BBError {
    fn from(s: &str) -> Self {
        BBError::Other(s.to_string())
    }
}
