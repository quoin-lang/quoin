(TestSuite.new:{ name = 'Fibers' }).add:{
    "* The first resume binds the block's parameter; each yield hands a value
    "* back out and suspends until the next resume.
    .test:
    yieldsValuesInOrder -> {
        f = Fiber.new:{ |start|
            n = start;
            { true }.whileDo:{
                Fiber.yield:n;
                n = n + 1;
            }
        };
        .is:{ f.resume:10 } equalTo:10;
        .is:{ f.resume } equalTo:11;
        .is:{ f.resume } equalTo:12;
    };

    "* When the block returns, that value comes out of the final resume and the
    "* fiber becomes done.
    .test:
    returnsFinalValueThenIsDone -> {
        f = Fiber.new:{
            Fiber.yield:1;
            Fiber.yield:2;
            'done'
        };
        .is:{ f.resume } equalTo:1;
        .is:{ f.resume } equalTo:2;
        .isFalse:{ f.done? };
        .is:{ f.resume } equalTo:'done';
        .isTrue:{ f.done? };
    };

    "* resume: passes a value back IN, which becomes the result of the yield
    "* expression inside the fiber. Two-way communication.
    .test:
    twoWayCommunication -> {
        f = Fiber.new:{ |x|
            a = Fiber.yield:(x + 1);
            b = Fiber.yield:(a + 1);
            a + b
        };
        .is:{ f.resume:10 } equalTo:11;
        .is:{ f.resume:100 } equalTo:101;
        .is:{ f.resume:1000 } equalTo:1100;
    };

    "* status walks created -> suspended -> done.
    .test:
    statusTransitions -> {
        f = Fiber.new:{ Fiber.yield:1; 2 };
        .is:{ f.status } equalTo:'created';
        f.resume;
        .is:{ f.status } equalTo:'suspended';
        f.resume;
        .is:{ f.status } equalTo:'done';
        .isFalse:{ f.alive? };
    };

    "* Fibers nest: an outer fiber can drive an inner one, and a yield always
    "* returns to the immediate resumer.
    .test:
    nestedFibers -> {
        inner = Fiber.new:{ Fiber.yield:'a'; Fiber.yield:'b'; 'inner-done' };
        outer = Fiber.new:{
            Fiber.yield:(inner.resume);
            Fiber.yield:(inner.resume);
            inner.resume
        };
        .is:{ outer.resume } equalTo:'a';
        .is:{ outer.resume } equalTo:'b';
        .is:{ outer.resume } equalTo:'inner-done';
    };

    "* The payoff of stackful fibers: yield works even from deep inside a native
    "* method that called back into the block (here, List#each).
    .test:
    yieldsFromInsideEach -> {
        f = Fiber.new:{
            #(10 20 30).each:{ |x| Fiber.yield:x };
            'done'
        };
        .is:{ f.resume } equalTo:10;
        .is:{ f.resume } equalTo:20;
        .is:{ f.resume } equalTo:30;
        .is:{ f.resume } equalTo:'done';
    };

    "* Resuming a finished fiber is an error.
    .test:
    resumingFinishedFiberThrows -> {
        f = Fiber.new:{ 42 };
        .is:{ f.resume } equalTo:42;
        .does:{ f.resume } throw:#/finished Fiber/;
    };

    "* yield only makes sense inside a running fiber.
    .test:
    yieldOutsideFiberThrows -> {
        .does:{ Fiber.yield:1 } throw:#/outside of a Fiber/;
    };

    "* The `^>` operator is sugar for `Fiber.yield:` and behaves identically.
    .test:
    yieldOperatorMatchesYieldMethod -> {
        f = Fiber.new:{ |start|
            n = start;
            { true }.whileDo:{
                ^> n;
                n = n + 1;
            }
        };
        .is:{ f.resume:10 } equalTo:10;
        .is:{ f.resume } equalTo:11;
        .is:{ f.resume } equalTo:12;
    };

    "* `^>` yields from inside a native iterator too, just like the method form.
    .test:
    yieldOperatorFromInsideEach -> {
        f = Fiber.new:{
            #(1 2 3).each:{ |x| ^> (x * 100) };
            'done'
        };
        .is:{ f.resume } equalTo:100;
        .is:{ f.resume } equalTo:200;
        .is:{ f.resume } equalTo:300;
        .is:{ f.resume } equalTo:'done';
    };
}
