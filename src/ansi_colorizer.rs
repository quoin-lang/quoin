use once_cell::sync::Lazy;
use regex::Regex;

const ANSI_PREFIX: &str = "\x1b[";
const ANSI_SUFFIX: &str = "m";
const RESET_ALL: &str = "\x1b[0;00;22;39;49m";

static STRIP_ANSI_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\x1b\[.+?m").unwrap());
// Match either $$ (escaped dollar) or $attr[text$] (color pattern).
// Ordered alternation ensures $$ is consumed before it could start a color pattern.
static COLORIZE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\$\$|\$(?P<attr>[^\[\n]+?)\[(?P<text>(?:\$\$|[^$])+?)\$]").unwrap());
static HEX_COLOR_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"#([A-Fa-f0-9]{2})([A-Fa-f0-9]{2})([A-Fa-f0-9]{2})").unwrap());

pub fn escape(text: &str) -> String {
    text.replace('$', "$$")
}

pub fn decolorize(text: &str) -> String {
    STRIP_ANSI_RE.replace_all(text, "").into_owned()
}

pub fn colorize(text: &str) -> String {
    COLORIZE_RE
        .replace_all(text, |caps: &regex::Captures| {
            // If the match is just "$$", leave it as-is (literal escaped dollar).
            if caps.get(0).map(|m| m.as_str()) == Some("$$") {
                return "$$".to_string();
            }
            let attr = parse_attribute(&caps["attr"]);
            let body = caps["text"].replace("$$", "$");
            format!("{attr}{body}{RESET_ALL}")
        })
        .into_owned()
}

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
    fn escape_doubles_dollar_signs() {
        assert_eq!(escape("hello $world"), "hello $$world");
    }

    #[test]
    fn decolorize_strips_ansi() {
        let colored = "\x1b[1mhello\x1b[0m";
        assert_eq!(decolorize(colored), "hello");
    }

    #[test]
    fn colorize_bold() {
        let result = colorize("$bw[hello$]");
        assert!(result.contains("hello"));
        assert!(result.contains("\x1b["));
        assert!(result.contains(RESET_ALL));
    }

    #[test]
    fn colorize_hex_color() {
        let result = colorize("$#ff0000[red$]");
        assert!(result.contains("38;2;255;0;0"), "got: {result}");
        assert!(result.contains("red"));
    }

    #[test]
    fn escaped_dollar_not_colorized() {
        // $$ is left alone; the rest is plain text, so no ANSI codes appear
        let result = colorize("$$bw[not colored$]");
        assert!(!result.contains("\x1b["), "got: {result}");
    }

    #[test]
    fn dollar_dollar_in_text_becomes_dollar() {
        let result = colorize("$bw[price: $$5$]");
        assert!(result.contains("price: $5"), "got: {result}");
    }

    #[test]
    fn colorize_tolerates_unknown_attributes() {
        // An unescaped '$' in a value can make the regex match a garbage attribute
        // (this exact input — "] $#bw" — once panicked the whole test reporter).
        // Hardening: unknown attributes are skipped, never panicked, and the text
        // still renders.
        let result = colorize("$] $#bw[oops$]");
        assert!(result.contains("oops"), "got: {result}");
    }
}
