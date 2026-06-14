(TestSuite.new:{ name = 'Classes' }).add:{
    .test:
    operatorNew -> {
        TSCNC_Constant <- 42;

        .does:{ TSCNC1 <- {} } resultIn:{ TSCNC1.class == Class };
        .does:{ TSCNC1 <- TSCNC2 <- {} } resultIn:{ TSCNC2.parent == TSCNC1 };
        .does:{ Abc <- TSCNC3 <- {} } throw:'Undefined variable Abc';
        .does:{ TSCNC_Constant <- TSCNC3 <- {} } throw:'Parent of TSCNC3 (TSCNC_Constant) is not a Class';
    };

    .test:
    operatorNewReturnsClass -> {
        newClass = nil;
        .does:{ newClass = TSCNC4 <- {} } resultIn:{ newClass == TSCNC4 };
    };

    .test:
    operatorExtend -> {
        .is:{ (42.class <-- { abc -> { 'Hi' } }); 42.abc } equalTo:'Hi';
        .is:{ n = 42; n <-- { inst -> { 'Hi' } }; n.inst; } equalTo:'Hi';
    };

    .test:
    operatorExtendReturnsLHS -> {
        .is:{ TSCNC1 <-- {} } equalTo:TSCNC1;
        .is:{ 42 <-- {} } equalTo:42;
    };

    .test:
    runsInit -> {
        .is:{
            TSCNC5 <- { |@d|
                init -> { @d = 77 }
                d -> { @d }
            };
            TSCNC5.new.d
        } equalTo:77;
    };

    .test:
    runsBlockInit -> {
        .is:{
            TSCNC6 <- { |@d|
                init: -> {|a b c| @d = a+b+c }
                d -> { @d }
            };
            (TSCNC6.new: { a = 10; b = 20; c = 30 }).d
        } equalTo:60;
    };

    .test:
    mixins -> {
        .is:{
            TSCNC7 <- { |@a @b|
                init -> { @a = 1; @b = 2 }
                m -> { @a }
                b -> { @b }
                z -> { @c }
            }

            TSCNC8 <- { |@b @c|
                .mix:TSCNC7;

                init -> {
                    @b = 11;
                    @c = 22
                }

                b -> { @b }
                c -> { #( @a @b @c ) }
            }

            newObj = TSCNC8.new;
            #( newObj.c newObj.m newObj.z newObj.b )
        } equalTo:#( #( 1 11 22 ) 1 22 );
    };
}
