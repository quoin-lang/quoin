(TestSuite.new:{ name = 'Booleans' }).add:{
    .test:
    equality -> {
        .isTrue:{ true == true };
        .isTrue:{ false == false };
        .isFalse:{ true == false };
        .isFalse:{ false == true };
        .isFalse:{ true != true };
        .isFalse:{ false != false };
        .isTrue:{ true != false };
        .isTrue:{ false != true };
    };

    .test:
    negate -> {
        .isFalse:{ !true };
        .isTrue:{ !false };
        .isFalse:{ true.not };
        .isTrue:{ false.not };
    };

    .test:
    s -> {
        .is:{ true.s } equalTo:'true';
        .is:{ false.s } equalTo:'false';
    };

    .test:
    conditionals -> {
        .is:{ true.if:{ 11 } else:{ 22 } } equalTo:11;
        .is:{ false.if:{ 11 } else:{ 22 } } equalTo:22;
        .is:{ true.if:{ 11 } } equalTo:11;
        .is:{ false.if:{ 11 } } equalTo:nil;
        .is:{ true.else:{ 22 } } equalTo:nil;
        .is:{ false.else:{ 22 } } equalTo:22;
    };

    .test:
    comparisons -> {
        .isTrue:{ true < false };
        .isTrue:{ true <= false };
        .isFalse:{ true > false };
        .isFalse:{ true >= false };

        .isFalse:{ false < true };
        .isFalse:{ false <= true };
        .isTrue:{ false > true };
        .isTrue:{ false >= true };

        .isFalse:{ true < true };
        .isTrue:{ true <= true };
        .isFalse:{ true > true };
        .isTrue:{ true >= true };

        .isFalse:{ false < false };
        .isTrue:{ false <= false };
        .isFalse:{ false > false };
        .isTrue:{ false >= false };
    };
}
