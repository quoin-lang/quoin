block = { 42 * 43 }
block.value == 1806

blockWithArgs = { |a b| a > b }
false == blockWithArgs.valueWithArgs:#(10 20)

blockWithTypedArgs = { |a:Double b:Integer| a + b }
12.3 == blockWithTypedArgs.valueWithArgs:#(2.3 10)

Block == blockWithTypedArgs.class
2 == blockWithTypedArgs.arity
#( 'a' 'b' ) == blockWithTypedArgs.args
