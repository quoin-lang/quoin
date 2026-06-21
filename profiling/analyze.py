#!/usr/bin/env python3
"""Analyze a samply / Firefox "processed" profile for the qn VM.

Usage:
    profiling/analyze.py PROFILE.json.gz [BINARY] [-n TOP]
    profiling/analyze.py BEFORE.json.gz --compare AFTER.json.gz

Reports self-time by module, top self-time functions, and top inclusive
functions. qn frames are symbolicated with `atos` and demangled with `rustfilt`
(both used when present; the script degrades gracefully without them).

If BINARY is omitted it is inferred from the profile filename:
    .../before*.json.gz -> .../qn-before
    .../after*.json.gz  -> .../qn-after
    otherwise           -> target/profiling/qn

samply stores only addresses, so the BINARY the profile was recorded against is
required to symbolicate (see CLAUDE.md — keep qn-before / qn-after).
"""
import argparse
import collections
import gzip
import json
import os
import re
import shutil
import subprocess

DEFAULT_VMADDR = 0x100000000  # arm64 macOS executable __TEXT base
LIBTAG = {
    "libsystem_malloc.dylib": "[malloc]",
    "libsystem_platform.dylib": "[memmove/memset]",
    "libsystem_kernel.dylib": "[syscall]",
    "libsystem_c.dylib": "[libc]",
}
# Legacy ($LT$-style) encoding atos leaves in demangled _ZN symbols.
PUNY = [("$LT$", "<"), ("$GT$", ">"), ("$LP$", "("), ("$RP$", ")"),
        ("$C$", ","), ("$u20$", " "), ("$u7b$", "{"), ("$u7d$", "}"),
        ("$BP$", "*"), ("$RF$", "&"), ("..", "::")]


def load_thread(path):
    with gzip.open(path) as f:
        prof = json.load(f)
    t = prof["threads"][0]
    strings = t.get("stringArray") or t.get("stringTable")
    return prof, t, strings


def infer_binary(profile_path):
    d = os.path.dirname(os.path.abspath(profile_path))
    base = os.path.basename(profile_path)
    cand = None
    if base.startswith("before"):
        cand = os.path.join(d, "qn-before")
    elif base.startswith("after"):
        cand = os.path.join(d, "qn-after")
    if cand and os.path.exists(cand):
        return cand
    return "target/profiling/qn" if os.path.exists("target/profiling/qn") else None


def text_vmaddr(binary):
    """The binary's __TEXT vmaddr (samply relative addresses are offset from it)."""
    try:
        out = subprocess.run(["otool", "-l", binary], capture_output=True, text=True).stdout
        in_text = False
        for line in out.splitlines():
            s = line.strip()
            if s.startswith("segname __TEXT"):
                in_text = True
            elif in_text and s.startswith("vmaddr"):
                return int(s.split()[1], 16)
    except Exception:
        pass
    return DEFAULT_VMADDR


def atos_lines(binary, reladdrs):
    """{reladdr(int): raw atos line} for the given relative addresses."""
    if not binary or not os.path.exists(binary) or not shutil.which("atos") or not reladdrs:
        return {}
    base = text_vmaddr(binary)
    args = ["atos", "-o", binary, "-arch", "arm64", "-l", hex(base)]
    args += [hex(base + r) for r in reladdrs]
    out = subprocess.run(args, capture_output=True, text=True).stdout.splitlines()
    return {r: out[i] for i, r in enumerate(reladdrs) if i < len(out)}


def rustfilt(text):
    """Demangle Rust symbols embedded in `text` via rustfilt, if installed.

    rustfilt is a line-preserving text filter, so a newline-joined batch comes
    back with the same number of lines."""
    if not shutil.which("rustfilt"):
        return text
    return subprocess.run(["rustfilt"], input=text, capture_output=True, text=True).stdout


def clean(sym):
    """Strip the trailing hash, the `(in lib) + off` suffix, and legacy $LT$ encoding."""
    sym = re.sub(r"::h[0-9a-f]{16}", "", sym)
    for a, b in PUNY:
        sym = sym.replace(a, b)
    return re.sub(r"\s*\(in [^)]*\).*", "", sym).strip()


def compute(t, strings):
    fn_name = t["funcTable"]["name"]
    fn_res = t["funcTable"].get("resource", [None] * t["funcTable"]["length"])
    fr_func = t["frameTable"]["func"]
    st_prefix = t["stackTable"]["prefix"]
    st_frame = t["stackTable"]["frame"]
    smp = t["samples"]
    stacks = smp["stack"]
    weights = smp.get("weight") or [1] * len(stacks)
    res_name = t.get("resourceTable", {}).get("name")

    def libof(fx):
        r = fn_res[fx]
        if res_name and r is not None and 0 <= r < len(res_name):
            return strings[res_name[r]]
        return "?"

    self_t, incl_t, self_lib = collections.Counter(), collections.Counter(), collections.Counter()
    for si, w in zip(stacks, weights):
        if si is None:
            continue
        leaf = fr_func[st_frame[si]]
        self_t[leaf] += w
        self_lib[libof(leaf)] += w
        seen, cur = set(), si
        while cur is not None:
            seen.add(fr_func[st_frame[cur]])
            cur = st_prefix[cur]
        for fx in seen:
            incl_t[fx] += w
    return {
        "total": sum(weights), "n": len(stacks),
        "self_t": self_t, "incl_t": incl_t, "self_lib": self_lib,
        "libof": libof, "name": lambda fx: strings[fn_name[fx]],
    }


def symbol_map(c, binary, top):
    """{funcidx: pretty name} for every qn frame that may appear in the top lists.

    atos resolves `0x` address frames; already-named-but-mangled frames pass
    through unchanged; the whole batch is then demangled once via rustfilt."""
    name, libof = c["name"], c["libof"]
    cand = {fx for fx, _ in c["self_t"].most_common(top)}
    cand |= {fx for fx, _ in c["incl_t"].most_common(top * 5)}
    qn = [fx for fx in cand if libof(fx) == "qn"]
    reladdrs = sorted({int(name(fx), 16) for fx in qn if name(fx).startswith("0x")})
    atos = atos_lines(binary, reladdrs)
    raw = [atos.get(int(name(fx), 16), name(fx)) if name(fx).startswith("0x") else name(fx)
           for fx in qn]
    pretty = rustfilt("\n".join(raw)).splitlines()
    if len(pretty) != len(raw):
        pretty = raw
    return {fx: clean(pretty[i]) for i, fx in enumerate(qn)}


def analyze(profile_path, binary, top):
    prof, t, strings = load_thread(profile_path)
    c = compute(t, strings)
    total, name, libof = c["total"], c["name"], c["libof"]
    sym = symbol_map(c, binary, top)

    def label(fx):
        if libof(fx) == "qn":
            return sym.get(fx, clean(name(fx)))
        return f"{LIBTAG.get(libof(fx), '[' + libof(fx) + ']')} {name(fx)}"

    print(f"profile: {profile_path}")
    print(f"binary : {binary or '(none — unsymbolicated)'}")
    print(f"samples: {c['n']}  (interval {prof.get('meta', {}).get('interval', '?')} ms)\n")
    print("== self-time by module ==")
    for lib, w in c["self_lib"].most_common(8):
        print(f"  {100 * w / total:5.1f}%  {lib}")
    print(f"\n== top {top} self-time ==")
    for fx, w in c["self_t"].most_common(top):
        print(f"  {100 * w / total:5.1f}%  {label(fx)[:100]}")
    print(f"\n== top {top} inclusive (qn) ==")
    shown = 0
    for fx, w in c["incl_t"].most_common(top * 5):
        if libof(fx) != "qn":
            continue
        print(f"  {100 * w / total:5.1f}%  {label(fx)[:100]}")
        shown += 1
        if shown >= top:
            break


def compare(before_path, after_path):
    cs = [compute(*load_thread(p)[1:]) for p in (before_path, after_path)]

    def pct(c, lib):
        return 100 * c["self_lib"].get(lib, 0) / c["total"]

    libs = set(cs[0]["self_lib"]) | set(cs[1]["self_lib"])
    print(f"== self-time by module: BEFORE -> AFTER ==   (samples {cs[0]['n']} -> {cs[1]['n']})")
    for lib in sorted(libs, key=lambda l: -pct(cs[0], l)):
        b, a = pct(cs[0], lib), pct(cs[1], lib)
        print(f"  {b:5.1f}% -> {a:5.1f}%  ({a - b:+5.1f})  {lib}")
    print("\nNote: module-level deltas are reliable; function-level comparison across "
          "two builds is confounded by differing inlining.")


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("profile", help="PROFILE.json.gz (the BEFORE file with --compare)")
    ap.add_argument("binary", nargs="?", help="binary to symbolicate against (inferred if omitted)")
    ap.add_argument("-n", "--top", type=int, default=25, help="rows per section (default 25)")
    ap.add_argument("--compare", metavar="AFTER.json.gz", help="compare module self-time vs this profile")
    args = ap.parse_args()
    if args.compare:
        compare(args.profile, args.compare)
    else:
        analyze(args.profile, args.binary or infer_binary(args.profile), args.top)


if __name__ == "__main__":
    main()
