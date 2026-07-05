#!/usr/bin/env python3
"""Diff the vim Quoin syntax file against `qn highlight`, char by char.

Runs `qn highlight gallery.qn` and a headless vim over the same file, then
compares the per-character (fg color, bold) each produces. Deviations the
regex highlighter can't avoid (documented in ../README.md) are whitelisted;
anything else fails the run.

Usage: python3 compare.py [--qn PATH] [--gallery PATH] [-v]
"""

import argparse
import os
import re
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, "..", "..", ".."))

SGR_RE = re.compile(r"\x1b\[([0-9;]*)m")

# The identifier hues cycle by *binding* depth in the ANSI highlighter but by
# nesting depth of *use* in vim, so any cross-pair within the palette is an
# accepted deviation (closures referencing outer locals).
IDENT_HUES = {"#5fd7af", "#aeb1ab", "#c79ca9", "#85b9a5"}

ALLOWED = {
    # Comments: ANSI renders #b9bdba + faint; vim bakes the dim into the hex.
    ("#b9bdba~", "#7b7e7c"),
    # Named-block symbols `{ #gen |..| }`: the ANSI highlighter drops the span
    # (no source offsets) and shows plain; vim colors it like a symbol.
    ("-", "#4682b4"),
    # Class-header type params `Pair(T U)`: not visited by the ANSI
    # highlighter (shown plain); vim colors them like any capitalized name.
    ("-", "#ef65a5"),
    # `^> expr` in expression position (`var a = ^> (x + 1)`): the ANSI
    # highlighter only handles YieldReturn as a statement, so the operand is
    # left plain; vim colors it normally. (Fixable upstream by adding a
    # YieldReturn arm to highlight_expression.) The operand's identifiers can
    # carry any depth hue, hence the whole family.
    ("-", "#00bfff"),
    *(("-", hue) for hue in ("#5fd7af", "#aeb1ab", "#c79ca9", "#85b9a5")),
}


def parse_ansi(text):
    """ANSI stream -> rows of (char, attr) where attr is '#rrggbb[!][~]'."""
    rows, row = [], []
    fg, bold, faint = None, False, False
    pos = 0
    while pos < len(text):
        m = SGR_RE.match(text, pos)
        if m:
            codes = [c for c in m.group(1).split(";")]
            i = 0
            while i < len(codes):
                c = codes[i]
                if c in ("0", "00", ""):
                    fg, bold, faint = None, False, False
                elif c == "1":
                    bold = True
                elif c == "2":
                    faint = True
                elif c == "22":
                    bold = faint = False
                elif c == "39":
                    fg = None
                elif c == "38" and codes[i + 1 : i + 2] == ["2"]:
                    r, g, b = (int(x) for x in codes[i + 2 : i + 5])
                    fg = f"#{r:02x}{g:02x}{b:02x}"
                    i += 4
                i += 1
            pos = m.end()
            continue
        ch = text[pos]
        pos += 1
        if ch == "\n":
            rows.append(row)
            row = []
            continue
        attr = fg or "-"
        if attr == "#ffffff":  # HighlightType::None — vim leaves it Normal
            attr = "-"
        if bold:
            attr += "!"
        if faint:
            attr += "~"
        row.append((ch, attr))
    if row:
        rows.append(row)
    return rows


def vim_dump(gallery, syntax_dir):
    out = tempfile.NamedTemporaryFile(suffix=".tsv", delete=False)
    out.close()
    env = dict(os.environ, QUOIN_SYN_DUMP=out.name)
    cmd = [
        "vim", "-es", "--not-a-term", "-N", "-u", "NONE", "-i", "NONE",
        "--cmd", f"set rtp+={syntax_dir}",
        "-S", os.path.join(HERE, "dump_syntax.vim"),
        gallery,
    ]
    subprocess.run(cmd, env=env, check=False, capture_output=True, timeout=120)
    with open(out.name) as f:
        rows = [line.rstrip("\n").split("\t") if line.rstrip("\n") else []
                for line in f]
    os.unlink(out.name)
    return rows


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--qn", default=os.environ.get("QN", os.path.join(REPO, "target", "debug", "qn")))
    ap.add_argument("--gallery", default=os.path.join(HERE, "gallery.qn"))
    ap.add_argument("-v", "--verbose", action="store_true")
    args = ap.parse_args()

    ansi = subprocess.run([args.qn, "highlight", args.gallery],
                          capture_output=True, text=True, check=True).stdout
    ansi_rows = parse_ansi(ansi)

    with open(args.gallery) as f:
        source_lines = f.read().split("\n")
    stripped = ["".join(ch for ch, _ in row) for row in ansi_rows]
    if stripped != [l for l in source_lines][: len(stripped)]:
        sys.exit("FATAL: decolorized `qn highlight` output != gallery source")

    vim_rows = vim_dump(args.gallery, os.path.join(REPO, "editors", "vim"))

    groups = {}  # (ansi_attr, vim_attr) -> [(line, col, context)]
    total = compared = 0
    for l, row in enumerate(ansi_rows):
        vrow = vim_rows[l] if l < len(vim_rows) else []
        bc = 0  # vim columns are byte-indexed
        for c, (ch, a_attr) in enumerate(row):
            v_attr = vrow[bc] if bc < len(vrow) else "-"
            bc += len(ch.encode("utf-8"))
            total += 1
            if ch.isspace():
                continue
            compared += 1
            if a_attr == v_attr:
                continue
            ctx = source_lines[l][max(0, c - 12) : c + 12].strip()
            groups.setdefault((a_attr, v_attr), []).append((l + 1, c + 1, ctx))

    ok = True
    for (a, v), sites in sorted(groups.items(), key=lambda kv: -len(kv[1])):
        allowed = (a, v) in ALLOWED or (
            a.rstrip("!~") in IDENT_HUES and v.rstrip("!~") in IDENT_HUES
        )
        if not allowed:
            ok = False
        if allowed and not args.verbose:
            continue
        tag = "allowed " if allowed else "MISMATCH"
        print(f"{tag} ansi={a:<10} vim={v:<10} x{len(sites)}", file=sys.stderr)
        for line, col, ctx in sites[:3]:
            print(f"    {line}:{col}  ...{ctx}...", file=sys.stderr)

    n_allowed = sum(len(s) for k, s in groups.items()
                    if (k in ALLOWED) or (k[0].rstrip("!~") in IDENT_HUES and k[1].rstrip("!~") in IDENT_HUES))
    n_bad = sum(len(s) for s in groups.values()) - n_allowed
    print(f"{compared} chars compared: {compared - n_allowed - n_bad} exact, "
          f"{n_allowed} allowed deviations, {n_bad} mismatches")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
