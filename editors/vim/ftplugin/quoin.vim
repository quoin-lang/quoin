" Vim filetype plugin for Quoin (.qn)

if exists("b:did_ftplugin")
  finish
endif
let b:did_ftplugin = 1

" `"*` starts a line comment.
setlocal commentstring=\"*\ %s
" `?` is an identifier character (foo?, defined?).
setlocal iskeyword=@,48-57,_,63
" `use io/file` resolves to io/file.qn.
setlocal suffixesadd=.qn

let b:undo_ftplugin = "setlocal commentstring< iskeyword< suffixesadd<"
