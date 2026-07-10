# Reference documentation — comment docs, one pipeline, `qn doc`

*Status: PHASES 1 + 2 SHIPPED (2026-07-09; designed at `944d5d8`, built the same day).
`qn doc [PATH…] [--out DIR] [--json] [--coverage]` generates HTML + JSON for the stdlib plus
user units (extractor `src/docs.rs`, generator `src/runner_doc.rs`,
`.doc(..)`/`.class_doc(..)` on `NativeClassBuilder`). Phase 2: **extension sites** — a
statically-named reopen records its location (`RecordClassSite` → `class_meta.extensions`),
so the block above `Name <-- {…}` is reachable and stands in for a missing class doc
([IO]File answers with its one-shot-writes commentary); **`doc` / `docFor:` on Object** —
lazy, following `can?:`'s receiver convention (a Class answers for instance methods,
`.meta` for class-side), plus `$doc Name[.selector]` in the REPL; **`qn highlight --html`**
— a second formatter over the same `HighlightSpan`s, whose dark-scheme CSS is *generated
from* the ANSI `colors_for` table (light-scheme colors are hand-picked: the terminal palette
assumes a dark background), and whose classes + stylesheet the doc generator's fenced
examples inline — one table, two consumers. Tests: `tests/doc_gen.rs`,
`qnlib/tests/62-docs.qn`, `src/docs_tests.rs`. One §4 note: extraction is a line-based scan
rather than `scan_comments` (a doc line is a whole-line `"*` comment and strings cannot span
lines — recorded in `src/docs.rs`). Remaining: phase 3 (doctests), extension-manifest docs.*

*The three forks are settled: comment docs over a `.doc:` authoring API, plain adjacency over
a new doc sigil, HTML + JSON output. Companion to `docs/INTROSPECTION.md` (the read-only
surface this rides on). The language *reference book* (`docs/language/`, RELEASE_PREP Tier 2)
is a separate, hand-written artifact; this system generates the per-class API reference that
book links into.*

## 1. Why

Quoin has no generated API reference. The shipped surface is documented three different ways —
`"*` comment blocks in `qnlib/`, `//` comments above `NativeClassBuilder` closures in
`src/runtime/`, and nothing at all for extension-provided classes — and none of them reaches a
user. The REPL's `$class` shows selectors but cannot say what any of them does.

The design constraint that shapes everything: **Quoin classes, native classes, mixins, and
extension classes must document through one pipeline.** The wrong way is to parse two languages
(Quoin source and Rust source) and merge the results. The right way is already in the tree: the
VM's class table unifies all of them, and `src/introspect.rs` (`describe_class`, `globals`)
already walks it returning plain owned structs with selectors, variants, types, seal/abstract
flags, and — the load-bearing part — a `SourceLoc` per Quoin-defined method.

## 2. Grounding: the three facts the design leans on

1. **Comment recovery exists.** The parser drops comments (pest trivia), but `quoin-fmt` could
   not survive that either, so `crates/quoin-fmt` has `scan_comments`: a state machine mirroring
   `quoin_syntax::complete` that recovers both comment forms (`"* …` line, `" … "` block) from
   raw source, byte-ranged, skipping string/regex contexts so a `"` inside `'…'` or `#/…/` is
   never mistaken for a comment. Battle-tested by the formatter's "never lose a comment,
   byte-for-byte" guarantee. Doc extraction reuses it rather than re-lexing.
2. **Every Quoin method knows where it lives.** `MethodVariant.source: Option<SourceLoc>`
   (`src/introspect.rs`) carries file/line/column.
3. **The installed binary carries its stdlib source.** `src/stdlib.rs::resolve(path)` returns
   the embedded source text, so doc extraction works outside a checkout — `$doc` in a user's
   REPL can answer for `[IO]Stdin.readLine` with no source tree on disk.

Together: *attach a doc to a method* = take its `SourceLoc`, fetch the source (embedded or
disk), lift the contiguous `"*` block immediately above it. No grammar change, no new syntax,
no runtime cost until someone asks.

## 3. Decided: comment docs, not a `.doc:` authoring API

The alternative was runtime attachment — multiline strings in the grammar plus
`.doc:'…'` sends inside class bodies. Rejected, for reasons in descending weight:

- **The corpus already exists.** `qnlib/` holds hundreds of `"*` blocks sitting directly above
  the members they describe; under comment docs that corpus *is* the documentation on day one.
  Under `.doc:` every block gets rewritten into a string literal, and forever after a method's
  description has two possible homes.
- **`.doc:` executes.** Docs become order-dependent statements; every program allocates every
  docstring at startup; the embedded stdlib grows hundreds of sends that run before `main`.
- **It needs multiline strings first** — a real grammar feature with its own design questions
  (escaping, indentation stripping), existing *for* docs. Comment blocks already span lines.
  Multiline strings may still happen; they are decoupled from this design on purpose.
- **Per-method attachment is awkward as a message** — "applies to the previous definition" is
  action-at-a-distance. In a comment convention, adjacency is the semantics.

Costs accepted with the decision: docs are not first-class runtime values (the query API in §6
covers every observed need), and adjacency is a convention a blank line can silently break
(mitigated by `qn doc --coverage`, §7).

## 4. Decided: attachment is plain adjacency

No new sigil (`"**` was considered). The rules:

- A contiguous run of `"*` lines **immediately** above a definition — no blank line between —
  is that definition's doc. This applies to methods (`sel -> { … }`, `sel: -> { … }`, either
  side of `.meta`), class definitions (`Name <- { … }`), and class extensions
  (`Name <-- { … }`).
- A blank line detaches: the block becomes file/section commentary and attaches to nothing.
  Authors keep an implementation note out of the docs by spacing it off the definition.
- The extracted text strips the leading `"*` and at most one following space per line.
- **First line is the summary** (shown in selector lists and `$class`); the rest is the body;
  fenced code blocks are examples (and the future doctest input, §9).
- A class *extension*'s doc block documents the extension site, not the class; the generator
  shows the class doc from the definition site and lists extension docs beneath it.

Implementation note: `ClassInfo` has no `source` field today — methods do, classes don't. The
class-definition site is known at `DefineClass` time; thread it through so class docs extract
the same way method docs do, rather than scanning for `Name <-` textually.

## 5. Native classes: `.doc()` on the builder

`NativeClassBuilder.returns(...)` already establishes the pattern — a post-hoc modifier on the
last-registered method via `last_side`. Docs get the same shape:

```rust
.instance_method("readLine", |vm, mc, r, _| { ... })
.returns("String?")
.doc("The next line, without its terminator; nil at end of input.")
```

plus `.class_doc("…")` for the class itself. Stored beside `ret_type` in the builder's method
metadata, surfaced through `describe_class` like everything else. The existing `//` comments
above the closures migrate into `.doc()` strings mechanically over time; many are already
docstring-quality. Parsing Rust source (`syn` / rustdoc JSON) to harvest them automatically is
rejected: heavy, brittle, and impossible from an installed binary.

**Extensions** get the pipeline for free once `ClassDecl` (the manifest an extension returns at
spawn) gains optional per-class and per-selector doc strings — this is deferred decision #7
from `docs/EXT_PACKAGING.md`, and this section is its design: manifest → `install_ext_class` →
the same class metadata → `describe_class`. Ship it when a bundled extension wants docs.

## 6. Runtime query API — lazy, read-only

`.doc:` survives as the *reader*:

- `Point.doc` / `Point.docFor:#x` / `Point.meta.docFor:#new:` — and a `$doc` REPL command.
- Resolution is **lazy**: introspection gives the `SourceLoc`, the source comes from
  `stdlib::resolve` (embedded) or disk, `scan_comments` + the §4 adjacency rules lift the
  block. Cache per (file), not per (query). Zero startup cost; nothing retained in programs
  that never ask.
- Classes defined in `-e` strings or REPL lines have no file and answer `nil`. Native methods
  answer from builder metadata; no file involved.

## 7. The generator: `qn doc`

A CLI verb in the binary, like `fmt` and `check` — not an external tool.

```
qn doc [PATH…] [--out DIR] [--json] [--coverage]
```

- Boots a VM exactly as `qn -e` does (embedded stdlib); `PATH…` are optional additional units
  to load first (`use`d), so a user documents their own package with the same command.
- Walks `introspect::globals()` → `describe_class` per class. Docs from §4 (Quoin) and §5
  (native/extension). The class table is the source of truth; nothing re-parses source except
  the comment lift.
- **HTML**: one page per class + a namespace-grouped index (`[IO]`, `[OS]`, `[Web]`, `[HTTP]`,
  core). Self-contained — one inline stylesheet, no JS dependencies. Signatures cross-link:
  `param_types` / `ret_type` are class names, so `^String` links to `String.html`. Source
  links from `SourceLoc`. In-page anchors are `#i-<selector>` / `#c-<selector>` (instance vs
  class side — `read` can exist on both); `:`, `?` and `!` are all legal in URI fragments, so
  selectors embed verbatim (`#i-at:put:`).
- **JSON** (`--json`): the doc model serialized, `{"version": 1, …}`, one file per run. This is
  the contract for other renderers — LSP hover, a future website — so the HTML renderer is a
  consumer of the model, not the model.
- **`--coverage`**: list public classes/selectors with no doc, exit non-zero over a threshold.
  This is the mitigation for silent adjacency-detachment (§3) and how CI keeps docs from
  rotting once they exist.

## 8. `qn highlight --html` — shared code styles (decided, next pass)

`qn highlight` renders ANSI today, but the architecture already splits model from renderer:
`quoin_syntax::highlight` produces byte-ranged, typed `HighlightSpan`s (and `HighlightType`
already includes `Comment`), and `src/highlighter.rs::format_ansi` is merely one formatter
over them. The feature is a second formatter, not a new highlighter:

- `format_html(source, spans) -> String` emitting `<span class="qn-<type>">…</span>` with
  HTML-escaped text, plus a `stylesheet() -> &'static str` mapping each `HighlightType` to a
  class with its color — the single place code style lives.
- `qn highlight --html FILE` wraps that in a minimal standalone page.
- **The doc generator uses the same two functions** for fenced examples in doc bodies and for
  signature rendering, and inlines the same `stylesheet()`. "Docs and `qn highlight` share
  code styles" is then true by construction — one mapping, two consumers — not by keeping two
  stylesheets in sync.
- The ANSI palette (`colors_for`) and the CSS mapping should be derived from one table per
  `HighlightType` so terminal and web renderings agree in spirit, even though the color spaces
  differ.

## 9. Phases

- **Phase 1 — extraction + generator.** The §4 extractor (reusing `scan_comments`),
  `ClassInfo.source`, `.doc()`/`.class_doc()` on the builder, `qn doc` with HTML + JSON +
  `--coverage`. The stdlib ships documented on day one by virtue of the existing corpus.
- **Phase 2 — surfaces.** `docFor:` / `$doc` (§6), `qn highlight --html` + shared stylesheet
  (§8), doc rendering inside REPL completion where cheap.
- **Phase 3 — doctests.** Extract fenced examples from doc bodies and run them under `qn`;
  folds into the RELEASE_PREP Tier 2 "doc-example harness" item, which wants the same
  machinery for `docs/language/`.
- **Deferred, explicitly:** extension-manifest docs (designed in §5, built when wanted),
  markdown richness beyond paragraphs + fenced code, multiline strings (independent feature,
  decide on its own merits).
