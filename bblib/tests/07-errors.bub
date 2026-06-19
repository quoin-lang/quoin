(TestSuite.new:{ name = 'Errors' }).add:{
    .test:
    throw -> {
        .does:{ 77.throw } throw:77;
        .does:{ Integer.throw } throw:Integer;
        .does:{ 'Hi'.throw } throw:'Hi';
        .does:{ Error.new.throw } throw:Error;
    };

    "* Class-side `throw:` builds and throws an instance of that error type."
    .test:
    typedThrow -> {
        .does:{ TypeError.throw:'nope' } throw:TypeError;
        "* a TypeError is-an Error, so it matches the base type too"
        .does:{ TypeError.throw:'nope' } throw:Error;
    };

    "* Errors carry a message and an optional payload."
    .test:
    errorCarriesMessageAndPayload -> {
        caught = nil;
        { ArgumentError.throw:'too many' payload:#(1 2 3) }.catch:{ |e| caught = e };
        .is:{ caught.message } equalTo:'too many';
        .is:{ caught.payload } equalTo:#(1 2 3);
        .is:{ caught.class } equalTo:ArgumentError;
    };

    "* Errors are dispatched by type in a catch handler via case/`~`."
    .test:
    catchByType -> {
        classify = { |block|
            block.catch:{ |e|
                e.case:{
                    .when:TypeError do:'type';
                    .when:ArgumentError do:'arg';
                    .default:'other';
                }
            }
        };
        .is:{ classify.value:{ TypeError.throw:'x' } } equalTo:'type';
        .is:{ classify.value:{ ArgumentError.throw:'x' } } equalTo:'arg';
        .is:{ classify.value:{ IndexError.throw:'x' } } equalTo:'other';
    };

    "* Default display is `ClassName: message`."
    .test:
    errorDisplay -> {
        .is:{ (TypeError.new:{ message = 'boom' }).s } equalTo:'TypeError: boom';
    };

    "* User-defined errors just subclass Error."
    .test:
    userDefinedError -> {
        Error <- WidgetError <- {};
        .does:{ WidgetError.throw:'broke' } throw:WidgetError;
        .does:{ WidgetError.throw:'broke' } throw:Error;
    };

    "* Internal runtime errors now surface as structured Error objects you can
    "* dispatch on - and they're still matchable by message via does:throw:."
    .test:
    runtimeTypeErrorIsStructured -> {
        caught = nil;
        { #(1 2).at:'x' }.catch:{ |e| caught = e };
        .isTrue:{ caught.class == TypeError };
        .isTrue:{ TypeError ~ caught };
        .does:{ #(1 2).at:'x' } throw:TypeError;
        .does:{ #(1 2).at:'x' } throw:#/integer/;
    };

    .test:
    runtimeMessageNotUnderstood -> {
        .does:{ 5.bogusMethodXyz } throw:MessageNotUnderstood;
        .does:{ 5.bogusMethodXyz } throw:#/bogusMethodXyz/;
    };
}
