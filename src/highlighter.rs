//! ANSI rendering for Quoin syntax highlighting.
//!
//! The AST→span core (the [`HighlightParser`], [`HighlightType`],
//! [`HighlightSpan`], and the [`colors_for`] palette) now lives in
//! `quoin_syntax::highlight`; this module re-exports it and keeps the ANSI
//! formatter used by `qn highlight`, which depends on the VM's `ansi_colorizer`.

pub use quoin_syntax::highlight::*;

use crate::ansi_colorizer;

fn slice(source: &str, start: usize, end: usize) -> &str {
    source.get(start..end).unwrap_or("")
}

fn color_for(htype: HighlightType, counter: usize) -> &'static str {
    let colors = colors_for(htype);
    colors[counter % colors.len()]
}

/// Render highlight spans into an ANSI-colored string.
pub fn format_ansi(source: &str, mut spans: Vec<HighlightSpan>) -> String {
    // Order by (start, end); drop later spans that share a range with an
    // earlier one (spans covering an equal range are treated as duplicates).
    spans.sort_by(|a, b| (a.start, a.end).cmp(&(b.start, b.end)));
    spans.dedup_by(|a, b| a.start == b.start && a.end == b.end);

    // Emit SGR directly from the palette specs — no markup round-trip, so the
    // source text needs no escaping and can never collide with the markup grammar.
    let mut sb = String::new();
    for span in &spans {
        sb.push_str(&ansi_colorizer::sgr(color_for(span.htype, span.counter)));
        sb.push_str(slice(source, span.start, span.end));
        sb.push_str(ansi_colorizer::SGR_RESET);
    }
    sb
}

/// Parse (or, for incomplete input, predictively complete), highlight, and ANSI-format a
/// source string. Resilient: it never panics and returns the source unchanged when there are
/// no spans (an uncompletable line), so a live-highlighting caller's text/cursor stay correct.
pub fn highlight_to_ansi(source: &str) -> String {
    let spans = highlight_resilient(source);
    if spans.is_empty() {
        // Uncompletable (or empty) input: render the text verbatim. `format_ansi` would emit
        // `""` here (it builds output only from spans), which would corrupt the visible line.
        return source.to_string();
    }
    format_ansi(source, spans)
}

// ---- HTML rendering (docs/DOCS_ARCH.md §8) ---------------------------------------------
//
// A second formatter over the same `HighlightSpan`s ANSI renders — one span model, two
// consumers. `qn highlight --html` and the doc generator's fenced examples both call these,
// so "docs and the highlighter share code styles" is true by construction: the class names
// come from `css_class`, and the dark-scheme colors are generated from the SAME `colors_for`
// table the terminal uses. Light-scheme colors are hand-picked (the ANSI palette is tuned for
// a dark terminal; several entries are unreadable on white).

/// The CSS class for a highlight type — `qn-` + a stable kebab name. Types whose ANSI palette
/// rotates (identifiers, block braces) get a `-N` suffix appended by `format_html`.
pub fn css_class(htype: HighlightType) -> &'static str {
    match htype {
        HighlightType::None => "qn-plain",
        HighlightType::ErrorStatement => "qn-error",
        HighlightType::MethodReturnStatement => "qn-return",
        HighlightType::BlockReturnStatement => "qn-return",
        HighlightType::NumberLiteral => "qn-number",
        HighlightType::StringLiteral => "qn-string",
        HighlightType::SymbolLiteral => "qn-symbol",
        HighlightType::RegexLiteral => "qn-regex",
        HighlightType::Identifier => "qn-ident",
        HighlightType::InstanceIdentifier => "qn-ivar",
        HighlightType::BlockBrace => "qn-brace",
        HighlightType::CollectionBrace => "qn-collection",
        HighlightType::Operator => "qn-op",
        HighlightType::Comment => "qn-comment",
        HighlightType::MethodSignature => "qn-signature",
        HighlightType::Global => "qn-global",
        HighlightType::Namespace => "qn-namespace",
        HighlightType::Keyword => "qn-keyword",
        HighlightType::Path => "qn-path",
    }
}

/// Every type with a palette, for stylesheet generation.
const ALL_TYPES: &[HighlightType] = &[
    HighlightType::ErrorStatement,
    HighlightType::MethodReturnStatement,
    HighlightType::NumberLiteral,
    HighlightType::StringLiteral,
    HighlightType::SymbolLiteral,
    HighlightType::RegexLiteral,
    HighlightType::Identifier,
    HighlightType::InstanceIdentifier,
    HighlightType::BlockBrace,
    HighlightType::CollectionBrace,
    HighlightType::Operator,
    HighlightType::Comment,
    HighlightType::MethodSignature,
    HighlightType::Global,
    HighlightType::Namespace,
    HighlightType::Keyword,
    HighlightType::Path,
];

/// An ANSI palette entry is `#rrggbb` with an optional `;xx` modifier (`bw` bold, `lw` light).
fn split_entry(entry: &str) -> (&str, Option<&str>) {
    match entry.split_once(';') {
        Some((hex, m)) => (hex, Some(m)),
        None => (entry, None),
    }
}

/// The `<head>` links loading the code font — Fira Code, whose ligatures suit Quoin's
/// arrow-heavy syntax (`->`, `-->`, `<-`, `<--`, `==`, `!=`, `>=`). Served from jsDelivr's
/// copy of the OFFICIAL distribution, not Google Fonts: Google's pipeline strips the
/// discretionary GSUB features during subsetting (measured on the served TTF: only
/// `calt dnom frac locl numr tnum` survive), so the `ss05` `@` variant the stylesheet asks
/// for would be a silent no-op there; the jsDelivr file carries all `ss01`–`ss10` and
/// `cv01`–`cv32`. Version-pinned for determinism. Linked rather than copied into generated
/// folders; the `ui-monospace` fallback in [`code_stylesheet`] means an offline page simply
/// renders in the system code font. The one sanctioned external resource — scripts stay
/// forbidden (tests/doc_gen.rs pins both).
pub fn code_font_links() -> &'static str {
    concat!(
        "<link rel=\"preconnect\" href=\"https://cdn.jsdelivr.net\" crossorigin>\n",
        "<link rel=\"stylesheet\" ",
        "href=\"https://cdn.jsdelivr.net/npm/firacode@6.2.0/distr/fira_code.css\">"
    )
}

/// The code stylesheet both consumers inline. Dark-scheme colors are generated from the ANSI
/// `colors_for` table (exact terminal parity); light-scheme from a hand-picked map, since the
/// terminal palette assumes a dark background — `#ffffff` operators would vanish on white.
pub fn code_stylesheet() -> String {
    // Light-scheme counterpart per class (rotations collapse to one light color; the
    // rotation's job — telling nesting levels apart — matters most in the dark theme's
    // saturated palette, and light keeps the page calm).
    fn light(class: &str) -> &'static str {
        match class {
            "qn-error" => "#b3261e",
            "qn-number" => "#0868a5",
            "qn-string" | "qn-symbol" | "qn-regex" => "#20639b",
            "qn-ident" => "#1f7a5c",
            "qn-ivar" => "#2b7a8c",
            "qn-brace" => "#c05e2f",
            "qn-collection" => "#3d8156",
            "qn-comment" => "#767b76",
            "qn-signature" => "#6b4bb8",
            "qn-global" => "#b0316f",
            "qn-namespace" => "#a02a68",
            "qn-keyword" => "#9a6410",
            "qn-path" => "#33689c",
            _ => "inherit", // qn-plain / qn-op / qn-return: default foreground
        }
    }
    // The font rule lives HERE, not in each consumer's page style, so the doc pages and
    // `qn highlight --html` cannot disagree. Emitted after the page styles in both consumers,
    // so it wins over any `font:` shorthand they set for sizing. `ss05` selects Fira Code's
    // `@` variant; features not named keep their defaults, so the `calt` ligatures stay on.
    let mut css = String::from(
        "pre.qn-code, pre, code, .sig { font-family: 'Fira Code', ui-monospace, \
         SFMono-Regular, Menlo, monospace; font-feature-settings: 'ss05'; }\n\
         pre.qn-code { line-height: 1.45; }\n",
    );
    let mut seen: Vec<&str> = Vec::new();
    for &t in ALL_TYPES {
        let class = css_class(t);
        if seen.contains(&class) {
            continue;
        }
        seen.push(class);
        let palette = colors_for(t);
        // Light scheme (the page default).
        let (_, modifier) = split_entry(palette[0]);
        let weight = if modifier == Some("bw") {
            " font-weight: 600;"
        } else {
            ""
        };
        css.push_str(&format!(
            ".{class} {{ color: {};{weight} }}\n",
            light(class)
        ));
        // Dark scheme: the terminal palette verbatim, rotation entries included.
        for (i, entry) in palette.iter().enumerate() {
            let (hex, _) = split_entry(entry);
            let color = if hex == "#ffffff" {
                "inherit".to_string()
            } else {
                hex.to_string()
            };
            let selector = if palette.len() > 1 {
                format!(".{class}-{i}")
            } else {
                format!(".{class}")
            };
            if palette.len() > 1 {
                // Rotation classes need a light rule too, or they'd be unstyled there.
                css.push_str(&format!("{selector} {{ color: {}; }}\n", light(class)));
            }
            css.push_str(&format!(
                "@media (prefers-color-scheme: dark) {{ {selector} {{ color: {color};{weight} }} }}\n"
            ));
        }
    }
    css
}

fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Render highlight spans as HTML `<span class="qn-…">` runs. Text between spans (if any) is
/// emitted escaped and unclassed, so nothing is ever dropped; a span overlapping the cursor
/// (defensive — the walker emits disjoint spans) is skipped rather than duplicating text.
pub fn format_html(source: &str, mut spans: Vec<HighlightSpan>) -> String {
    spans.sort_by(|a, b| (a.start, a.end).cmp(&(b.start, b.end)));
    spans.dedup_by(|a, b| a.start == b.start && a.end == b.end);

    let mut out = String::new();
    let mut cursor = 0usize;
    for span in &spans {
        if span.start < cursor {
            continue;
        }
        if span.start > cursor {
            out.push_str(&html_escape(slice(source, cursor, span.start)));
        }
        let base = css_class(span.htype);
        let class = if colors_for(span.htype).len() > 1 {
            format!(
                "{base} {base}-{}",
                span.counter % colors_for(span.htype).len()
            )
        } else {
            base.to_string()
        };
        out.push_str(&format!(
            "<span class=\"{class}\">{}</span>",
            html_escape(slice(source, span.start, span.end))
        ));
        cursor = span.end;
    }
    out.push_str(&html_escape(slice(source, cursor, source.len())));
    out
}

/// Parse (resiliently) and render `source` as an HTML fragment inside `<pre class="qn-code">`.
/// Uncompletable input renders escaped but unstyled — same fallback as the ANSI path.
pub fn highlight_to_html(source: &str) -> String {
    let spans = highlight_resilient(source);
    let body = if spans.is_empty() {
        html_escape(source)
    } else {
        format_html(source, spans)
    };
    format!("<pre class=\"qn-code\">{body}</pre>")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn types(spans: &[HighlightSpan]) -> Vec<HighlightType> {
        spans.iter().map(|s| s.htype).collect()
    }

    fn highlight(source: &str) -> Vec<HighlightSpan> {
        let program = crate::parser::parse_quoin_string(source);
        let mut parser = HighlightParser::new(source);
        parser.highlight_program(&program)
    }

    #[test]
    fn highlight_to_ansi_preserves_text() {
        // Live highlighting requires the rendered (decolorized) text to equal the input
        // exactly — including for incomplete input, where the predictive completion is
        // highlighted but cropped — or the editor's cursor positioning breaks.
        for src in [
            "1 + 2 * foo", // valid
            "1 +",         // trailing operator
            "Foo <- {",    // open block
            "#(1 2",       // open list
            "'hello",      // open string
            "x = 1 +",     // assignment + trailing op
            "a.foo:",      // open keyword selector
            "Box <--",     // definition operator
            ")",           // uncompletable — rendered verbatim
        ] {
            let plain = ansi_colorizer::decolorize(&highlight_to_ansi(src));
            assert_eq!(plain, src, "text not preserved for {src:?}");
        }
    }

    #[test]
    fn number_literal_is_tagged() {
        let spans = highlight("123;");
        assert!(
            types(&spans).contains(&HighlightType::NumberLiteral),
            "{spans:?}"
        );
        // the NumberLiteral span should cover "123"
        let num = spans
            .iter()
            .find(|s| s.htype == HighlightType::NumberLiteral)
            .unwrap();
        assert_eq!(&"123;"[num.start..num.end], "123");
    }

    #[test]
    fn string_literal_is_tagged() {
        let spans = highlight("'hello';");
        let s = spans
            .iter()
            .find(|s| s.htype == HighlightType::StringLiteral)
            .unwrap();
        assert_eq!(&"'hello';"[s.start..s.end], "'hello'");
    }

    #[test]
    fn identifier_is_tagged() {
        let spans = highlight("foo;");
        let s = spans
            .iter()
            .find(|s| s.htype == HighlightType::Identifier)
            .unwrap();
        assert_eq!(&"foo;"[s.start..s.end], "foo");
    }

    #[test]
    fn global_for_uppercase_and_reserved_ident() {
        let spans = highlight("Foo;");
        assert!(types(&spans).contains(&HighlightType::Global), "{spans:?}");

        let spans = highlight("nil;");
        assert!(types(&spans).contains(&HighlightType::Global), "{spans:?}");
    }

    #[test]
    fn instance_identifier_is_tagged() {
        let spans = highlight("@x;");
        let s = spans
            .iter()
            .find(|s| s.htype == HighlightType::InstanceIdentifier)
            .unwrap();
        assert_eq!(&"@x;"[s.start..s.end], "@x");
    }

    #[test]
    fn block_braces_are_tagged() {
        let spans = highlight("{ 1 };");
        let braces: Vec<_> = spans
            .iter()
            .filter(|s| s.htype == HighlightType::BlockBrace)
            .collect();
        assert_eq!(braces.len(), 2, "{spans:?}");
    }

    #[test]
    fn collection_braces_are_tagged() {
        let spans = highlight("#(1 2);");
        assert!(
            types(&spans).contains(&HighlightType::CollectionBrace),
            "{spans:?}"
        );
    }

    #[test]
    fn method_signature_is_tagged() {
        let spans = highlight("x.foo: 1;");
        assert!(
            types(&spans).contains(&HighlightType::MethodSignature),
            "{spans:?}"
        );
    }

    #[test]
    fn error_statement_is_tagged() {
        let spans = highlight("!!!;");
        assert!(
            types(&spans).contains(&HighlightType::ErrorStatement),
            "{spans:?}"
        );
    }

    #[test]
    fn namespace_is_tagged() {
        let spans = highlight("[foo/bar]baz;");
        assert!(
            types(&spans).contains(&HighlightType::Namespace),
            "{spans:?}"
        );
    }

    #[test]
    fn namespaced_type_annotation_is_tagged() {
        // A namespaced type in annotation position gets the same namespace hue as an
        // expression-position reference — in a block arg and in a `var x: T` decl.
        let spans = highlight("{ |e:[Web]Halt| e };");
        assert!(
            types(&spans).contains(&HighlightType::Namespace),
            "{spans:?}"
        );
        let spans = highlight("var x: [IO]File = 1;");
        assert!(
            types(&spans).contains(&HighlightType::Namespace),
            "{spans:?}"
        );
    }

    #[test]
    fn use_with_package_tags_keyword_package_path() {
        let src = "use std:io/file;";
        let spans = highlight(src);
        let kw = spans
            .iter()
            .find(|s| s.htype == HighlightType::Keyword)
            .unwrap();
        assert_eq!(&src[kw.start..kw.end], "use");
        let pkg = spans
            .iter()
            .find(|s| s.htype == HighlightType::Namespace)
            .unwrap();
        assert_eq!(&src[pkg.start..pkg.end], "std:");
        let path = spans
            .iter()
            .find(|s| s.htype == HighlightType::Path)
            .unwrap();
        assert_eq!(&src[path.start..path.end], "io/file");
    }

    #[test]
    fn use_without_package_globs_path_and_roundtrips() {
        let src = "use core/*;";
        let spans = highlight(src);
        assert!(
            !types(&spans).contains(&HighlightType::Namespace),
            "{spans:?}"
        );
        let path = spans
            .iter()
            .find(|s| s.htype == HighlightType::Path)
            .unwrap();
        assert_eq!(&src[path.start..path.end], "core/*");
        // colors strip back to the original source
        assert_eq!(ansi_colorizer::decolorize(&highlight_to_ansi(src)), src);
    }

    #[test]
    fn use_as_identifier_is_not_a_keyword() {
        // `use` is a soft keyword — as a plain variable it must not be tagged Keyword.
        let spans = highlight("use = 5;");
        assert!(
            !types(&spans).contains(&HighlightType::Keyword),
            "{spans:?}"
        );
    }

    #[test]
    fn block_comment_tagged_trailing_text_is_none() {
        // A `"..."` block comment between tokens, with whitespace after it.
        let source = "foo \"a comment\" ;";
        let spans = highlight(source);
        let comment = spans
            .iter()
            .find(|s| s.htype == HighlightType::Comment)
            .expect("expected a comment span");
        assert_eq!(&source[comment.start..comment.end], "\"a comment\"");
        // No span past the comment should be mislabeled as a comment.
        assert!(
            spans
                .iter()
                .all(|s| s.htype != HighlightType::Comment || s.start <= comment.start),
            "trailing text was tagged as a comment: {spans:?}"
        );
    }

    #[test]
    fn namespaced_identifier_not_duplicated() {
        // Regression: the namespace prefix must not be emitted twice.
        let source = "x = [IO]File;";
        let ansi = highlight_to_ansi(source);
        assert_eq!(ansi_colorizer::decolorize(&ansi), source);

        // The namespace span and the name span must not overlap.
        let spans = highlight(source);
        let ns = spans
            .iter()
            .find(|s| s.htype == HighlightType::Namespace)
            .unwrap();
        let name = spans
            .iter()
            .find(|s| s.htype == HighlightType::Global)
            .unwrap();
        assert_eq!(&source[ns.start..ns.end], "[IO]");
        assert_eq!(&source[name.start..name.end], "File");
    }

    #[test]
    fn format_ansi_roundtrips_to_plain_text() {
        let source = "foo + 123;";
        let ansi = highlight_to_ansi(source);
        // Stripping the ANSI codes should recover the original source.
        assert_eq!(ansi_colorizer::decolorize(&ansi), source);
    }

    #[test]
    fn format_ansi_emits_escape_codes() {
        let ansi = highlight_to_ansi("123;");
        assert!(ansi.contains('\x1b'), "expected ANSI codes in {ansi:?}");
    }
}
