(TestSuite.new:{ name = 'JSON' }).add:{
    .test:
    serialize -> {
        .is:{ Json.serialize:nil } equalTo:'null';
        .is:{ Json.serialize:true } equalTo:'true';
        .is:{ Json.serialize:false } equalTo:'false';
        .is:{ Json.serialize:123 } equalTo:'123';
        .is:{ Json.serialize:123.45 } equalTo:'123.45';
        .is:{ Json.serialize:123.45.decimal } equalTo:'123.45';
        .is:{ Json.serialize:'hi' } equalTo:'"hi"';
        .is:{ Json.serialize:#{'a':1 'b':2} } equalTo:'{"a":1,"b":2}';
        .is:{ Json.serialize:#(1 2 3) } equalTo:'[1,2,3]';
    };

    .test:
    deserialize -> {
        .is:{ (Json.deserialize:'{"a": 42}').a } equalTo:42;
        .is:{ (Json.deserialize:'{"a": 42.0}').a } equalTo:42.0;
        .is:{ (Json.deserialize:'{"a": "hi"}').a } equalTo:'hi';
        .is:{ (Json.deserialize:'{"a": true}').a } equalTo:true;
        .is:{ (Json.deserialize:'{"a": false}').a } equalTo:false;
        .is:{ (Json.deserialize:'{"a": [1,2,3]}').a } equalTo:#( 1 2 3 );
    };

    .test:
    jsonObjectPaths -> {
        .is:{ (Json.deserialize:'{"a": [1,{"five":5},3]}').a.second.five } equalTo:5;
    };
}
