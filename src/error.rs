use crate::highlighter::{HighlightParser, HighlightSpan};
use crate::parser::parse_building_blocks_string;
use crate::value::SourceInfo;
use std::error::Error;
use std::{cmp, fmt, fs};

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
            let program = parse_building_blocks_string(&content);
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
        let program = parse_building_blocks_string(fallback_text);
        let mut parser = HighlightParser::new(fallback_text);
        let spans = parser.highlight_program(&program);
        Some(crate::highlighter::format_ansi(fallback_text, spans))
    };
    parse_and_highlight_fallback().unwrap_or_else(|| fallback_text.to_string())
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
