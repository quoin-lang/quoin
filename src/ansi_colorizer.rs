use once_cell::sync::Lazy;
use regex::Regex;

const ANSI_PREFIX: &str = "\x1b[";
const ANSI_SUFFIX: &str = "m";
const RESET_ALL: &str = "\x1b[0;00;22;39;49m";

// `.*?` (not `.+?`) so a zero-parameter SGR — `\x1b[m`, a valid reset, which
// `parse_attribute` emits when an attribute contributes no code (e.g. a `#xx` that
// isn't a 6-digit hex) — strips as a unit. With `.+?` the closing `m` was consumed as
// content, and the match ran on to the *next* `m`, deleting the visible text between.
static STRIP_ANSI_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\x1b\[.*?m").unwrap());
static HEX_COLOR_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"#([A-Fa-f0-9]{2})([A-Fa-f0-9]{2})([A-Fa-f0-9]{2})").unwrap());

/// Escape text for embedding in markup: doubles `[` so a value can never open a tag.
/// A lone `]` is always literal and needs no escaping.
pub fn escape(text: &str) -> String {
    text.replace('[', "[[")
}

pub fn decolorize(text: &str) -> String {
    STRIP_ANSI_RE.replace_all(text, "").into_owned()
}

/// One markup span's own attributes — also a style-stack frame, where it holds the
/// MERGED state (own + everything enclosing) so a close can restore precisely.
#[derive(Clone, Default)]
struct SpanStyle {
    /// SGR fg fragment: `"31"` (named) or `"38;2;r;g;b"` (hex).
    fg: Option<String>,
    /// SGR bg fragment: `"41"` or `"48;2;r;g;b"`.
    bg: Option<String>,
    /// Style codes in first-seen order, deduped.
    flags: Vec<&'static str>,
}

impl SpanStyle {
    /// The child state a tag opens inside `self`: the tag's colors override,
    /// its flags accumulate.
    fn merged_with(&self, tag: &SpanStyle) -> SpanStyle {
        let mut flags = self.flags.clone();
        for f in &tag.flags {
            if !flags.contains(f) {
                flags.push(f);
            }
        }
        SpanStyle {
            fg: tag.fg.clone().or_else(|| self.fg.clone()),
            bg: tag.bg.clone().or_else(|| self.bg.clone()),
            flags,
        }
    }

    /// The SGR sequence that establishes this style — `None` when there is
    /// nothing to establish (the root state).
    fn sgr(&self) -> Option<String> {
        let mut parts: Vec<&str> = self.flags.clone();
        if let Some(fg) = &self.fg {
            parts.push(fg);
        }
        if let Some(bg) = &self.bg {
            parts.push(bg);
        }
        if parts.is_empty() {
            return None;
        }
        Some(format!("{ANSI_PREFIX}{}{ANSI_SUFFIX}", parts.join(";")))
    }
}

fn style_flag(tok: &str) -> Option<&'static str> {
    Some(match tok {
        "bold" => "1",
        "dim" => "2",
        "italic" => "3",
        "underline" => "4",
        "underline2" => "21",
        "blink" => "5",
        "blink2" => "6",
        "reverse" => "7",
        "strike" => "9",
        _ => return None,
    })
}

/// The SGR color fragment a tag token denotes — a named color (classic palette
/// codes, so the terminal theme applies) or a `#rrggbb` hex (truecolor).
fn color_fragment(tok: &str, bg: bool) -> Option<String> {
    if let Some(hex) = tok.strip_prefix('#') {
        if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let n = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).unwrap();
        let plane = if bg { 48 } else { 38 };
        return Some(format!("{plane};2;{};{};{}", n(0), n(2), n(4)));
    }
    let tok = match tok {
        "gray" | "grey" => "bright-black",
        other => other,
    };
    let (name, bright) = match tok.strip_prefix("bright-") {
        Some(rest) => (rest, true),
        None => (tok, false),
    };
    let idx = match name {
        "black" => 0,
        "red" => 1,
        "green" => 2,
        "yellow" => 3,
        "blue" => 4,
        "magenta" => 5,
        "cyan" => 6,
        "white" => 7,
        _ => return None,
    };
    let base = match (bg, bright) {
        (false, false) => 30,
        (false, true) => 90,
        (true, false) => 40,
        (true, true) => 100,
    };
    Some((base + idx).to_string())
}

enum Tag {
    Open(SpanStyle),
    Close,
}

/// Parse a bracket run's content as a tag. `None` means "not markup" — the run
/// stays literal text, which is what keeps `[IO]Stdout`, list renderings, and
/// already-rendered SGR sequences safe to pass through the colorizer.
fn parse_tag(content: &str) -> Option<Tag> {
    if content.contains('\n') {
        return None;
    }
    let content = content.trim();
    if content == "/" {
        return Some(Tag::Close);
    }
    if content.is_empty() {
        return None;
    }
    let mut style = SpanStyle::default();
    let mut toks = content.split_whitespace();
    while let Some(tok) = toks.next() {
        if tok == "on" {
            style.bg = Some(color_fragment(toks.next()?, true)?);
        } else if let Some(f) = style_flag(tok) {
            if !style.flags.contains(&f) {
                style.flags.push(f);
            }
        } else {
            // Any token that isn't an attribute disqualifies the whole run
            // (the `?`: a non-color here answers None for the lot).
            style.fg = Some(color_fragment(tok, false)?);
        }
    }
    Some(Tag::Open(style))
}

/// Render Rich-style markup to ANSI: `[red bold]text[/]`, `[#ff6961 on #222222]…[/]`,
/// `[[` for a literal `[`. Spans NEST — a close restores the enclosing style (a
/// style stack), not the terminal default. A bracket run that doesn't parse as a
/// tag is literal text; unclosed spans are implicitly closed at end of string.
pub fn colorize(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    let mut stack: Vec<SpanStyle> = Vec::new();
    let mut rest = text;
    while let Some(pos) = rest.find('[') {
        out.push_str(&rest[..pos]);
        let after = &rest[pos + 1..];
        if let Some(stripped) = after.strip_prefix('[') {
            out.push('[');
            rest = stripped;
            continue;
        }
        // On any non-tag, emit the `[` alone and rescan from the next char, so a
        // real tag later in the same run (`[a[red]b`) still renders.
        let Some(close) = after.find(']') else {
            out.push('[');
            rest = after;
            continue;
        };
        match parse_tag(&after[..close]) {
            Some(Tag::Open(tag)) => {
                // Opening emits only the tag's OWN attributes — SGR is additive
                // from the enclosing state (colors overwrite, flags accumulate).
                if let Some(s) = tag.sgr() {
                    out.push_str(&s);
                }
                let parent = stack.last().cloned().unwrap_or_default();
                stack.push(parent.merged_with(&tag));
                rest = &after[close + 1..];
            }
            Some(Tag::Close) => {
                stack.pop();
                out.push_str(RESET_ALL);
                if let Some(restored) = stack.last().and_then(|s| s.sgr()) {
                    out.push_str(&restored);
                }
                rest = &after[close + 1..];
            }
            None => {
                out.push('[');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    if !stack.is_empty() {
        out.push_str(RESET_ALL);
    }
    out
}

/// The ANSI SGR start sequence for a `colors_for`-style attribute spec (`#rrggbb` plus optional
/// `;bw`/`;lw`/… attributes) — e.g. `#93c6a5` → `\x1b[38;2;147;198;165m`. Pairs with [`SGR_RESET`].
/// Used by the value pretty-printer to color tokens with the same palette as input highlighting.
pub(crate) fn sgr(spec: &str) -> String {
    parse_attribute(spec)
}

/// The reset that ends an [`sgr`] span.
pub(crate) const SGR_RESET: &str = RESET_ALL;

fn parse_attribute(s: &str) -> String {
    let mut result = String::from(ANSI_PREFIX);
    for part in s.split(';').map(str::trim).filter(|p| !p.is_empty()) {
        if let Some(hex) = part.strip_prefix('#') {
            result.push_str(&hex_color_to_ansi(&format!("#{hex}")));
        } else {
            let code = match part {
                "bw" => "1",
                "lw" => "2",
                "nw" => "22",
                "i" => "3",
                "_" => "4",
                "=" => "21",
                "-" => "9",
                "b0" => "25",
                "b1" => "5",
                "b9" => "6",
                "!" => "7",
                // Unknown attribute: skip it rather than crash. This colorizer
                // renders arbitrary text — e.g. test-failure values embedded in
                // #ANSI'…' templates — so a stray '$' or '[' in a value must never
                // panic the whole run. At worst the styling for that fragment is
                // dropped; the text itself still renders.
                _ => continue,
            };
            result.push(';');
            result.push_str(code);
        }
    }
    result.push_str(ANSI_SUFFIX);
    result
}

fn hex_color_to_ansi(hex: &str) -> String {
    let caps = match HEX_COLOR_RE.captures(hex) {
        Some(c) => c,
        None => return String::new(),
    };
    let r = u8::from_str_radix(&caps[1], 16).unwrap();
    let g = u8::from_str_radix(&caps[2], 16).unwrap();
    let b = u8::from_str_radix(&caps[3], 16).unwrap();
    format!("38;2;{r};{g};{b}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_doubles_open_brackets() {
        assert_eq!(escape("a [red] rag"), "a [[red] rag");
        assert_eq!(decolorize(&colorize(&escape("a [red] rag"))), "a [red] rag");
    }

    #[test]
    fn decolorize_strips_ansi() {
        let colored = "\x1b[1mhello\x1b[0m";
        assert_eq!(decolorize(colored), "hello");
    }

    #[test]
    fn decolorize_strips_empty_sgr_without_eating_text() {
        // A zero-parameter SGR (`\x1b[m`, a valid reset) must strip on its own, not
        // swallow the text up to the next `m`.
        let s = "\x1b[m!=\x1b[0;00;22;39;49m";
        assert_eq!(decolorize(s), "!=");
    }

    #[test]
    fn colorize_styles_and_hex() {
        let result = colorize("[bold]hello[/]");
        assert!(result.starts_with("\x1b[1m"), "got: {result:?}");
        assert!(result.contains("hello") && result.ends_with(RESET_ALL));

        let result = colorize("[#ff0000]red[/]");
        assert!(result.contains("38;2;255;0;0"), "got: {result:?}");

        // One tag, several attributes; `on` takes the background color.
        let result = colorize("[bold #ff0000 on #002200]x[/]");
        assert!(
            result.contains("1;38;2;255;0;0;48;2;0;34;0"),
            "got: {result:?}"
        );
    }

    #[test]
    fn named_colors_use_the_terminal_palette() {
        assert!(colorize("[red]r[/]").contains("\x1b[31m"));
        assert!(colorize("[bright-cyan]c[/]").contains("\x1b[96m"));
        assert!(colorize("[gray]g[/]").contains("\x1b[90m"));
        assert!(colorize("[white on red]w[/]").contains("\x1b[37;41m"));
    }

    #[test]
    fn nesting_restores_the_enclosing_style() {
        // [red]a [bold]b[/] c[/] — after the inner close, red is re-established
        // (RESET + the restored frame's SGR), not dropped to the default.
        let result = colorize("[red]a [bold]b[/] c[/]");
        let expected = format!(
            "\x1b[31ma \x1b[1mb{RESET_ALL}\x1b[31m c{RESET_ALL}",
            RESET_ALL = RESET_ALL
        );
        assert_eq!(result, expected);
    }

    #[test]
    fn unknown_bracket_runs_stay_literal() {
        // Not every bracket is markup: namespaced classes, suite prefixes, and
        // arbitrary values pass through untouched.
        assert_eq!(colorize("[IO]Stdout"), "[IO]Stdout");
        assert_eq!(colorize("a[1] = 2"), "a[1] = 2");
        assert_eq!(colorize("[no such attr]x"), "[no such attr]x");
        // ...and a literal run does not hide a real tag later in the string.
        assert_eq!(
            decolorize(&colorize("[Suite] [red]FAIL[/]")),
            "[Suite] FAIL"
        );
    }

    #[test]
    fn escaped_bracket_prints_one_bracket() {
        assert_eq!(colorize("[[red]"), "[red]");
        assert_eq!(decolorize(&colorize("[[[#ff0000]x[/]]")), "[x]");
    }

    #[test]
    fn already_rendered_sgr_passes_through() {
        // colorize over its own output must not corrupt it: SGR sequences contain
        // no `]`, so their `[` is emitted literally.
        let once = colorize("[red]x[/]");
        assert_eq!(colorize(&once), once);
    }

    #[test]
    fn unclosed_span_resets_at_end() {
        let result = colorize("[red]never closed");
        assert!(result.ends_with(RESET_ALL), "got: {result:?}");
    }

    #[test]
    fn stray_close_resets_to_base() {
        // A close with nothing open is markup (reset), not a panic and not garbage.
        let result = colorize("a [/] b");
        assert_eq!(decolorize(&result), "a  b");
    }

    #[test]
    fn empty_span_does_not_leak() {
        // An interpolated value that rendered as "" leaves an empty span; it must
        // render as open+reset with the surrounding text intact.
        let result = colorize("a [#ff6961][/] at [#ffffff]?[/]");
        assert!(result.contains("38;2;255;105;97"), "got: {result:?}");
        assert_eq!(decolorize(&result), "a  at ?");
    }
}
