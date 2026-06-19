(TestSuite.new:{ name = 'Range' }).add:{
    .test:
    numberRange -> {
        .is:{ (1..5).list } equalTo:#(1 2 3 4);
        .is:{ (1..5).collect:{ |n| n*10 } } equalTo:#(10 20 30 40);
        .is:{ (5..1).list } equalTo:#(5 4 3 2);
        .is:{ (5..1).collect:{ |n| n*10 } } equalTo:#(50 40 30 20);
        .does:{ (1..5) } match:3;
        .does:{ (1..5) } match:1;
        .does:{ (1..5) } notMatch:5;
        .does:{ (5..1) } match:3;
        .does:{ (5..1) } match:5;
        .does:{ (5..1) } notMatch:1;
    };
}
