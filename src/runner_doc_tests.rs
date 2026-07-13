//! The `--md` code-span linker's contract: exact class names (bare, namespaced,
//! nullable) and `Class#sel:` / `Class.sel:` citations link; everything else —
//! unknown names, filenames, decimals, operator selectors — is left alone.

use super::*;

fn link(span: &str) -> Option<String> {
    let known = |c: &str| matches!(c, "List" | "[IO]File" | "Integer" | "Fiber");
    code_span_link(span, &known, &|c| format!("../reference/{}", page_name(c)))
}

#[test]
fn bare_class_names_link() {
    assert_eq!(link("List").as_deref(), Some("../reference/List.html"));
    assert_eq!(
        link("[IO]File").as_deref(),
        Some("../reference/IO.File.html")
    );
    assert_eq!(
        link("Integer?").as_deref(),
        Some("../reference/Integer.html"),
        "the nullable spelling cites the base class"
    );
    assert_eq!(link("Foo"), None);
    assert_eq!(link("Foo?"), None);
}

#[test]
fn method_citations_deep_link_their_anchors() {
    assert_eq!(
        link("List#at:put:").as_deref(),
        Some("../reference/List.html#i-at:put:")
    );
    assert_eq!(
        link("Fiber.yield:").as_deref(),
        Some("../reference/Fiber.html#c-yield:")
    );
    assert_eq!(
        link("[IO]File#exists?:").as_deref(),
        Some("../reference/IO.File.html#i-exists?:"),
        "namespaced receiver, `?` selector"
    );
    assert_eq!(
        link("List#size").as_deref(),
        Some("../reference/List.html#i-size")
    );
    assert_eq!(
        link("Fiber.sleep:ms").as_deref(),
        Some("../reference/Fiber.html#c-sleep:"),
        "a glued argument name drops out of the anchor"
    );
}

#[test]
fn non_citations_stay_plain() {
    assert_eq!(link("Foo#bar:"), None, "unknown class");
    assert_eq!(link("x.md"), None, "filename");
    assert_eq!(link("README.md"), None);
    assert_eq!(link("3.14"), None, "decimal");
    assert_eq!(link("List#=="), None, "operator selectors are not citable");
    assert_eq!(link("List#"), None, "empty selector");
    assert_eq!(link("qn doc"), None, "prose-shaped spans");
}
