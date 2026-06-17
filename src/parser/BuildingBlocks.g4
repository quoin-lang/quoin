grammar BuildingBlocks;


/////////// GRAMMAR

program : (stmt ';'?)+ ;

stmt : METHOD_RETURN expr      #MethodReturn
     | YIELD_RETURN expr       #YieldReturn
     | BLOCK_RETURN expr       #BlockReturn
     | assignment              #AssignmentStmt
     | bang3                   #Bang3Stmt
     | dot3                    #Dot3Stmt
     | huh3                    #Huh3Stmt
     | expr                    #ExprStmt
     ;

bang3 : '!!!' ;
dot3 : '...' ;
huh3 : '???' ;

selector : (ident '+'? ':')+ #SelectorWArgs
         | ident             #SelectorNoArgs
         | ident '!'         #SelectorNoArgsBang
         | symbol            #SelectorSymbol
         ;

assignment : lvalue+ '=' expr ;

lvalue : nsvarident              #IdentLValue
       | '*' nsvarident          #SplatLValue
       | '_'                     #IgnoredLValue
       | '*' '_'                 #IgnoredSplatLValue
       | '(' lvalue+ ')'         #SubLValue
       ;

expr : '(' expr ')'                             #NestedExpr
     | '-' expr                                 #UnMinusExpr
     | '+' expr                                 #UnPlusExpr
     | '!' expr                                 #UnBangExpr
     | '%' expr                                 #UnModExpr
     | parent=nsvarident '<-' name=nsvarident
                         '<-' block             #ClassDef2Expr
     | name=nsvarident '<-' block               #ClassDefExpr
     | nsvarident '<-' expr                     #ConstDefExpr
     | expr '<--' block                         #ClassExtExpr
     | selector '->' block                      #MethodDefExpr
     | selector '-->' block                     #MethodExtExpr
     | left=expr '..' right=expr                #RangeExpr
     | ('.' sig=callSig)                        #DefCallExpr
     | subject=expr ('.' sig=callSig)           #ExprCallExpr
     | left=expr '+' right=expr                 #AddExpr
     | left=expr '-' right=expr                 #SubExpr
     | left=expr '/' right=expr                 #DivExpr
     | left=expr '*' right=expr                 #MulExpr
     | left=expr '%' right=expr                 #ModExpr
     | left=expr '~' right=expr                 #MatchExpr
     | left=expr '>' '=' right=expr             #GtEqExpr
     | left=expr '>' right=expr                 #GtExpr
     | left=expr '<' '=' right=expr             #LtEqExpr
     | left=expr '<' right=expr                 #LtExpr
     | left=expr '&&' right=expr                #AndExpr
     | left=expr '||' right=expr                #OrExpr
     | left=expr '==' right=expr                #EqExpr
     | left=expr '!=' right=expr                #NotEqExpr
     | USER_LIST_START expr* ')'                #UserListExpr // :(
     | '#' '(' expr* ')'                        #ListExpr
     | '#' '<' expr* '>'                        #SetExpr
     | '#' '{' ( k+=expr ':' v+=expr )* '}'     #DictExpr
     | number                                   #LiteralNumber
     | string                                   #LiteralString
     | symbol                                   #LiteralSymbol
     | block                                    #BlockExpr
     | nsvarident                               #IdentExpr
     | REGEXP                                   #RegexExpr
     | userString                               #UserStringExpr
     ;

userString : USER_STRING ;

callSig : (id+=ident ':' val+=expr)+ #CallSigWArg
        | id=ident                   #CallSigNoArg
        | id=ident '!'               #CallSigNoArgBang
        ;

nsvarident : ns=namespace ident #NamespacedIdent
           | '@' ident          #InstanceIdent
           | ident              #LocalIdent
           ;

namespace : '[' first=ident ('/' rest=ident)* '/'? ']' #FullNS
          | '[' '/' ']'                                #RootNS
          ;

// XXX: It would reduce duplication to allow any ident here and check against Constants.KeywordConstants.
keyword : 'nil' | 'true' | 'false' ;

block : '{' symbol blockDecls (stmt ';'?)* '}' #NamedBlockWDecls
      | '{' blockDecls (stmt ';'?)* '}'        #BlockWDecls
      | '{' (stmt ';'?)* '}'                   #BlockNoDecls
      ;

blockDecls : '|' blockArg* block? '-' blockDecl* '|'
           | '|' blockArg* block? '|'
           ;

blockArg : '_'                             #BlockArgIgnored
         | name=argIdent ':' argtype=ident #BlockArgTyped
         | name=argIdent                   #BlockArgUntyped
         ;

blockDecl : name=argIdent ':' argtype=ident #BlockDeclTyped
          | name=argIdent                   #BlockDeclUntyped
          ;

string : STRING ;

argIdent : '@' ident  #ArgIdentInst
         | ident      #ArgIdentNormal
         ;

ident : keyword #IdentKeyword
      | IDENT   #IdentOther
      ;

symbol : SYMBOL ;

number : NUMBER ;

/////////// LEXER

WS : [ \r\n\t]+ -> skip ; // ignore whitespace

IDENT : IDENT_PREFIX IDENT_REST* ;

fragment XIDENT : IDENT_PREFIX (IDENT_REST|':')* ;

USER_LIST_START : '#' IDENT '(' ;

SYMBOL : '#' ( STRING | XIDENT ) ;

STRING : '\'' ( ~['\\] | ESCAPE_SEQUENCE )* '\'';

REGEXP : '#' '/' ( ~[/\\] | REGEX_ESCAPE_SEQUENCE )* '/' ;

USER_STRING : '#' IDENT STRING ;

EOL_COMMENT : '"' '*' ~('\n')* -> skip ;

METHOD_RETURN : '^^' ;
YIELD_RETURN : '^>' ;
BLOCK_RETURN : '^' ;

EMPTY_COMMENT : '"' '"' -> skip ;

COMMENT : '"' ~'*' ( '\\' '"' | ~'"' )* '"' -> skip ;

fragment INT : '0' | [1-9] [0-9]* ;

NUMBER : INT ('.' [0-9]+)? | ('.' [0-9]+) ;

fragment IDENT_PREFIX : [a-zA-Z_] ;

fragment IDENT_REST : [a-zA-Z0-9?_]  ;

fragment ESCAPE_SEQUENCE
    : '\\' [tnr"'\\]
    | '\\' [ux] [0-9a-fA-F][0-9a-fA-F][0-9a-fA-F][0-9a-fA-F]
    ;

fragment REGEX_ESCAPE_SEQUENCE
    : '\\' [a-zA-Z0-9\\]
    | '\\' [ux] [0-9a-fA-F][0-9a-fA-F][0-9a-fA-F][0-9a-fA-F]
    ;
