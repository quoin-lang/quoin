" Dumps per-character gui colors for the loaded buffer as TSV: one row per
" line, one cell per byte column, `<fg-hex>` or `-` (unhighlighted), with a
" trailing `!` when bold. Driven by compare.py:
"   vim -es --not-a-term -N -u NONE -i NONE \
"       --cmd 'set rtp+=editors/vim' -S dump_syntax.vim gallery.qn
set nocompatible
let s:out = empty($QUOIN_SYN_DUMP) ? 'syn_dump.tsv' : $QUOIN_SYN_DUMP
syntax enable
set filetype=quoin
let s:lines = []
for s:l in range(1, line('$'))
  let s:cells = []
  for s:c in range(1, col([s:l, '$']) - 1)
    let s:id = synIDtrans(synID(s:l, s:c, 1))
    let s:fg = synIDattr(s:id, 'fg#', 'gui')
    let s:bold = synIDattr(s:id, 'bold', 'gui') ==# '1'
    call add(s:cells, (empty(s:fg) ? '-' : s:fg) . (s:bold ? '!' : ''))
  endfor
  call add(s:lines, join(s:cells, "\t"))
endfor
call writefile(s:lines, s:out)
qall!
