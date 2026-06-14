(TestSuite.new:{ name = 'Methods' }).add:{
    .test:
    operatorReturnsMethod -> {
        .is:{ method = z -> {}; #( method.selector method.extension? ) } equalTo:#( #z false );
    };

    .test:
    canEndInBang -> {
        .is:{
            TSMTC1 <- { blam! -> { 'Blam!' } };
            TSMTC1.new.blam!
        } equalTo:'Blam!';
    };

    .test:
    dispatchTypePriority -> {
        .is:{
            TSMTC2 <- {
                x: -> { |x:Integer| 'Integer: %' % (x) }
                x: --> { |x:String| 'String: %' % x }
                x: --> { |x:Object| 'Other: %' % x }
            };
            #( TSMTC2.new.x:55 TSMTC2.new.x:'str' TSMTC2.new.x:true )
        } equalTo:#( 'Integer: 55' 'String: str' 'Other: true' );
    };

    .test:
    dispatchNoMatch -> {
        .does:{
            TSMTC3 <- {
                z: -> { |z:Integer| 'Integer: %' % z }
                z: --> { |z {z~String}| 'String: %' % z }
            };
            TSMTC3.new.z:true
        } throw:#/Missing method 'TSMTC3#z:' on TSMTC3.*for argument type.Boolean.*Candidates were:.*z:Integer/;
    };
}
