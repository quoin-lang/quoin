//! The minimal-markdown contract: exactly the constructs the corpus uses, and the
//! protections that keep prose honest (code spans shield `*`, unpaired markers stay
//! literal, unknown constructs degrade to text instead of mangling).

use super::*;

#[test]
fn headings_get_slug_anchors() {
    let html = render("## 23. Sockets & streams\n", false);
    assert_eq!(
        html,
        "<h2 id=\"23-sockets-streams\">23. Sockets &amp; streams</h2>\n"
    );
}

#[test]
fn code_spans_shield_emphasis_and_links() {
    let html = render("run `qn fmt lib/*.qn bin/*` twice\n", false);
    assert!(html.contains("<code>qn fmt lib/*.qn bin/*</code>"));
    assert!(!html.contains("<em>"), "the * inside code is not emphasis");
    let html = render("a `x` *b* **c** [d](e.md)\n", true);
    assert!(html.contains("<em>b</em>"));
    assert!(html.contains("<strong>c</strong>"));
    assert!(html.contains("<a href=\"e.html\">d</a>"));
}

#[test]
fn code_span_labels_still_linkify() {
    // The book index's chapter links: the label IS a code span.
    let html = render("### Part I · [`01-foundations.md`](01-foundations.md)\n", true);
    assert!(
        html.contains("<a href=\"01-foundations.html\"><code>01-foundations.md</code></a>"),
        "a [label](url) whose label is a code span is one link: {html}"
    );
    let html = render("see [the `qn` CLI](08-tooling.md) for more\n", true);
    assert!(html.contains("<a href=\"08-tooling.html\">the <code>qn</code> CLI</a>"));
}

#[test]
fn unpaired_markers_stay_literal() {
    assert!(render("a ` b\n", false).contains("a ` b"));
    assert!(
        render("2 * 3 * 4\n", false).contains("2 * 3 * 4"),
        "spaced stars are math, not emphasis"
    );
}

#[test]
fn quoin_fences_highlight_and_norun_too() {
    let html = render("```quoin\nvar x = 1\n```\n", false);
    assert!(
        html.contains("qn-code"),
        "quoin fences go through the highlighter"
    );
    let html = render("```quoin norun\nvar x = 1\n```\n", false);
    assert!(
        html.contains("qn-code"),
        "norun means don't RUN, not don't highlight"
    );
    let html = render("```\nplain <text>\n```\n", false);
    assert!(html.contains("<pre>plain &lt;text&gt;</pre>"));
}

#[test]
fn blockquotes_contain_lists_and_fences() {
    let md = "> **Rules**\n> - one `a`\n> - two\n>\n> ```quoin\n> 1 + 1\n> ```\n";
    let html = render(md, false);
    assert!(html.contains("<blockquote>"));
    assert!(html.contains("<strong>Rules</strong>"));
    assert!(html.contains("<li>one <code>a</code></li>"));
    assert!(
        html.contains("qn-code"),
        "a fence inside a quote box still highlights"
    );
}

#[test]
fn tables_render_with_headers() {
    let md = "| Command | Does |\n|---|---|\n| `$c` | resume |\n";
    let html = render(md, false);
    assert!(html.contains("<th>Command</th>"));
    assert!(html.contains("<td><code>$c</code></td>"));
    assert!(
        !html.contains("---"),
        "the separator row is structure, not content"
    );
}

#[test]
fn table_cells_keep_pipes_inside_code_spans() {
    let md = "| Kind | Example |\n|---|---|\n| Block | `{ |n| n * 2 }` |\n";
    let html = render(md, false);
    assert!(
        html.contains("<td><code>{ |n| n * 2 }</code></td>"),
        "a | inside a code span is content, not a separator: {html}"
    );
    // GitHub-style escaped pipes are unescaped content, in and out of code spans.
    let md = "| Op | Note |\n|---|---|\n| `a \\|\\| b` | or \\| pipe |\n";
    let html = render(md, false);
    assert!(html.contains("<td><code>a || b</code></td>"), "{html}");
    assert!(html.contains("<td>or | pipe</td>"), "{html}");
}

#[test]
fn lists_ordered_and_wrapped() {
    let md = "1. first\n2. second line\n   wraps here\n\n- bullet\n";
    let html = render(md, false);
    assert!(html.contains("<ol>"));
    assert!(html.contains("<li>second line wraps here</li>"));
    assert!(html.contains("<ul>\n<li>bullet</li>"));
}

#[test]
fn md_links_rewrite_only_when_asked() {
    let md = "[next](02-blocks.md) [toc](README.md#top) [ext](https://x.y/a.md)\n";
    let on = render(md, true);
    assert!(on.contains("href=\"02-blocks.html\""));
    assert!(
        on.contains("href=\"index.html#top\""),
        "README maps to index, fragment kept"
    );
    assert!(
        on.contains("href=\"https://x.y/a.md\""),
        "absolute URLs pass through"
    );
    let off = render(md, false);
    assert!(off.contains("href=\"02-blocks.md\""));
}

#[test]
fn title_comes_from_the_first_h1() {
    assert_eq!(
        title_of("intro\n\n# The Book\n").as_deref(),
        Some("The Book")
    );
    assert_eq!(title_of("no heading\n"), None);
}
