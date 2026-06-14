(TestSuite.new:{ name = 'Errors' }).add:{
    .test:
    throw -> {
        .does:{ 77.throw } throw:77;
        .does:{ Integer.throw } throw:Integer;
        .does:{ 'Hi'.throw } throw:'Hi';
        .does:{ Error.new.throw } throw:Error;
    };
}
