use std::error::Error;
use std::fmt;

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
    /// Catch-all for generic error strings
    Other(String),
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
