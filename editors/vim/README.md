# Quoin syntax highlighting for Vim

Vim/Neovim syntax highlighting for `.qn` files whose colors match the VM's
ANSI highlighter (`qn highlight`, the REPL, error snippets). The palette is
copied from `colors_for()` in `crates/quoin-syntax/src/highlight.rs`.

## Install

Symlink this directory as a vim package (loads automatically, no config):

```sh
mkdir -p ~/.vim/pack/quoin/start
ln -s /path/to/quoin/editors/vim ~/.vim/pack/quoin/start/quoin
```

For Neovim use `~/.local/share/nvim/site/pack/quoin/start` instead, or add
`set runtimepath+=/path/to/quoin/editors/vim` to your vimrc.

Exact colors require `set termguicolors` (truecolor). Without it, nearest
xterm-256 approximations are used.

## Fidelity

`test/compare.py` diffs the vim highlighting against `qn highlight` char by
char — over `test/gallery.qn` (which exercises every highlight type) and any
other file:

```sh
cargo build                             # compare.py uses target/debug/qn
python3 editors/vim/test/compare.py     # gallery; -v shows allowed deviations
python3 editors/vim/test/compare.py --gallery qnlib/core/00-bootstrap.qn
```

As of writing, the gallery and every file under `qnlib/` show **zero**
mismatches outside the allowed deviations below.

### Known deviations (regex vs AST highlighting)

- **Identifier hues**: the ANSI highlighter cycles the 4 identifier colors by
  the scope depth where a name was first *bound* (a symbol-table walk); vim
  colors by the nesting depth of *use*. Identical whenever a variable is used
  at its binding depth; a closure referencing an outer local shows the inner
  depth's hue. Blocks nested 6+ deep wrap mod 5 instead of tracking true
  depth (brace colors stay exact; identifier hues can drift there).
- **Comments**: ANSI renders `#b9bdba` + terminal faint (SGR 2); vim has no
  faint attribute, so the dim is baked into the hex (`#7b7e7c`).
- **ANSI blind spots** (vim colors what ANSI accidentally leaves white):
  named-block symbols (`{ #gen |x| ... }`), class-header type params
  (`Pair(T U) <- ...`), and the operand of `^>` used as an expression
  (no `YieldReturn` arm in `highlight_expression`).

## Files

- `syntax/quoin.vim` — the highlighter. Block braces reproduce the ANSI
  5-color depth cycle exactly via depth-threaded nested regions;
  collections/parens pass depth through unchanged, headers carry the block's
  depth, guard blocks nest like body blocks.
- `ftdetect/quoin.vim` — `*.qn` → `filetype=quoin`.
- `ftplugin/quoin.vim` — `commentstring`, `?` as an identifier char,
  `suffixesadd=.qn`.
- `test/gallery.qn`, `test/dump_syntax.vim`, `test/compare.py` — the
  fidelity harness described above.
