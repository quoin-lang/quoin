(TestSuite.new:{ name = 'Generators' }).add:{
    "* A collection that implements only each: gets all the combinators."
    .test:
    customEachOnly -> {
        GTColl <- { |@items|
            .can:Iterate;
            each: -> { |b| @items.each:{ |x| b.valueWithSelfOrArg:x } }
        };
        c = GTColl.new:{ items = #(1 2 3 4) };
        .is:{ c.collect:{ |x| x * 10 } } equalTo:#(10 20 30 40);
        .is:{ c.select:{ |x| x % 2 == 0 } } equalTo:#(2 4);
        .is:{ c.reduce:{ |a x| a + x } } equalTo:10;
        .is:{ c.first } equalTo:1;
    };

    "* nil is a valid element now - no next/nil-sentinel conflation."
    .test:
    nilIsAValidElement -> {
        .is:{ #(1 nil 3).collect:{ |x| x } } equalTo:#(1 nil 3);
        .is:{ #(1 nil 3).count } equalTo:3;
    };

    "* Iteration is re-entrant: two concurrent passes over one collection."
    .test:
    reentrantIteration -> {
        pairs = #();
        c = #(1 2 3);
        c.each:{ |a| c.each:{ |b| pairs.add:#(a b) } };
        .is:{ pairs.count } equalTo:9;
        .is:{ pairs.first } equalTo:#(1 1);
        .is:{ pairs.last } equalTo:#(3 3);
    };

    "* A Generator turns a yielding block into an iterable, and is re-runnable."
    .test:
    generatorAsIterable -> {
        g = Generator.from:{ ^>1; ^>2; ^>3 };
        .is:{ g.collect:{ |x| x } } equalTo:#(1 2 3);
        .is:{ g.collect:{ |x| x * 2 } } equalTo:#(2 4 6);
    };

    "* Infinite generators are fine when consumed lazily via take:."
    .test:
    infiniteGeneratorTake -> {
        naturals = Generator.from:{ n = 0; { true }.whileDo:{ ^>n; n = n + 1 } };
        .is:{ naturals.take:5 } equalTo:#(0 1 2 3 4);

        fibs = Generator.from:{ a = 0; b = 1; { true }.whileDo:{ ^>a; t = a + b; a = b; b = t } };
        .is:{ fibs.take:8 } equalTo:#(0 1 1 2 3 5 8 13);
    };

    "* External pull iterator: hasNext? + next."
    .test:
    externalIterator -> {
        it = #(10 20 30).iterator;
        .isTrue:{ it.hasNext? };
        .is:{ it.next } equalTo:10;
        .is:{ it.next } equalTo:20;
        .is:{ it.next } equalTo:30;
        .isFalse:{ it.hasNext? };
    };

    "* zip: and drop: are built on the external iterator."
    .test:
    zipAndDrop -> {
        .is:{ (1..4).zip:(10..14) } equalTo:#( #(1 10) #(2 11) #(3 12) );
        .is:{ (1..6).drop:3 } equalTo:#(4 5);
    };
}
