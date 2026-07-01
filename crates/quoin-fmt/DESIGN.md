# `quoin-fmt` — the Quoin source formatter

An opinionated, zero-config formatter for Quoin (`.qn`) source: `gofmt`/`prettier`
philosophy, one canonical style, no options. Delivered as a library crate so the `qn`
binary (and, later, an editor/LSP) can call into it.

## 1. Goals

- **Canonical layout.** Normalize indentation, spacing, blank lines, and — the ambitious
  part — *re-decide line breaks* to fit a width budget. Two runs on any input converge.
- **Never change meaning.** `parse(src)` and `parse(format(src))` are the same AST.
- **Never lose a comment.** Every comment survives, byte-for-byte (trailing whitespace aside).
- **Zero configuration.** The style is baked in; there is nothing to tune.

## 2. What the syntax layer gives us (and doesn't)

The parser (`quoin-syntax`) is **pest**, which is *scannerless* — there is no token stream
to reuse, and `WHITESPACE`/`COMMENT` are **silent** grammar rules, so **the AST contains no
trivia**. A naive AST pretty-printer would delete every comment.

What we do get:

- **Precise spans.** Every `Node` carries `SourceInfo { start, end, line, column,
  source_text }` — byte offsets *and* the exact original slice. Leaves (numbers, strings,
  regexes) normalize their `value`, so verbatim spelling survives **only** via `source_text`.
- **An AST-equality primitive for free.** `Node`'s `PartialEq` ignores position once
  `clear_source_info()` (already in `quoin-syntax`) strips it; `IdentifierNode`/`NamespaceNode`
  exclude it from equality outright. That makes the "meaning unchanged" invariant a one-liner.

Two span gotchas the formatter must handle (both learned the hard way, both regression-tested):

- **Leading `(` is outside the span.** A parenthesized expression's node starts at the inner
  token, so `(x).m` reports its start *after* the `(`. We extend each statement start left over
  leading `(` + whitespace (`statement_content_start`).
- **Statement spans run to the next statement.** A top-level statement's `end` swallows the
  trailing whitespace, the `;` separator, and trailing comments up to the next statement. We
  re-derive the real end by trimming that trivia backwards (`statement_content_end`).
- **BOM.** The parser strips a leading U+FEFF before computing offsets, so we strip it too, or
  every span is off by 3 bytes.

## 3. Architecture

```
source ──▶ parse (quoin-syntax) ──▶ AST
       └─▶ scan_comments ──────────▶ [Comment]   (byte-ranged, re-attached by position)
                                        │
                          lower AST + comments ──▶ Doc  (Wadler/Leijen algebra)
                                                     │
                                              render(width) ──▶ String
```

- **`comments`** — a small state machine (mirroring `quoin_syntax::complete`) that recovers
  both comment forms (line `"* …`, block `" … "`) from the raw source, skipping string/regex
  contexts so a `"` inside `'…'` or `#/…/` is never mistaken for a comment.
- **`doc`** — the layout engine: `Text / Verbatim / Line / SoftLine / HardLine / Concat /
  Nest / Align / Group`. `Group` renders flat when it fits the width and broken otherwise;
  `Align` pins breaks to the current column (needed for the keyword-continuation style, §5).
- **`format`** — lowers the AST to a `Doc`, interleaving comments recovered from the gaps
  between node spans.
- **`verify`** — the guardrails (§6), shared by unit and corpus tests.

### Comment attachment

Comments are re-attached by byte position. For each gap between statements we split comments
into **trailing** (same line as the previous statement, before any newline) and **leading**
(their own line, hugging the next statement); comments *inside* a statement's span ride along
in its verbatim slice and are never re-emitted.

### Verbatim slices never get re-indented

A Quoin string literal may contain a literal newline, so shifting a block's lines to re-indent
it would corrupt strings. Re-indentation therefore happens **only structurally**, via the doc
engine's `Nest`, once a node is genuinely lowered — never by munging a verbatim slice. This is
why deeper lowering (P1+) replaces verbatim slices with real `Doc` trees rather than shifting text.

## 4. Phasing

The end state is the full width-driven pretty-printer; we get there in safe increments, with
the §6 guardrails green at every step:

- **P0 (this commit).** Canonical *top-level* layout: one statement per line, explicit `;`
  between statements (none after the last), one blank line between definitions when the source
  had one, comments re-attached. Each statement **body** is emitted verbatim. Proves the
  pipeline + guardrails over the whole corpus without touching statement internals.
- **P1.** Recurse into class/method/block bodies: canonical indentation and spacing, breaks
  kept as authored. Verbatim shrinks to expression leaves.
- **P2.** Width-driven `Group`s: break long keyword sends, list/map literals, and chains to fit.
- **P3.** `qn fmt` CLI — in place by default, `--dry-run` (stdout), `--diff` (unified diff via the
  system `diff`), `--check` (CI gate, exit 1 if unformatted); directories recurse. Then `qn fmt qnlib/`.

## 5. Canonical style

| Rule | Decision |
| --- | --- |
| Indent | 4 spaces |
| Wrap column | 100 |
| Binary operators | spaced: `a + b`, `a <= b` |
| Delimiter interior | padded when non-empty (`#( 1 2 3 )`, `{ |x| … }`), tight when empty (`#()`, `{}`) |
| `;` | between statements; omitted after the last in a block/program |
| Blank lines | at most one between definitions; runs collapse to one |
| Doc comments | `"* …` kept on their own line directly above the definition they lead |
| Leaves | verbatim spelling preserved (numbers, strings, regexes) |

**Multi-keyword sends** (`if:else:`, `case:when:do:`) that don't fit break with **each
continuation keyword on its own line at the statement's base column**; block bodies nest one
level (+4) and closing braces return to the base:

```
cond.if:{
    doThing;
}
else:{
    fallback;
}
```

Because breaks fall to the statement's base indent (not a column derived from the receiver), the
indent grows by a fixed step per nesting level instead of by the subject's width — deeply nested
conditionals (as in `qnlib/net/http.qn`) stay near the left margin rather than drifting into the
right edge.

Because the existing corpus uses a different (`+1` indent) convention, `qn fmt qnlib/` will
produce a large reflow diff once P2 lands — expected, and proven safe by the AST-equality guard.

## 6. Correctness guardrails (`verify`)

Three properties, enforced in unit tests and over the entire `qnlib/**` corpus (`tests/corpus.rs`):

1. **Semantics preserved** — `parse(src) == parse(format(src))` (positions cleared).
2. **Comments preserved** — the multiset of comment texts is unchanged (trailing-trimmed).
3. **Idempotent** — `format(format(src)) == format(src)`.

Properties 1 and 2 are also enforced **at runtime**: `format_source` re-parses its own output and
returns `FormatError::Verification` instead of the string if either is violated, so a bug can never
silently write meaning-changing output — e.g. a dropped `;` that would rebind a `.`-leading statement
onto the previous block (`foo -> {…}` then `.mix:X`). This is why `qn fmt --write` is safe by
construction, not just by testing. (Idempotence is a quality property, checked in tests only.)

Files that don't parse are skipped and counted; the formatter is not a linter.
