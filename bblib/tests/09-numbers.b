(TestSuite.new:{ name = 'Numbers' }).add:{
    .test:
    literals -> {
        .does:{ 1000000000 } match:Integer;
        .does:{ 1000000000000 } match:Decimal;
    };

    .test:
    misc -> {
        .is:{ 42.next } equalTo:43;
        .is:{ 42.0.next } equalTo:43.0;
        .is:{ 42.decimal.next } equalTo:43.decimal;
    };

    .test:
    addition -> {
        .is:{ 42+1 } equalTo:43;
        .is:{ 42+1.0 } equalTo:43;
        .is:{ 42.0+1 } equalTo:43.0;
        .is:{ 42.0+1.0 } equalTo:43.0;
        .is:{ 42+(-1) } equalTo:41;
        .is:{ 42+(-1.0) } equalTo:41;
        .is:{ 42.0+(-1.0) } equalTo:41.0;

        .is:{ -42+1 } equalTo:-41;
        .is:{ -42+1.0 } equalTo:-41;
        .is:{ -42.0+1 } equalTo:-41.0;
        .is:{ -42.0+1.0 } equalTo:-41.0;
        .is:{ -42+(-1) } equalTo:-43;
        .is:{ -42+(-1.0) } equalTo:-43;
        .is:{ -42.0+(-1.0) } equalTo:-43.0;
    };

    .test:
    subtraction -> {
        .is:{ 42-1 } equalTo:41;
        .is:{ 42-1.0 } equalTo:41;
        .is:{ 42.0-1 } equalTo:41.0;
        .is:{ 42.0-1.0 } equalTo:41.0;
        .is:{ 42-(-1) } equalTo:43;
        .is:{ 42-(-1.0) } equalTo:43;
        .is:{ 42.0-(-1.0) } equalTo:43.0;

        .is:{ -42-1 } equalTo:-43;
        .is:{ -42-1.0 } equalTo:-43;
        .is:{ -42.0-1 } equalTo:-43.0;
        .is:{ -42.0-1.0 } equalTo:-43.0;
        .is:{ -42-(-1) } equalTo:-41;
        .is:{ -42-(-1.0) } equalTo:-41;
        .is:{ -42.0-(-1.0) } equalTo:-41.0;
    };

    .test:
    multiplication -> {
        .is:{ 3*4 } equalTo:12;
        .is:{ 3*4.0 } equalTo:12;
        .is:{ 3.0*4 } equalTo:12.0;
        .is:{ 3.0*4.0 } equalTo:12.0;
        .is:{ 3*(-4) } equalTo:-12;
        .is:{ 3*(-4.0) } equalTo:-12;
        .is:{ 3.0*(-4) } equalTo:-12;
        .is:{ 3.0*(-4.0) } equalTo:-12.0;

        .is:{ -3*4 } equalTo:-12;
        .is:{ -3*4.0 } equalTo:-12;
        .is:{ -3.0*4 } equalTo:-12.0;
        .is:{ -3.0*4.0 } equalTo:-12.0;
        .is:{ -3*(-4) } equalTo:12;
        .is:{ -3*(-4.0) } equalTo:12;
        .is:{ -3.0*(-4) } equalTo:12.0;
        .is:{ -3.0*(-4.0) } equalTo:12.0;
    };

    .test:
    division -> {
        .is:{ 12/3 } equalTo:4;
        .is:{ 12/3.0 } equalTo:4;
        .is:{ 12.0/3 } equalTo:4.0;
        .is:{ 12.0/3.0 } equalTo:4.0;
        .is:{ 12/(-3) } equalTo:-4;
        .is:{ 12/(-3.0) } equalTo:-4;
        .is:{ 12.0/(-3) } equalTo:-4;
        .is:{ 12.0/(-3.0) } equalTo:-4.0;

        .is:{ -12/3 } equalTo:-4;
        .is:{ -12/3.0 } equalTo:-4;
        .is:{ -12.0/3 } equalTo:-4.0;
        .is:{ -12.0/3.0 } equalTo:-4.0;
        .is:{ -12/(-3) } equalTo:4;
        .is:{ -12/(-3.0) } equalTo:4;
        .is:{ -12.0/(-3) } equalTo:4;
        .is:{ -12.0/(-3.0) } equalTo:4.0;
    };

    .test:
    string -> {
        .is:{ -42.s } equalTo:'-42';
        .is:{ 0.42.s } equalTo:'0.42';
    };

    .test:
    unary -> {
        .is:{ -42 } equalTo:-42;
        .is:{ --42 } equalTo:42;
        .is:{ +42 } equalTo:42;
        .is:{ +-42 } equalTo:-42;
    };

    .test:
    equality -> {
        .isTrue:{ 1 == 1 };
        .isTrue:{ 1.0 == 1.0000000000000001 };
        .isTrue:{ 1 == 1.0 };
        .isFalse:{ 1 == 2 };
        .isFalse:{ 1.0 == 1.00000000000001 };
        .isFalse:{ 1.0 == 2 };

        .isFalse:{ 1 == true };
        .isFalse:{ 1 == 'kombucha' };
        .isFalse:{ 1 == #(1) };
        .isFalse:{ 1 == #{'1': 1} };
        .isFalse:{ 1 == nil };
        .isFalse:{ 1 == #sym };

        .isFalse:{ 1 != 1 };
        .isFalse:{ 1.0 != 1.0000000000000001 };
        .isFalse:{ 1 != 1.0 };
        .isTrue:{ 1 != 2 };
        .isTrue:{ 1.0 != 1.00000000000001 };
        .isTrue:{ 1.0 != 2 };

        .isTrue:{ 1 != true };
        .isTrue:{ 1 != 'kombucha' };
        .isTrue:{ 1 != #(1) };
        .isTrue:{ 1 != #{'1': 1} };
        .isTrue:{ 1 != nil };
        .isTrue:{ 1 != #sym };
    };
}
