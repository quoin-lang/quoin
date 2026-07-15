//! Error construction and reporting: `annotate_error`, error-object building,
//! snippet highlighting, and the compile/type diagnostic renderers. Extends
//! `VmState`.

use super::*;

impl<'gc> VmState<'gc> {
    /// `file:line:col: <level>: <message>` header, the level keyword colored (yellow warning,
    /// red error, gray note) like uncaught errors. `indent` shifts a provenance note under its
    /// parent. Shared by `report_type_warnings` and `report_compile_error`.
    pub(crate) fn diag_header(
        level: &str,
        color: &str,
        message: &str,
        span: Option<&crate::value::SourceInfo>,
        colorize: bool,
        indent: bool,
    ) -> String {
        let pad = if indent { "  " } else { "" };
        let label = if colorize {
            ansi_colorizer::colorize(&format!("[{color}]{level}[/]"))
        } else {
            level.to_string()
        };
        match span {
            Some(s) => {
                // The `file:line:col` form colored exactly like a stack
                // trace's location (gray separators, cyan numbers).
                let loc = if colorize {
                    ansi_colorizer::colorize(&format!(
                        "{}[#808080]:[/][#00bfff]{}[/][#808080]:[/][#00bfff]{}[/]",
                        s.filename,
                        s.line,
                        s.column + 1
                    ))
                } else {
                    format!("{}:{}:{}", s.filename, s.line, s.column + 1)
                };
                format!("{pad}{loc}: {label}: {message}\n")
            }
            None => format!("{pad}{label}: {message}\n"),
        }
    }

    /// The offending line under a gray `|` gutter with a caret beneath the span — the same
    /// visual language as an uncaught error's source block. `None` if the file can't be read.
    fn diag_source_block(span: &crate::value::SourceInfo, colorize: bool) -> Option<String> {
        let content = fs::read_to_string(&span.filename).ok()?;
        let line_text = content.lines().nth(span.line.saturating_sub(1))?;
        let width = content
            .get(span.start..span.end)
            .map(|s| s.chars().count())
            .unwrap_or(1)
            .max(1);
        let gutter = span.line.to_string();
        let pad = " ".repeat(gutter.len());
        let pipe = if colorize {
            ansi_colorizer::colorize("[#808080]|[/]")
        } else {
            "|".to_string()
        };
        let line_hl = if colorize {
            highlight_to_ansi(line_text)
        } else {
            line_text.to_string()
        };
        let carets = format!("{}{}", " ".repeat(span.column), "^".repeat(width));
        let carets = if colorize {
            ansi_colorizer::colorize(&format!("[#ffcc00]{carets}[/]"))
        } else {
            carets
        };
        Some(format!(
            "  {pad} {pipe}\n  {gutter} {pipe} {line_hl}\n  {pad} {pipe} {carets}\n"
        ))
    }

    /// Emit collected compile-time type diagnostics through the stderr sink (so under the DAP
    /// adapter, with `capture` on, they become `output` events rather than leaking to raw stderr).
    /// Each is rendered `file:line:col: warning: message` (the standard, editor-jumpable form) when
    /// a span is known, else bare `warning: message`. Best-effort; never fatal. (Phase 4.)
    pub fn report_type_warnings(&mut self, diagnostics: &[crate::compiler::Diagnostic]) {
        let colorize = self.options.supports_color;
        let mut out = String::new();
        for d in diagnostics {
            out.push_str(&Self::diag_header(
                "warning",
                "#ffcc00",
                &d.message,
                d.span.as_ref(),
                colorize,
                false,
            ));
            if let Some(s) = &d.span
                && let Some(block) = Self::diag_source_block(s, colorize)
            {
                out.push_str(&block);
            }
            // Why-chain notes (Phase 4 provenance): each under its own span, indented.
            for note in &d.notes {
                out.push_str(&Self::diag_header(
                    "note",
                    "#808080",
                    &note.message,
                    note.span.as_ref(),
                    colorize,
                    true,
                ));
                if let Some(s) = &note.span
                    && let Some(block) = Self::diag_source_block(s, colorize)
                {
                    out.push_str(&block);
                }
            }
        }
        let _ = self.write_std(StdStream::Err, out.as_bytes());
    }

    /// Report a fatal compile error through the stderr sink, `file:line:col: error: message`
    /// with the offending line and caret — the same visual language as `report_type_warnings`,
    /// at `error` level. Used by the file-based entry points (run, check, debug, benchmark);
    /// string-based modes (`-e`, REPL, `Runtime.eval:`, workers) use the error's `Display`,
    /// which embeds `(line …, column …)` the way those modes render parse errors.
    pub fn report_compile_error(&mut self, err: &crate::compiler::CompileError) {
        let colorize = self.options.supports_color;
        let mut out = Self::diag_header(
            "error",
            "#ff6961",
            &err.message,
            err.span.as_ref(),
            colorize,
            false,
        );
        if let Some(s) = &err.span
            && let Some(block) = Self::diag_source_block(s, colorize)
        {
            out.push_str(&block);
        }
        let _ = self.write_std(StdStream::Err, out.as_bytes());
    }

    pub fn annotate_error(&self, error: QuoinError) -> QuoinError {
        // An uncaught Quoin throw reaches here as `Thrown`; surface the actual
        // thrown value (which lives in `active_exception`) for display.
        let error = if matches!(error, QuoinError::Thrown) {
            let msg = match self.exceptions.active {
                Some(v) => format!("{}", v),
                None => "uncaught exception".to_string(),
            };
            QuoinError::Other(msg)
        } else {
            error
        };
        if matches!(error, QuoinError::WithSourceInfo { .. }) {
            return error;
        }
        if let Some(frame) = self.frames.last() {
            let active_ip = if frame.ip > 0 { frame.ip - 1 } else { 0 };
            let active_source_info = frame
                .block
                .template
                .source_map
                .get(active_ip)
                .and_then(|opt| opt.as_ref())
                .or(frame.block.template.source_info.as_ref())
                .cloned();
            if let Some(source_info) = active_source_info {
                let supports_color = self.options.supports_color;

                let colorize_selector = |sel: &str, cls: &str| -> String {
                    if supports_color {
                        format!("[#ab82ff]{}[/][#808080]:[/][#5fd7af]{}[/]", sel, cls)
                    } else {
                        format!("{}:{}", sel, cls)
                    }
                };
                let colorize_simple = |sel: &str| -> String {
                    if supports_color {
                        format!("[#ab82ff]{}[/]", sel)
                    } else {
                        sel.to_string()
                    }
                };

                let mut frames_info = Vec::new();
                let n = self.frames.len();
                for (i, f) in self.frames.iter().enumerate().rev() {
                    if i == n - 1 {
                        continue;
                    }
                    let frame_ip = if f.ip > 0 { f.ip - 1 } else { 0 };

                    let si_opt = f
                        .block
                        .template
                        .source_map
                        .get(frame_ip)
                        .and_then(|opt| opt.as_ref())
                        .or(f.block.template.source_info.as_ref())
                        .cloned();

                    // The failing instruction is a send — plain or a fused superinstruction;
                    // pull `(selector, num_args)` from whichever form it is.
                    let send_at_ip = match f.block.template.bytecode.get(frame_ip) {
                        Some(Instruction::Send(s, n))
                        | Some(Instruction::SendLocal(_, s, n))
                        | Some(Instruction::SendConst(_, s, n))
                        | Some(Instruction::SendField(_, s, n))
                        | Some(Instruction::SendLocalLocal(_, _, s, n))
                        | Some(Instruction::SendLocalConst(_, _, s, n)) => Some((*s, *n)),
                        _ => None,
                    };
                    let formatted_selector = if let Some((selector, num_args)) = send_at_ip {
                        let selector = selector.as_str();
                        let args_vec = if num_args > 0 {
                            if i == n - 1 {
                                self.exceptions.last_send_args.clone()
                            } else {
                                self.frames[i + 1].args.clone()
                            }
                        } else {
                            Vec::new()
                        };

                        if !args_vec.is_empty() {
                            let mut parts = Vec::new();
                            let mut current = String::new();
                            for c in selector.chars() {
                                current.push(c);
                                if c == ':' {
                                    parts.push(current);
                                    current = String::new();
                                }
                            }
                            if !current.is_empty() {
                                parts.push(current);
                            }

                            let mut formatted_parts = Vec::new();
                            for (idx, part) in parts.iter().enumerate() {
                                if let Some(arg) = args_vec.get(idx) {
                                    let mut p = part.clone();
                                    if p.ends_with(':') {
                                        p.pop();
                                    }
                                    formatted_parts.push(colorize_selector(&p, &arg.class_name()));
                                } else {
                                    formatted_parts.push(colorize_simple(part));
                                }
                            }
                            formatted_parts.join(" ")
                        } else {
                            colorize_simple(selector)
                        }
                    } else if i == 0 {
                        colorize_simple("(top)")
                    } else {
                        let sel_str = f
                            .selector
                            .map(|s| s.as_str().to_string())
                            .unwrap_or_else(|| "value".to_string());
                        colorize_simple(&sel_str)
                    };

                    let formatted_loc = if let Some(si) = &si_opt {
                        let display_filename = Path::new(&si.filename)
                            .file_name()
                            .and_then(|s| s.to_str())
                            .unwrap_or(&si.filename)
                            .to_string();
                        if supports_color {
                            format!(
                                " [#808080]in[/] {}[#808080]:[/][#00bfff]{}[/][#808080]:[/][#00bfff]{}[/]",
                                display_filename, si.line, si.column
                            )
                        } else {
                            format!(" in {}:{}:{}", display_filename, si.line, si.column)
                        }
                    } else {
                        "".to_string()
                    };

                    let at_str = if supports_color {
                        "[#808080]at[/]"
                    } else {
                        "at"
                    };
                    let prefix_colored =
                        format!("{} {}{}", at_str, formatted_selector, formatted_loc);
                    let prefix_plain = if supports_color {
                        ansi_colorizer::decolorize(&ansi_colorizer::colorize(&prefix_colored))
                    } else {
                        prefix_colored.clone()
                    };
                    let plain_len = prefix_plain.chars().count();

                    frames_info.push((prefix_colored, plain_len, si_opt));
                }

                // Always append the (top) frame at the bottom if it was not already the only frame formatted as (top)
                if n > 0 {
                    let first_frame = &self.frames[0];
                    let first_ip = if first_frame.ip > 0 {
                        first_frame.ip - 1
                    } else {
                        0
                    };
                    let si_opt = first_frame
                        .block
                        .template
                        .source_map
                        .get(first_ip)
                        .and_then(|opt| opt.as_ref())
                        .or(first_frame.block.template.source_info.as_ref())
                        .cloned();

                    let formatted_selector = colorize_simple("(top)");

                    let formatted_loc = if let Some(si) = &si_opt {
                        let display_filename = Path::new(&si.filename)
                            .file_name()
                            .and_then(|s| s.to_str())
                            .unwrap_or(&si.filename)
                            .to_string();
                        if supports_color {
                            format!(
                                " [#808080]in[/] {}[#808080]:[/][#00bfff]{}[/][#808080]:[/][#00bfff]{}[/]",
                                display_filename, si.line, si.column
                            )
                        } else {
                            format!(" in {}:{}:{}", display_filename, si.line, si.column)
                        }
                    } else {
                        "".to_string()
                    };

                    let at_str = if supports_color {
                        "[#808080]at[/]"
                    } else {
                        "at"
                    };
                    let prefix_colored =
                        format!("{} {}{}", at_str, formatted_selector, formatted_loc);
                    let prefix_plain = if supports_color {
                        ansi_colorizer::decolorize(&ansi_colorizer::colorize(&prefix_colored))
                    } else {
                        prefix_colored.clone()
                    };
                    let plain_len = prefix_plain.chars().count();

                    // Only push if the last trace element is not already representing (top) at the same location
                    let is_dup = if let Some(last_info) = frames_info.last() {
                        last_info.0 == prefix_colored
                    } else {
                        false
                    };

                    if !is_dup {
                        frames_info.push((prefix_colored, plain_len, si_opt));
                    }
                }

                let max_l = frames_info.iter().map(|info| info.1).max().unwrap_or(0);
                let target_alignment = cmp::max(54, max_l + 2);

                let console_width = self.options.console_width.unwrap_or(80) as usize;
                let available_width = console_width.saturating_sub(target_alignment + 4);
                let show_snippet = available_width >= 15;
                let w = available_width;

                let mut trace = Vec::new();
                for (prefix_colored, plain_len, si_opt) in frames_info {
                    let mut line = if supports_color {
                        ansi_colorizer::colorize(&prefix_colored)
                    } else {
                        prefix_colored
                    };

                    if let Some(si) = si_opt
                        && show_snippet
                        && let Some(snippet) = self.get_highlighted_snippet(
                            &si.filename,
                            si.line.saturating_sub(1),
                            si.column,
                            si.start,
                            si.end,
                            si.source_text.as_ref(),
                            w,
                        )
                    {
                        let padding_len = target_alignment.saturating_sub(plain_len);
                        let padding: String = " ".repeat(padding_len);
                        let separator = if supports_color {
                            ansi_colorizer::colorize("[#808080]<[/]")
                        } else {
                            "<".to_string()
                        };
                        line = format!("{}{}{} {}", line, padding, separator, snippet);
                    }
                    trace.push(line);
                }

                return QuoinError::WithSourceInfo {
                    error: Box::new(error),
                    source_info: source_info.clone(),
                    trace,
                    supports_color,
                };
            }
        }
        error
    }

    /// Build a typed Quoin error object: an instance of `class_name` with its `message`
    /// field set, plus any `extra` (name, value) fields. Falls back to a plain string if
    /// the class isn't registered yet (e.g. an error fired during bootstrap before the
    /// Error hierarchy is defined). The typed `make_*` helpers below are thin wrappers —
    /// each domain error sets `message` plus its own structured fields.
    fn build_error_object(
        &self,
        mc: &Mutation<'gc>,
        class_name: &str,
        message: &str,
        extra: &[(&str, Value<'gc>)],
    ) -> Value<'gc> {
        let key = NamespacedName::new(Vec::new(), class_name.to_string());
        let class_opt = self.globals.borrow().get(&key).copied();
        if let Some(Value::Class(cls)) = class_opt {
            let obj = self.new_object(mc, cls);
            let msg_val = self.new_string(mc, message.to_string());
            if let Some(slot) = self.field_slot(cls, "message") {
                obj.borrow_mut(mc).fields[slot] = msg_val;
            }
            for (name, val) in extra {
                if let Some(slot) = self.field_slot(cls, name) {
                    obj.borrow_mut(mc).fields[slot] = *val;
                }
            }
            Value::Object(obj)
        } else {
            self.new_string(mc, message.to_string())
        }
    }

    /// Build a Quoin `Error` instance of the named class with `message`/`payload`.
    pub fn make_error(
        &self,
        mc: &Mutation<'gc>,
        class_name: &str,
        message: &str,
        payload: Option<Value<'gc>>,
    ) -> Value<'gc> {
        match payload {
            Some(p) => self.build_error_object(mc, class_name, message, &[("payload", p)]),
            None => self.build_error_object(mc, class_name, message, &[]),
        }
    }

    /// Build a Quoin `IoError` carrying `message` and a `kind` symbol (e.g.
    /// `#connectionRefused`).
    pub fn make_io_error(&self, mc: &Mutation<'gc>, kind: &str, message: &str) -> Value<'gc> {
        let kind_val = self.new_symbol(mc, kind.to_string());
        self.build_error_object(mc, "IoError", message, &[("kind", kind_val)])
    }

    /// Build a Quoin `IndexError` carrying `message` and the offending `index`/`length`.
    pub fn make_index_error(
        &self,
        mc: &Mutation<'gc>,
        index: i64,
        len: i64,
        message: &str,
    ) -> Value<'gc> {
        let index_val = self.new_int(mc, index);
        let length_val = self.new_int(mc, len);
        self.build_error_object(
            mc,
            "IndexError",
            message,
            &[("index", index_val), ("length", length_val)],
        )
    }

    /// Convert an internal `QuoinError` into the Quoin value a `catch:` handler should
    /// receive. Domain variants become typed `Error` objects so guest code can dispatch
    /// on them; control-flow signals and internal errors stay a descriptive string. The
    /// match is exhaustive over domain variants on purpose — a new typed error that
    /// forgets its arm here is then a compile error, not a silent fall-through to string.
    pub fn quoinerror_to_value(&self, mc: &Mutation<'gc>, error: &QuoinError) -> Value<'gc> {
        match error {
            QuoinError::TypeError { msg, .. } => self.make_error(mc, "TypeError", msg, None),
            QuoinError::ArgumentCountMismatch { msg, .. } => {
                self.make_error(mc, "ArgumentError", msg, None)
            }
            QuoinError::ArithmeticError(msg) => self.make_error(mc, "ArithmeticError", msg, None),
            QuoinError::MessageNotUnderstood {
                receiver, selector, ..
            } => {
                let msg = format!("no method '{}' for {}", selector, receiver);
                self.make_error(mc, "MessageNotUnderstood", &msg, None)
            }
            QuoinError::AmbiguousMethod { msg, .. } => {
                self.make_error(mc, "AmbiguousMethodError", msg, None)
            }
            QuoinError::Io { kind, message } => self.make_io_error(mc, kind.symbol(), message),
            QuoinError::IndexError { index, len, msg } => {
                self.make_index_error(mc, *index, *len, msg)
            }
            QuoinError::Timeout { ms } => {
                let ms_val = self.new_int(mc, *ms);
                self.build_error_object(mc, "TimeoutError", &error.to_string(), &[("ms", ms_val)])
            }
            QuoinError::ValueError(msg) => self.make_error(mc, "ValueError", msg, None),
            QuoinError::ParseError(msg) => self.make_error(mc, "ParseError", msg, None),
            QuoinError::ClassError(msg) => self.make_error(mc, "ClassError", msg, None),
            QuoinError::NameError(msg) => self.make_error(mc, "NameError", msg, None),
            QuoinError::StackExhausted(msg) => self.make_error(mc, "StackError", msg, None),
            QuoinError::ExtensionError {
                message,
                remote_stack,
            } => {
                if remote_stack.is_empty() {
                    self.make_error(mc, "Error", message, None)
                } else {
                    // The opaque cross-process blob rides on the error object for
                    // programmatic access (`ex.remoteStack`); the printer shows it fenced.
                    let blob = self.new_string(mc, remote_stack.clone());
                    self.build_error_object(mc, "Error", message, &[("remoteStack", blob)])
                }
            }
            QuoinError::WithSourceInfo { error, .. } => self.quoinerror_to_value(mc, error),
            QuoinError::NotCallable(_)
            | QuoinError::StackUnderflow(_)
            | QuoinError::Other(_)
            | QuoinError::Thrown
            | QuoinError::NonLocalReturn
            | QuoinError::Cancelled
            | QuoinError::ExitRequested(_) => {
                let s = format!("{}", error);
                self.new_string(mc, s)
            }
        }
    }

    #[allow(clippy::too_many_arguments)] // snippet renderer takes the full source-location context
    pub(super) fn get_highlighted_snippet(
        &self,
        filename: &str,
        line_idx: usize,
        column: usize,
        node_start_offset: usize,
        node_end_offset: usize,
        source_text: Option<&String>,
        w: usize,
    ) -> Option<String> {
        let supports_color = self.options.supports_color;
        let content = match fs::read_to_string(filename) {
            Ok(s) => s,
            Err(_) => {
                if let Some(text) = source_text {
                    let snippet_text = if text.chars().count() > w {
                        let sliced: String = text.chars().take(w).collect();
                        sliced
                    } else {
                        text.clone()
                    };
                    if supports_color {
                        // Resilient: `snippet_text` is `source_text` truncated to `w`, so it can
                        // end mid-expression — `highlight_to_ansi` predictively completes it and
                        // never panics (and returns the text verbatim when it can't parse).
                        return Some(highlight_to_ansi(&snippet_text));
                    }
                    return Some(snippet_text);
                }
                return None;
            }
        };

        let mut current_line = 0;
        let mut line_start_byte = 0;
        let mut line_end_byte = content.len();
        for (i, c) in content.char_indices() {
            if c == '\n' {
                if current_line == line_idx {
                    line_end_byte = i;
                    break;
                }
                current_line += 1;
                line_start_byte = i + 1;
            }
        }
        if current_line != line_idx {
            if current_line == line_idx && line_start_byte <= content.len() {
                line_end_byte = content.len();
            } else {
                return None;
            }
        }

        if line_end_byte > line_start_byte && content.as_bytes()[line_end_byte - 1] == b'\r' {
            line_end_byte -= 1;
        }

        let line_str = &content[line_start_byte..line_end_byte];
        let line_chars: Vec<(usize, char)> = line_str.char_indices().collect();
        let line_char_count = line_chars.len();

        let node_text = content
            .get(node_start_offset..node_end_offset)
            .unwrap_or("");
        let node_char_count = node_text.chars().count();

        let start_col = cmp::min(column, line_char_count);
        let end_col = cmp::min(start_col + node_char_count, line_char_count);

        let node_center = start_col + (end_col - start_col) / 2;
        let mut win_start = node_center.saturating_sub(w / 2);
        let mut win_end = win_start + w;
        if win_end > line_char_count {
            let overflow = win_end - line_char_count;
            win_start = win_start.saturating_sub(overflow);
            win_end = line_char_count;
        }

        let get_char_byte_offset = |char_idx: usize| -> usize {
            if char_idx >= line_char_count {
                line_end_byte
            } else {
                line_start_byte + line_chars[char_idx].0
            }
        };

        let win_start_byte = get_char_byte_offset(win_start);
        let win_end_byte = get_char_byte_offset(win_end);
        let snippet_text = &content[win_start_byte..win_end_byte];

        if supports_color {
            // Resilient highlight of the full file, then crop spans to the window. Guarded on
            // non-empty so a file that can't be parsed/completed falls through to plain text
            // rather than panicking (old behavior) or emitting an empty snippet.
            let spans = highlight_resilient(&content);
            if !spans.is_empty() {
                let mut snippet_spans = Vec::new();
                for span in spans {
                    let overlap_start = cmp::max(span.start, win_start_byte);
                    let overlap_end = cmp::min(span.end, win_end_byte);
                    if overlap_start < overlap_end {
                        snippet_spans.push(HighlightSpan {
                            start: overlap_start - win_start_byte,
                            end: overlap_end - win_start_byte,
                            htype: span.htype,
                            counter: span.counter,
                        });
                    }
                }
                if !snippet_spans.is_empty() {
                    return Some(format_ansi(snippet_text, snippet_spans));
                }
            }
        }

        Some(snippet_text.to_string())
    }
}
