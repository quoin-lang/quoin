//! Markdown → HTML, for `qn doc`: the project README preamble and the `--md` page
//! renderer (the book build — GitHub can't highlight Quoin; this pipeline can).
//!
//! Deliberately MINIMAL, not CommonMark: exactly the constructs the corpus uses —
//! ATX headings (with slug anchors), paragraphs, `-` and `1.` lists, tables,
//! `---` rules, blockquotes (which may CONTAIN lists and fences — the book's
//! rule/gotcha boxes), fenced code (`quoin`/`quoin norun` render through the shared
//! highlighter; anything else is a plain `<pre>`), and an inline layer — backtick
//! spans (protected from the other passes: a `*` inside `` `lib/*.qn` `` is code,
//! not emphasis), `**bold**`, `*italic*`, `[text](url)` links. Everything outside
//! the set renders as literal prose — honest, never mangled.

use std::fmt::Write as _;

/// A code-span resolver: the span's text (HTML-escaped, exactly as it will render)
/// → an href, or `None` to leave the span unlinked. The caller owns the policy —
/// which names are classes, where their pages live (`qn doc`'s book→reference links).
pub type CodeLinker<'a> = dyn Fn(&str) -> Option<String> + 'a;

/// What the block passes thread through to the inline layer.
struct Ctx<'a> {
    /// Map relative `*.md` hrefs to their rendered names (`x.md` → `x.html`,
    /// `README.md` → `index.html`) — on for a rendered SET (`--md`), off for a lone
    /// README preamble whose links point at repository files.
    rewrite: bool,
    /// Linkify inline code spans that resolve; `None` renders exactly as before.
    link: Option<&'a CodeLinker<'a>>,
}

/// Render a whole markdown document to body HTML.
pub fn render(md: &str, rewrite_md_links: bool) -> String {
    render_linked(md, rewrite_md_links, None)
}

/// `render`, with inline code spans offered to `link` — a resolved span renders as
/// `<a href="…"><code>…</code></a>`. Fenced code is never offered (it's a program
/// listing, not a citation).
pub fn render_linked(md: &str, rewrite_md_links: bool, link: Option<&CodeLinker>) -> String {
    let ctx = Ctx {
        rewrite: rewrite_md_links,
        link,
    };
    render_lines(&md.lines().collect::<Vec<_>>(), &ctx)
}

/// The first `# heading`'s text, for the page `<title>`.
pub fn title_of(md: &str) -> Option<String> {
    md.lines()
        .find(|l| l.starts_with("# "))
        .map(|l| l[2..].trim().to_string())
}

fn render_lines(lines: &[&str], ctx: &Ctx) -> String {
    let mut out = String::new();
    let mut para: Vec<&str> = Vec::new();
    let mut items: Vec<(bool, String)> = Vec::new(); // (ordered, text)
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_start();

        // Fenced code: swallow to the closing fence.
        if let Some(info) = trimmed.strip_prefix("```") {
            flush_para(&mut out, &mut para, ctx);
            flush_items(&mut out, &mut items, ctx);
            let info = info.trim().to_string();
            let mut body: Vec<&str> = Vec::new();
            i += 1;
            while i < lines.len() && !lines[i].trim_start().starts_with("```") {
                body.push(lines[i]);
                i += 1;
            }
            i += 1; // past the closing fence (or EOF)
            push_fence(&mut out, &info, &body.join("\n"));
            continue;
        }

        // Blockquote: strip the markers and render the inside recursively.
        if trimmed.starts_with('>') {
            flush_para(&mut out, &mut para, ctx);
            flush_items(&mut out, &mut items, ctx);
            let mut inner: Vec<&str> = Vec::new();
            while i < lines.len() {
                let t = lines[i].trim_start();
                let Some(rest) = t.strip_prefix('>') else {
                    break;
                };
                inner.push(rest.strip_prefix(' ').unwrap_or(rest));
                i += 1;
            }
            let _ = write!(
                out,
                "<blockquote>\n{}</blockquote>\n",
                render_lines(&inner, ctx)
            );
            continue;
        }

        // Table: consecutive `|` lines; the second is the header separator.
        if trimmed.starts_with('|') {
            flush_para(&mut out, &mut para, ctx);
            flush_items(&mut out, &mut items, ctx);
            let mut rows: Vec<&str> = Vec::new();
            while i < lines.len() && lines[i].trim_start().starts_with('|') {
                rows.push(lines[i].trim());
                i += 1;
            }
            push_table(&mut out, &rows, ctx);
            continue;
        }

        if let Some(rest) = line.strip_prefix('#') {
            flush_para(&mut out, &mut para, ctx);
            flush_items(&mut out, &mut items, ctx);
            let level = 1 + rest.chars().take_while(|&c| c == '#').count().min(4);
            let text = rest.trim_start_matches('#').trim();
            let _ = writeln!(
                out,
                "<h{level} id=\"{}\">{}</h{level}>",
                slug(text),
                inline(text, ctx)
            );
        } else if trimmed.chars().all(|c| c == '-') && trimmed.len() >= 3 {
            flush_para(&mut out, &mut para, ctx);
            flush_items(&mut out, &mut items, ctx);
            out.push_str("<hr>\n");
        } else if let Some(item) = trimmed.strip_prefix("- ") {
            flush_para(&mut out, &mut para, ctx);
            items.push((false, item.to_string()));
        } else if let Some(item) = ordered_item(trimmed) {
            flush_para(&mut out, &mut para, ctx);
            items.push((true, item.to_string()));
        } else if line.trim().is_empty() {
            flush_para(&mut out, &mut para, ctx);
            flush_items(&mut out, &mut items, ctx);
        } else if !items.is_empty() {
            // A wrapped continuation of the previous list item.
            let last = items.len() - 1;
            items[last].1.push(' ');
            items[last].1.push_str(line.trim());
        } else {
            para.push(line);
        }
        i += 1;
    }
    flush_para(&mut out, &mut para, ctx);
    flush_items(&mut out, &mut items, ctx);
    out
}

fn flush_para(out: &mut String, para: &mut Vec<&str>, ctx: &Ctx) {
    if !para.is_empty() {
        let _ = writeln!(out, "<p>{}</p>", inline(&para.join(" "), ctx));
        para.clear();
    }
}

fn flush_items(out: &mut String, items: &mut Vec<(bool, String)>, ctx: &Ctx) {
    if items.is_empty() {
        return;
    }
    // One list per run; its kind is the first item's (the corpus never mixes).
    let tag = if items[0].0 { "ol" } else { "ul" };
    let _ = writeln!(out, "<{tag}>");
    for (_, item) in items.iter() {
        let _ = writeln!(out, "<li>{}</li>", inline(item, ctx));
    }
    let _ = writeln!(out, "</{tag}>");
    items.clear();
}

/// `1. text` → `text` (any number, one dot, one space).
fn ordered_item(line: &str) -> Option<&str> {
    let dot = line.find(". ")?;
    line[..dot]
        .chars()
        .all(|c| c.is_ascii_digit())
        .then(|| &line[dot + 2..])
        .filter(|_| dot > 0)
}

fn push_fence(out: &mut String, info: &str, body: &str) {
    // `quoin` AND `quoin norun` highlight — norun means don't EXECUTE (that's
    // --check's business), not don't highlight.
    if info == "quoin" || info.starts_with("quoin ") {
        let _ = writeln!(out, "{}", crate::highlighter::highlight_to_html(body));
    } else {
        let _ = writeln!(out, "<pre>{}</pre>", esc(body));
    }
}

fn push_table(out: &mut String, rows: &[&str], ctx: &Ctx) {
    // A `|` inside a backtick code span (block params: `{ |n| … }`) or escaped
    // as `\|` is content, not a cell separator — matching GitHub, which also
    // unescapes `\|`.
    let cells = |row: &str| -> Vec<String> {
        let mut cells = vec![String::new()];
        let mut in_code = false;
        let mut chars = row.trim_matches('|').chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '\\' if chars.peek() == Some(&'|') => {
                    chars.next();
                    cells.last_mut().unwrap().push('|');
                }
                '`' => {
                    in_code = !in_code;
                    cells.last_mut().unwrap().push(c);
                }
                '|' if !in_code => cells.push(String::new()),
                _ => cells.last_mut().unwrap().push(c),
            }
        }
        cells.iter().map(|c| c.trim().to_string()).collect()
    };
    let is_separator = |row: &str| row.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '));
    out.push_str("<table>\n");
    for (idx, row) in rows.iter().enumerate() {
        if idx == 1 && is_separator(row) {
            continue;
        }
        let tag = if idx == 0 && rows.len() > 1 && is_separator(rows[1]) {
            "th"
        } else {
            "td"
        };
        out.push_str("<tr>");
        for cell in cells(row) {
            let _ = write!(out, "<{tag}>{}</{tag}>", inline(&cell, ctx));
        }
        out.push_str("</tr>\n");
    }
    out.push_str("</table>\n");
}

/// Heading text → an anchor id: lowercased, alphanumerics kept, runs of anything
/// else collapsed to one `-`, trimmed.
fn slug(text: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in text.chars() {
        if c.is_alphanumeric() {
            if dash && !out.is_empty() {
                out.push('-');
            }
            dash = false;
            out.extend(c.to_lowercase());
        } else {
            dash = true;
        }
    }
    out
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The inline layer. Backtick spans are carved out FIRST and protected — the
/// emphasis and link passes only see the prose between them (a `*` inside
/// `` `lib/*.qn` `` is code, not emphasis).
fn inline(text: &str, ctx: &Ctx) -> String {
    let escaped = esc(text);
    let parts: Vec<&str> = escaped.split('`').collect();
    // An even part count means an odd number of backticks: the tail one is literal.
    let unpaired_tail = parts.len().is_multiple_of(2);
    // Swap code spans for control-char placeholders before the prose layer, so a
    // [label](url) whose label is (or contains) a code span still parses as one
    // link; markdown source can't contain the \u{1} delimiter.
    let mut protected = String::new();
    // (plain <code>, linked form) — which one lands is decided at substitution.
    let mut spans: Vec<(String, String)> = Vec::new();
    for (i, part) in parts.iter().enumerate() {
        if i % 2 == 1 && !(unpaired_tail && i == parts.len() - 1) {
            let _ = write!(protected, "\u{1}{}\u{1}", spans.len());
            let code = format!("<code>{part}</code>");
            let linked = match ctx.link.and_then(|f| f(part)) {
                Some(href) => format!(r#"<a href="{href}">{code}</a>"#),
                None => code.clone(),
            };
            spans.push((code, linked));
        } else {
            if i % 2 == 1 {
                protected.push('`');
            }
            protected.push_str(part);
        }
    }
    let mut out = prose(&protected, ctx.rewrite);
    for (n, (code, linked)) in spans.iter().enumerate() {
        let key = format!("\u{1}{n}\u{1}");
        // A span that landed inside a [label](url) anchor stays plain code —
        // nesting the resolver's <a> inside the label's would be invalid HTML.
        let inside_a = out.find(&key).is_some_and(|pos| {
            match (out[..pos].rfind("<a "), out[..pos].rfind("</a>")) {
                (Some(open), close) => close.is_none_or(|c| c < open),
                (None, _) => false,
            }
        });
        out = out.replace(&key, if inside_a { code } else { linked });
    }
    out
}

/// Links, then bold, then italics — on escaped non-code prose.
fn prose(text: &str, rewrite: bool) -> String {
    let s = links(text, rewrite);
    let s = pairs(&s, "**", "strong");
    pairs(&s, "*", "em")
}

fn links(text: &str, rewrite: bool) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        let Some(mid) = rest[open..].find("](") else {
            break;
        };
        let mid = open + mid;
        let Some(end) = rest[mid + 2..].find(')') else {
            break;
        };
        let end = mid + 2 + end;
        let label = &rest[open + 1..mid];
        let url = rewrite_href(&rest[mid + 2..end], rewrite);
        out.push_str(&rest[..open]);
        let _ = write!(out, r#"<a href="{url}">{label}</a>"#);
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

/// `x.md` → `x.html`, `README.md` → `index.html` (fragments preserved) — for
/// RELATIVE links only; absolute URLs pass through.
fn rewrite_href(href: &str, rewrite: bool) -> String {
    if !rewrite || href.contains("://") {
        return href.to_string();
    }
    let (path, fragment) = match href.split_once('#') {
        Some((p, f)) => (p, Some(f)),
        None => (href, None),
    };
    let Some(stem) = path.strip_suffix(".md") else {
        return href.to_string();
    };
    let mapped = if stem == "README" || stem.ends_with("/README") {
        format!("{}index.html", &stem[..stem.len() - "README".len()])
    } else {
        format!("{stem}.html")
    };
    match fragment {
        Some(f) => format!("{mapped}#{f}"),
        None => mapped,
    }
}

/// Replace `marker`-delimited pairs with `<tag>…</tag>`; an unpaired marker stays
/// literal. Emphasis never opens before whitespace ends or closes after it starts.
fn pairs(text: &str, marker: &str, tag: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(a) = rest.find(marker) {
        let after = &rest[a + marker.len()..];
        let Some(b) = after.find(marker) else {
            break;
        };
        let body = &after[..b];
        // A span that starts or ends with whitespace (or is empty) is not emphasis.
        if body.is_empty()
            || body.starts_with(char::is_whitespace)
            || body.ends_with(char::is_whitespace)
        {
            out.push_str(&rest[..a + marker.len()]);
            rest = after;
            continue;
        }
        out.push_str(&rest[..a]);
        let _ = write!(out, "<{tag}>{body}</{tag}>");
        rest = &after[b + marker.len()..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
#[path = "md_html_tests.rs"]
mod md_html_tests;
