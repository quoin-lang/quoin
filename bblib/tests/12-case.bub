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

    "* A `when` value that is a Class matches via `~`, which for classes is an
    "* instance-of test. So `.case:` doubles as a type switch.
    .test:
    caseMatchesByClass -> {
        classify = { |val|
            val.case:{
                .when:Integer do:'int';
                .when:String do:'str';
                .when:Boolean do:'bool';
                .default:{'other'};
            }
        };
        .is:{ classify.value:5 } equalTo:'int';
        .is:{ classify.value:'hi' } equalTo:'str';
        .is:{ classify.value:true } equalTo:'bool';
        .is:{ classify.value:3.5 } equalTo:'other';
    };

    "* A `when` value that is a Block is used as a predicate: it is run with the
    "* subject (`cond ~ subject` executes the block on the subject).
    .test:
    caseMatchesByPredicateBlock -> {
        grade = { |score|
            score.case:{
                .when:{ |n| n >= 90 } do:'A';
                .when:{ |n| n >= 80 } do:'B';
                .when:{ |n| n >= 70 } do:'C';
                .default:{'F'};
            }
        };
        .is:{ grade.value:95 } equalTo:'A';
        .is:{ grade.value:85 } equalTo:'B';
        .is:{ grade.value:72 } equalTo:'C';
        .is:{ grade.value:40 } equalTo:'F';
    };

    "* Clauses are tested top-to-bottom and the first match wins, even when a
    "* later clause would also match.
    .test:
    caseFirstMatchWins -> {
        .is:{
            5.case:{
                .when:1..9 do:'first';
                .when:4..6 do:'second';
                .default:{'?'};
            }
        } equalTo:'first';
    };

    "* When the matching `do:` is a block, it receives the subject as its argument.
    .test:
    caseDoBlockReceivesSubject -> {
        .is:{
            5.case:{
                .when:Integer do:{ |n| n * 2 };
                .default:{ 0 };
            }
        } equalTo:10;
    };

    "* With no matching clause and no default, the case expression yields nil.
    .test:
    caseFallsThroughToNilWithoutDefault -> {
        .isFalse:{
            (5.case:{
                .when:1 do:'one';
                .when:2 do:'two';
            }).defined?
        };
    };

    "* The default clause is used when nothing else matches. It accepts either a
    "* block or a bare value, mirroring the two forms of `.when:do:`.
    .test:
    caseDefaultReached -> {
        .is:{
            99.case:{
                .when:1 do:'one';
                .when:2 do:'two';
                .default:{'none'};
            }
        } equalTo:'none';

        .is:{
            99.case:{
                .when:1 do:'one';
                .when:2 do:'two';
                .default:'none';
            }
        } equalTo:'none';
    };

    "* A single case can mix match strategies: literal equality, regex, and class.
    .test:
    caseMixesEqualityRegexAndClass -> {
        describe = { |s|
            s.case:{
                .when:'exact' do:'matched-exact';
                .when:#/^[0-9]+$/ do:'all-digits';
                .when:String do:'some-string';
                .default:{'not-a-string'};
            }
        };
        .is:{ describe.value:'exact' } equalTo:'matched-exact';
        .is:{ describe.value:'12345' } equalTo:'all-digits';
        .is:{ describe.value:'hello' } equalTo:'some-string';
    };
};
