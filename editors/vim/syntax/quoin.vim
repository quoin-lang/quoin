" Vim syntax file
" Language: Quoin (.qn)
"
" The palette mirrors the VM's ANSI highlighter — `colors_for()` in
" crates/quoin-syntax/src/highlight.rs, as rendered by `qn highlight`.
" Exact colors require 'termguicolors'; the ctermfg values are nearest
" xterm-256 approximations. A regex highlighter can only approximate the
" AST-driven one in a few places; see editors/vim/README.md for the list.
"
" Depth model: the ANSI highlighter cycles block-brace colors by block
" nesting depth (mod 5) and identifier colors by the depth a name was first
" bound (mod 4). Braces are reproduced exactly via five levels of nested
" regions; identifiers use the nesting depth of *use* as a stand-in for
" binding depth. Collections and parens pass the depth through unchanged,
" as in the ANSI highlighter.

if exists("b:current_syntax")
  finish
endif

" `?` is an identifier character in Quoin (foo?, defined?).
syntax iskeyword @,48-57,_,63

syn sync fromstart

" --- identifiers (lowest priority; one item per depth level) -----------------
for s:k in range(1, 5)
  execute 'syn match quoinIdent' . s:k . ' /\<[a-z_]\k*\>/' . (s:k == 1 ? '' : ' contained')
  " A splat-ignored lvalue `*_` is a single identifier span in the AST.
  execute 'syn match quoinIdent' . s:k . ' /\*_\>/' . (s:k == 1 ? '' : ' contained')
endfor

" --- reserved identifiers and capitalized globals ----------------------------
" true/false/nil color as globals — but a selector named `true:` is still a
" selector, so these are matches (overridden by the selector items below),
" not :syn keywords (which would always win).
syn match quoinGlobal /\<\%(true\|false\|nil\)\>/
syn match quoinGlobal /\<\u\k*\>/

syn match quoinInstanceIdent /@\h\k*/
syn match quoinNamespace /\[\%(\/\|\h\k*\%(\/\h\k*\)*\/\=\)\]/

" --- literals -----------------------------------------------------------------
syn match quoinNumber /\<\d\+\%(\.\d\+\)\=/
" A leading-dot float (.5) — but not the tail of a `..` range.
syn match quoinNumber /\.\@1<!\.\d\+/

" Error statements: !!! / ... / ???
syn match quoinError /!!!\|???\|\.\.\./

" Comments: `"* to end of line` and `"..."` (multiline, \" escapable).
syn region quoinComment start=/"\*\@!/ skip=/\\"/ end=/"/
syn match quoinComment /"\*.*$/

syn region quoinString start=/'/ skip=/\\\\\|\\'/ end=/'/
syn match quoinSymbol /#\h[0-9A-Za-z_?:]*/
" Quoted symbol #'...': color the # like the string that follows it.
syn match quoinSymbol /#\ze'/
syn region quoinRegex start=/#\// skip=/\\./ end=/\//

" --- method selectors ----------------------------------------------------------
" A keyword-message selector `foo:` (or variadic `foo+:`; the `+` stays plain)
" — except a typed declaration `var x: T`.
syn match quoinSelector /\%(\<\%(var\|let\)\s\+\)\@20<!\<\h\k*\ze+\=:/
" A method-definition selector before -> or --> (a `boom!` bang stays plain,
" as in the ANSI highlighter: the span covers only the identifier).
syn match quoinSelector /\<\h\k*\ze!\=\s*--\=>/
" A unary selector after `.` (not after a `..` range).
syn match quoinSelector /\%(\.\.\)\@2<!\.\@1<=\h\k*/
" A symbol selector before -> or --> (operator methods: #'+' -> { ... }).
syn match quoinSelector /#\%(\h[0-9A-Za-z_?:]*\|'\%(\\.\|[^'\\]\)*'\)\ze\s*--\=>/

" --- statement keywords (soft: only when followed by a plausible target) -------
syn match quoinKeyword /\<\%(var\|let\)\>\ze\s\+[[:alpha:]_@(*]/
syn match quoinKeyword /\<use\>\ze\s\+[[:alpha:]_*]/ nextgroup=quoinUsePath,quoinUsePkg skipwhite
syn match quoinUsePath /\*\|\h\k*\%(\/\h\k*\)*\%(\/\*\)\=/ contained
" Defined after quoinUsePath so `pkg:` wins where both match.
syn match quoinUsePkg /\h\k*:/ contained nextgroup=quoinUsePath

" --- containers (depth-threaded) ------------------------------------------------
" A bare-identifier dict key `#{ a: 1 }` is an identifier, not a selector.
syn match quoinDictKeyGlobal /\<\u\k*\ze:/ contained
" An ignored block arg `|_|` has no identifier node in the AST — stays plain.
syn match quoinHeaderIgnored /\<_\>/ contained

for s:k in range(1, 5)
  let s:n = s:k == 5 ? 1 : s:k + 1
  let s:c = s:k == 1 ? '' : ' contained'

  " Block braces increment the depth; everything else passes it through.
  execute 'syn region quoinBlock' . s:k . ' matchgroup=quoinBlockBrace' . s:k
        \ . ' start=/{/ end=/}/ contains=@quoinCtx' . s:n . s:c
  " The `|args ^Ret - decls|` header, anchored to the opening `{` (optionally
  " through a block name symbol) so `a || b` never starts one. No selector
  " items inside: `x: Type` header args are identifiers. Guard blocks
  " `|x { x > 0 }|` nest like body blocks (same depth rule as the ANSI
  " highlighter's decl_block handling).
  execute 'syn region quoinHeader' . s:k
        \ . ' start=/\%({\_s*\%(#\k\+\_s\{-}\)\=\)\@60<=|/ end=/|/ end=/\ze}/'
        \ . ' contained contains=quoinIdent' . s:k . ',quoinHeaderIgnored,quoinInstanceIdent,quoinGlobal,quoinNamespace,quoinComment,quoinBlock' . s:k
  execute 'syn region quoinList' . s:k . ' matchgroup=quoinCollectionBrace'
        \ . ' start=/#(/ end=/)/ contains=@quoinCtx' . s:k . s:c
  execute 'syn region quoinSet' . s:k . ' matchgroup=quoinCollectionBrace'
        \ . ' start=/#</ end=/>/ contains=@quoinCtx' . s:k . s:c
  execute 'syn region quoinDict' . s:k . ' matchgroup=quoinCollectionBrace'
        \ . ' start=/#{/ end=/}/ contains=@quoinCtx' . s:k . ',quoinDictKeyGlobal,quoinDictKey' . s:k . s:c
  execute 'syn region quoinParen' . s:k
        \ . ' start=/(/ end=/)/ contains=@quoinCtx' . s:k . s:c
  execute 'syn match quoinDictKey' . s:k . ' /\<[a-z_]\k*\ze:/ contained'

  " User string #Name'...' — # colored like the string, name like an identifier.
  execute 'syn match quoinUserStringHash' . s:k . " /#\\ze\\h\\k*'/ nextgroup=quoinUserStringName" . s:k . s:c
  execute 'syn match quoinUserStringName' . s:k . ' /\h\k*/ contained nextgroup=quoinString'
  " User list #Name(...) — # and parens like collection braces.
  execute 'syn match quoinUserListHash' . s:k . ' /#\ze\h\k*(/ nextgroup=quoinUserListName' . s:k . s:c
  execute 'syn match quoinUserListName' . s:k . ' /\h\k*/ contained nextgroup=quoinUserListParen' . s:k
  execute 'syn region quoinUserListParen' . s:k . ' matchgroup=quoinCollectionBrace'
        \ . ' start=/(/ end=/)/ contained contains=@quoinCtx' . s:k

  execute 'syn cluster quoinCtx' . s:k . ' contains=@quoinCommon'
        \ . ',quoinIdent' . s:k . ',quoinHeader' . s:k . ',quoinBlock' . s:k
        \ . ',quoinList' . s:k . ',quoinSet' . s:k . ',quoinDict' . s:k . ',quoinParen' . s:k
        \ . ',quoinUserStringHash' . s:k . ',quoinUserListHash' . s:k
endfor

syn cluster quoinCommon contains=quoinGlobal,quoinInstanceIdent,quoinNamespace,
      \quoinNumber,quoinError,quoinComment,quoinString,quoinSymbol,quoinRegex,
      \quoinSelector,quoinKeyword

" --- colors (hex values from colors_for(); cterm = nearest xterm-256) ----------
hi def quoinIdentA guifg=#5fd7af ctermfg=79
hi def quoinIdentB guifg=#aeb1ab ctermfg=145
hi def quoinIdentC guifg=#c79ca9 ctermfg=181
hi def quoinIdentD guifg=#85b9a5 ctermfg=108
hi def quoinGlobal guifg=#ef65a5 ctermfg=205
hi def quoinInstanceIdent guifg=#6ab1c2 ctermfg=74
hi def quoinNamespace guifg=#d53b82 ctermfg=168
hi def quoinNumber guifg=#00bfff ctermfg=39
hi def quoinString guifg=#4682b4 ctermfg=67
hi def link quoinSymbol quoinString
hi def link quoinRegex quoinString
" ANSI renders comments #b9bdba + faint (SGR 2); vim has no faint, so the
" dimming is baked in (2/3 of the hex).
hi def quoinComment guifg=#7b7e7c ctermfg=244
hi def quoinSelector guifg=#ab82ff ctermfg=141
hi def quoinKeyword guifg=#e0a45a gui=bold ctermfg=179 cterm=bold
hi def quoinUsePath guifg=#6aa9e0 ctermfg=74
hi def link quoinUsePkg quoinNamespace
hi def quoinError guifg=#d9534f gui=bold ctermfg=167 cterm=bold
hi def quoinCollectionBrace guifg=#93c6a5 ctermfg=115
hi def link quoinDictKeyGlobal quoinGlobal
" Block depth d gets BlockBrace colors[d % 5] (see colors_for): level 1 =
" colors[1], ..., level 5 = colors[0], then the levels wrap.
hi def quoinBlockBrace1 guifg=#80f0ff ctermfg=123
hi def quoinBlockBrace2 guifg=#fa859d ctermfg=211
hi def quoinBlockBrace3 guifg=#eabe95 ctermfg=180
hi def quoinBlockBrace4 guifg=#a4dbbe ctermfg=151
hi def quoinBlockBrace5 guifg=#f79c88 ctermfg=216

" Identifier depth d gets Identifier colors[d % 4]; level k sits at depth k-1.
let s:ident_hi = ['quoinIdentA', 'quoinIdentB', 'quoinIdentC', 'quoinIdentD']
for s:k in range(1, 5)
  let s:g = s:ident_hi[(s:k - 1) % 4]
  execute 'hi def link quoinIdent' . s:k . ' ' . s:g
  execute 'hi def link quoinDictKey' . s:k . ' ' . s:g
  execute 'hi def link quoinUserStringName' . s:k . ' ' . s:g
  execute 'hi def link quoinUserListName' . s:k . ' ' . s:g
  execute 'hi def link quoinUserStringHash' . s:k . ' quoinString'
  execute 'hi def link quoinUserListHash' . s:k . ' quoinCollectionBrace'
endfor

unlet s:k s:n s:c s:g s:ident_hi

let b:current_syntax = "quoin"
