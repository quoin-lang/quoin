(TestSuite.new:{ name = 'Strings' }).add:{
    .test:
    equality -> {
        .is:{ 'abc'.s } equalTo:'abc';
        .is:{ 'abc' + 'def' } equalTo:'abcdef';
        .is:{ 'abcdef' } notEqualTo:'abc';
    };

    .test:
    comparisons -> {
        .is:{ 'aa' } lessThan:'ab';
        .is:{ 'ab' } greaterThan:'aa';
        .is:{ 'aa' } lessThanOrEqualTo:'aa';
        .is:{ 'ab' } greaterThanOrEqualTo:'ab';
        .is:{ 'aa' } lessThanOrEqualTo:'ab';
        .is:{ 'ab' } greaterThanOrEqualTo:'aa';
    };

    .test:
    formatting -> {
        .is:{ '%1-%2-%3' % #(42 43 44) } equalTo:'42-43-44';
        .is:{ '%c-%b-%a' % #{ 'a':42 'b':43 'c':44 } } equalTo:'44-43-42';
        .is:{ %'1%{2}3' } equalTo:'123';
    };

    .test:
    misc -> {
        .is:{ 'abcdefg'.length } equalTo:7;

        .isTrue:{ 'abcd'.contains?:'c' };
        .isTrue:{ 'abcd'.ends?:'d' };
        .isTrue:{ 'abcd'.starts?:'a' };
        .is:{ 'abcd'.index:'c' } equalTo:2;
        .is:{ 'abcd'.insert:'e' at:4 } equalTo:'abcde';

        .is:{ 'AbCdE'.lower } equalTo:'abcde';
        .is:{ 'AbCdE'.upper } equalTo:'ABCDE';

        .is:{ 'foo bar baz'.split:' ' } equalTo:#( 'foo' 'bar' 'baz' );
        .is:{ 'foo   bar  baz '.split:#/\s+/ } equalTo:#( 'foo' 'bar' 'baz' '' );
        .is:{ 'foo   bar  baz '.split } equalTo:#( 'foo' 'bar' 'baz' '' );
    };
}
