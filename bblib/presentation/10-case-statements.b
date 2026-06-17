name.case:{
    .when:'Damon'               do:{ 'Hi Damon'.puts          };
    .when:'Bagel'               do:{ 'Hello delicious'.puts   };
    .when:#/^[A-Z]+$/           do:{ 'NO NEED TO SHOUT'.puts  };
    .when:{|x| x.length > 20 }  do:{ 'YOUR NAME IS LONG'.puts };

    .default:{ 'Who are you and what did you do with my bagel'.puts };
};

'Low' == 5.case:{
    .when:1..10  do:'Low';
    .when:11..20 do:'High';
};
