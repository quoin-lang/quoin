//! ANSI rendering for Quoin syntax highlighting.
//!
//! The AST→span core (the [`HighlightParser`], [`HighlightType`],
//! [`HighlightSpan`], and the [`colors_for`] palette) now lives in
//! `quoin_syntax::highlight`; this module re-exports it and keeps the ANSI
//! formatter used by `qn highlight`, which depends on the VM's `ansi_colorizer`.

pub use quoin_syntax::highlight::*;

use crate::ansi_colorizer;
use crate::parser::parse_quoin_string;

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

    let mut sb = String::new();
    for span in &spans {
        sb.push('$');
        sb.push_str(color_for(span.htype, span.counter));
        sb.push('[');
        sb.push_str(&ansi_colorizer::escape(slice(source, span.start, span.end)));
        sb.push_str("$]");
    }

    ansi_colorizer::colorize(&sb)
}

/// Convenience: parse, highlight, and ANSI-format a source string.
pub fn highlight_to_ansi(source: &str) -> String {
    let program = parse_quoin_string(source);
    let mut parser = HighlightParser::new(source);
    let spans = parser.highlight_program(&program);
    format_ansi(source, spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn types(spans: &[HighlightSpan]) -> Vec<HighlightType> {
        spans.iter().map(|s| s.htype).collect()
    }

    fn highlight(source: &str) -> Vec<HighlightSpan> {
        let program = parse_quoin_string(source);
        let mut parser = HighlightParser::new(source);
        parser.highlight_program(&program)
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
