(TestSuite.new:{ name = 'Case' }).add:{
    .test:
    caseWithEquality -> {
        .is:{
            5.case:{
                .when:6 do:'six';
                .when:5 do:'five';
                .when:7 do:'seven';
                .default:'?';
            }
        } equalTo:'five';

        .is:{
            6.case:{
                .when:6 do:{'six'};
                .when:5 do:{'five'};
                .when:7 do:{'seven'};
                .default:{'?'};
            }
        } equalTo:'six';
    };

    .test:
    caseWithMatchRange -> {
        .is:{
            5.case:{
                .when:1..3 do:'low';
                .when:4..6 do:'medium';
                .when:6..9 do:'high';
                .default:{'?'};
            }
        } equalTo:'medium';
    };

    .test:
    caseWithMatchRegex -> {
        .is:{
            'abc'.case:{
                .when:#/b/ do:'b';
                .when:#/e/ do:'e';
                .when:#/h/ do:'h';
                .default:{'?'};
            }
        } equalTo:'b';
    };
};
