(TestSuite.new:{ name = 'Blocks' }).add:{
    .test:
    arity -> {
        .is:{ {}.arity } equalTo:0;
        .is:{ {|x|}.arity } equalTo:1;
        .is:{ {|x y|}.arity } equalTo:2;
        .is:{ {|_ y|}.arity } equalTo:2;
    };

    .test:
    args -> {
        .is:{ {}.args } equalTo:#();
        .is:{ {|x|}.args } equalTo:#( 'x' );
        .is:{ {|x y|}.args } equalTo:#( 'x' 'y' );
        .is:{ {|_ y|}.args } equalTo:#( '_' 'y' );
    };

    .test:
    blockName -> {
        .is:{ {}.name } equalTo:nil;
        .is:{ {#x |-|}.name } equalTo:'x';
        .is:{ {#x |x|}.name } equalTo:'x';
    };

    .test:
    code -> {
        .is:{ {|a b| a+b }.code } equalTo:'{|a b| a+b }';
    };

    .test:
    value -> {
        .is:{ {1; 2; 3}.value } equalTo:3;
    };

    .test:
    valueArgs -> {
        .is:{ {|n| n*n}.value:3 } equalTo:9;
        .is:{ {|l| (l.at:0) + (l.at:1)}.value:#(10 5) } equalTo:15;
        .is:{ {|x y| x+y}.valueWithArgs:#(10 5) } equalTo:15;
    };

    .test:
    valueSelf -> {
        .is:{ {.s}.valueWithSelf:42 } equalTo:'42';
    };

    .test:
    valueSelfArgs -> {
        .is:{ {|x| .s + x}.value:'bar' withSelf:'foo' } equalTo:'foobar';
        .is:{ {|x y| .s + x + y}.value:#('bar' 'baz') withSelf:'foo' } equalTo:'foobarbaz';
    };

    .test:
    catch -> {
        .is:{ { 42.throw }.catch:{|ex| ex} } equalTo:42;
    };

    .test:
    catchFinally -> {
        finally = nil;
        .does:{
            { 42.throw }.catch:{|ex| finally = ex } finally:{ finally = 'Finally' }
        }
        resultIn:{
            finally == 'Finally'
        };
    };

    .test:
    match -> {
        .does:{ { .length == 3 } } match:'abc';
        .does:{ { |s| s.length == 3 } } match:'abc';
    };
}
