(TestSuite.new:{ name = 'Iterate' }).add:{
    .test:
    collect -> {
        .is:{ (1..5).collect:{|n| n*10 } } equalTo:#( 10 20 30 40 );
    };

    .test:
    select -> {
        .is:{ (1..6).select:{ |n| n%2 == 1 } } equalTo:#( 1 3 5 );
    };

    .test:
    all? -> {
        .isTrue:{ #(1 1 1).all?:{ |n| n == 1 } };
        .isFalse:{ #(1 2 1).all?:{ |n| n == 1 } };
    };

    .test:
    none? -> {
        .isTrue:{ #(1 2 3).none?:{ |n| n == 4 } };
        .isFalse:{ #(1 2 1).none?:{ |n| n == 2 } };
    };

    .test:
    any? -> {
        .isTrue:{ (1..5).any?:{ |n| n == 3 } };
        .isFalse:{ (1..5).any?:{ |n| n == 6 } };
    };

    .test:
    count -> {
        .is:{ (1..5).count } equalTo:4;
        .is:{ (1..5).count:{ |n| n >= 3 } } equalTo:2;
    };

    .test:
    detect -> {
        .is:{ (1..5).detect:{ |n| n == 3 } } equalTo:3;
    };

    .test:
    shorthand -> {
        .is:{ (1..5).first } equalTo:1;
        .is:{ (1..5).second } equalTo:2;
        .is:{ (1..5).third } equalTo:3;
        .is:{ (1..5).fourth } equalTo:4;
        .is:{ (1..5).last } equalTo:4;
    };

    .test:
    flatten -> {
        .is:{ #(1 #(2 3) 4 #(5 #(6 7))).flatten } equalTo:#( 1 2 3 4 5 6 7 );
    };

"
    .test:
    groupBy -> {
        .elementsOf:{ (1..6).groupBy:{|n| n%2 == 0 } } areEqualTo:#{ true:#(2 4) false:#(1 3 5) };
    };
"

    .test:
    contains? -> {
        .isTrue:{ (1..5).contains?:3 };
        .isTrue:{ #(1 3 99 1000).contains?:3 };
        .isFalse:{ (1..5).contains?:6 };
        .isFalse:{ #(1 3 99 1000).contains?:9 };
        .isTrue:{ #{ 'a':1 }.contains?:KeyValuePair.new:{ key='a'; value=1 } };
        .isFalse:{ #{ 'a':1 }.contains?:KeyValuePair.new:{ key='a'; value=2 } };
    };

    .test:
    reduce -> {
        .is:{ (1..6).reduce:{|sum n| sum+n } } equalTo:15;
        .is:{ (1..4).reduce:{ |d x| d.at:x.s put:x} into:#{} } equalTo:#{ '1':1 '2':2 '3':3 };
        .is:{ #{ 'a':1 'b':2 'c':3 }.reduce:{ |l x| l.add:x } into:#() } equalTo:#(
            KeyValuePair.new:{ key='a' value=1 }
            KeyValuePair.new:{ key='b' value=2 }
            KeyValuePair.new:{ key='c' value=3 }
        );
    };

    .test:
    max -> {
        .is:{ #(1 3 2 6 4 5).max:{|a b| a > b } } equalTo:6;
        .is:{ #(1 3 2 6 4 5).max } equalTo:6;
        .is:{ #('a' 'c' 'd' 'b').max } equalTo:'d';
    };

    .test:
    min -> {
        .is:{ #(1 3 2 6 4 5).min:{|a b| a > b } } equalTo:1;
        .is:{ #(1 3 2 6 4 5).min } equalTo:1;
        .is:{ #('a' 'c' 'd' 'b').min } equalTo:'a';
    };

    .test:
    partition -> {
        .is:{ (1..6).partition:{ |n| n%2 == 0 } } equalTo:#( #(2 4) #(1 3 5) );
    };

    .test:
    reject -> {
        .is:{ (1..6).reject:{ |n| n%2 == 0 } } equalTo:#(1 3 5);
    };

    .test:
    uniq -> {
        .is:{ #(1 1 2 5 3 3).uniq.sort } equalTo:#(1 2 3 5);
    };

    "
    .test:
    zip -> {
        .is:{ (1..4).zip:(11..14) } equalTo:#( #(1 11) #(2 12) #(3 13) );
    };
    "

    .test:
    reverse -> {
        .is:{ #( 1 2 3 4 ).reverse } equalTo:#( 4 3 2 1 );
    };

    .test:
    list -> {
        .is:{ (1..4).list } equalTo:#( 1 2 3 );
    };

    .test:
    sort -> {
        .is:{ #( 4 3 1 nil 5 2 7 6 ).sort } equalTo:#( 1 2 3 4 5 6 7 nil );
        .is:{ #{ 'b':1 'a':42 'c':99 }.list.sort:{|kvp| kvp.value } } equalTo:#(
            KeyValuePair.new:{ key='b' value=1 }
            KeyValuePair.new:{ key='a' value=42 }
            KeyValuePair.new:{ key='c' value=99 }
        );
    };
}
