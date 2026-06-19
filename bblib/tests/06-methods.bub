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
    dispatchByBlock -> {
        .is:{
            TSMTC3 <- {
                x: -> { |x {.class==Integer}| 'Integer: %' % (x) }
                x: --> { |x {.class==String}| 'String: %' % x }
                x: --> { |x {.class==Object}| 'Other: %' % x }
                x: --> { |x:Integer {x > 100}| 'Big Integer: %' % x }
                x: --> { |x:Integer {|n| n < 0}| 'Negative Integer: %' % x }
            };
            #( TSMTC3.new.x:55 TSMTC3.new.x:'str' TSMTC3.new.x:true TSMTC3.new.x:150 TSMTC3.new.x:-10 )
        } equalTo:#( 'Integer: 55' 'String: str' 'Other: true' 'Big Integer: 150' 'Negative Integer: -10' );
    };

"*    .test:
"*    dispatchNoMatch -> {
"*        .does:{
"*            TSMTC3 <- {
"*                z: -> { |z:Integer| 'Integer: %' % z }
"*                z: --> { |z {z~String}| 'String: %' % z }
"*            };
"*            TSMTC3.new.z:true
"*        } throw:#/Missing method 'TSMTC3#z:' on TSMTC3.*for argument type.Boolean.*Candidates were:.*z:Integer/;
"*    };
}
