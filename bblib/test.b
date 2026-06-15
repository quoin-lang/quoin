Mixin <- BuiltinAssertions <- {
    .meta <-- {
        assertMeetsRequirements: -> { |class:Class|
            (class.implements?:#addResult:).else:{
                ('Class % must implement #addResult: to use BuiltinAssertions' % class.name).throw
            }
        }
    }

    recordResult:evidence:block: -> { |testBlock:Block evidenceArg:List block:Block|
        assertionName = block.name.defined?.if:{ block.name } else:{ '{code}' };

        .reporter.startAssertion:assertionName;

        assertionResult = nil
        assertionElapsed = Timer.time:{
            assertionResult = testBlock.value
        }

        evidenceTemp = evidenceArg;

        r = self.addResult:TestAssertionResult.new:{
            test = self;
            name = assertionName;
            passed? = (assertionResult == true);
            evidence = evidenceTemp;
            elapsed = assertionElapsed;

            "* block.source.defined?.if:{ location = block.source }
        };

        .reporter.endAssertion:r;

        ^r
    }

    isTrue: -> { |block:Block| .recordResult:{true==block.value} evidence:#(true '!=' '{code}') block:block }
    isFalse: -> { |block:Block| .recordResult:{false==block.value} evidence:#(false '!=' '{code}') block:block }

    is:a: -> {|block:Block expected| actual = block.value; .recordResult:{actual ~ expected} evidence:#(expected '!~' actual) block:block }
    is:an: -> {|block:Block expected| .is:block a:expected }

    is:equalTo: -> {|block:Block expected| actual = block.value; .recordResult:{expected==actual} evidence:#(expected '!=' actual) block:block }
    is:notEqualTo: -> {|block:Block expected| actual = block.value; .recordResult:{expected!=actual} evidence:#(expected '==' actual) block:block }

    is:lessThan: -> {|block:Block expected| actual = block.value; .recordResult:{actual<expected} evidence:#(actual 'not <' expected) block:block }
    is:greaterThan: -> {|block:Block expected| actual = block.value; .recordResult:{actual>expected} evidence:#(actual 'not >' expected) block:block }
    is:lessThanOrEqualTo: -> {|block:Block expected| actual = block.value; .recordResult:{actual<=expected} evidence:#(actual 'not <=' expected) block:block }
    is:greaterThanOrEqualTo: -> {|block:Block expected| actual = block.value; .recordResult:{actual>=expected} evidence:#(actual 'not >=' expected) block:block }

    does:match: -> {|block:Block expected| actual = block.value; .recordResult:{actual ~ expected} evidence:#(expected 'didn\'t match' actual) block:block }
    does:notMatch: -> {|block:Block expected| actual = block.value; .recordResult:{!(actual ~ expected)} evidence:#(expected 'matched' actual) block:block }

    does:resultIn: -> {|block:Block expectedBlock:Block|
        block.value;
        .recordResult:{expectedBlock.value == true}
             evidence:#('{code}' 'does not result in' '{code}')
                block:block
    }

    does:throw: -> {|block:Block expectedError|
        actualError = nil;
        { #doesThrowBlock |-| block.value }.catch:{#doesThrowCatchBlock |x| actualError = x };
        .recordResult:{ expectedError ~ actualError }
             evidence:#(
                actualError.s
                'was thrown instead of'
                expectedError
            )
                block:block
    }
}

BuiltinAssertions <- IterateAssertions <- {
    elementsOf:areEqualTo: -> {|block:Block expected| .is:{ block.value.sort } equalTo:expected.sort }
}

TestAssertionResult <- { |@test @name @passed? @location @elapsed @evidence|
    init: -> { |test name:String passed?:Boolean location:String evidence:List elapsed|
        @test = test
        @name = name
        @passed? = passed?
        @location = location
        @elapsed = elapsed
        @evidence = evidence
    }

    test -> { @test }
    name -> { @name }
    passed? -> { @passed? }
    location -> { @location }
    elapsed -> { @elapsed }
    evidence -> { @evidence }
    expected -> { @evidence.first }
    comparison -> { @evidence.second }
    actual -> { @evidence.third }

    s --> {
        'TestAssertionResult{%1 %2 %3%4%5 %6ms %7}' % #(
            @test
            @passed?
            @evidence.first
            @evidence.second
            @evidence.third
            @elapsed
            @location
        )
    }
};

Test <- { |@name @method @assertions @reporter @wallElapsed|
    .mix:BuiltinAssertions;
    .mix:IterateAssertions;

    init: -> { |method:Method|
        @method = method
        @name = method.selector

        @assertions = #()
    }

    addResult: -> { |result:TestAssertionResult|
        @assertions.add:result
        ^result
    }

    name -> { @name }
    reporter -> { @reporter }
    passes -> { @assertions.select:{ |a| a.passed? } }
    failures -> { @assertions.select:{ |a| a.passed? == false } }
    elapsed -> { @assertions.sum:{ |a| a.elapsed } }
    wallElapsed -> { @wallElapsed }

    run: -> { |reporter|
        @reporter = reporter

        @wallElapsed = Timer.time:{
            @method.callOn:self
        }

        TestResult.new:{
            test = self
            name = @name
            assertions = @assertions
            elapsed = @assertions.sum:{ .elapsed }
            wallElapsed = @wallElapsed
        }
    }
};

TestResult <- { |@test @name @assertions @elapsed @wallElapsed|
    init: -> { |test:Test name:String assertions:List elapsed wallElapsed|
        @test = test
        @name = name
        @assertions = assertions
        @elapsed = elapsed
        @wallElapsed = wallElapsed
    }

    test -> { @test }

    name -> { @name }
    assertions -> { @assertions }
    passes -> { @assertions.select:{ .passed? } }
    failures -> { @assertions.select:{ .passed? == false } }
    elapsed -> { @elapsed }
    wallElapsed -> { @wallElapsed }

    s --> { 'TestResult{%1 %2 %3ms [%4]}' % #( @test @name @elapsed @assertions ) }
};

TestSuite <- { |@name @tests|
    init: -> { |name:String|
        @name = name

        @tests = #()
    }

    name -> { @name }

    add: -> { |b:Block|
        b.valueWithSelf:self
        ^self
    }

    test: -> { |m:Method|
        (m.block.arity > 1).if:{
            %'Test method %{m.selector} must have 0 or 1 args'.throw
        }
        @tests.add:Test.new:{ method = m }
    }

    tests -> { @tests }
};

TestReporter <- {
    .abstract!;

    startSuite: -> { |suite:TestSuite| ... }
    endSuite:elapsed:   -> { |suite:TestSuite elapsed| ... }

    startTest: -> { |test:Test| ... }
    endTest:   -> { |result:TestResult| ... }

    startAssertion: -> { |assertion| ... }
    endAssertion:   -> { |result| ... }
};

TestReporter <- PlainTestReporter <- { |@out @currentSuite|
    init: -> { |out| @out = out }

    startSuite: --> { |suite:TestSuite|
        @currentSuite = suite;
        @out.writeln:'[%1] Running %2 tests' % #( @currentSuite suite.tests.count )
    }

    endSuite:elapsed: --> { |suite:TestSuite elapsed|
        passes = suite.tests.sum:{ .passes.count }
        failures = suite.tests.sum:{ .failures.count }
        testsElapsed = suite.tests.sum:{ .elapsed }
        @out.writeln:'[%1] Finished in %4ms (%5ms) : %2 passes / %3 failures' % #(
            @currentSuite
            passes
            failures
            testsElapsed*1000.0
            elapsed*1000.0
        )
    }

    startTest: --> { |test:Test|
        @out.write:'[%1]   Test %2 ' % #(@currentSuite test)
    }

    endTest: --> { |result:TestResult|
        @out.write:' %1ms (%2ms)' % #(
            result.elapsed*1000.0
            result.wallElapsed*1000.0
        )
        (result.failures.any?).if:{
            @out.writeln:' %1 of %2 assertions failed:' % #(
                result.failures.count result.assertions.count
            );
            result.failures.each:{ |fr|
                @out.writeln:'[%1]     %2 %3 %4 at %5:%6:%7' % #(
                    @currentSuite
                    fr.expected.s.replace:#/\s+/ with:' '
                    fr.comparison
                    fr.actual.s.replace:#/\s+/ with:' '
                    '?' "*fr.location.file
                    '?' "*fr.location.line
                    '?' "*fr.location.column
                )
            };
        }
        else:{
            @out.writeln:' % passed' % result.passes.count
        }
    }

    startAssertion: --> { |assertion| }
    endAssertion:   --> { |assertionResult|
        @out.write:assertionResult.passed?.if:{'.'} else:{'!'}
    }
};

TestReporter <- AnsiTestReporter <- { |@out @currentSuite|
    init: -> { |out| @out = out }

    startSuite: --> { |suite:TestSuite|
        @currentSuite = suite
        @out.writeln:#ANSI'[$#5fd7af;bw[%1$]] Running %2 tests' % #( @currentSuite.name suite.tests.count )
    }

    endSuite:elapsed: --> { |suite:TestSuite elapsed|
        passes = suite.tests.sum:{ .passes.count }
        failures = suite.tests.sum:{ .failures.count }
        testsElapsed = suite.tests.sum:{ .elapsed }
        @out.writeln:#ANSI'[$#5fd7af;bw[%1$]] Finished in $#00bfff[%4ms$] ($#00bfff[%5ms$]) : $#69ff61[%2 passes$] / $#ff6961[%3 failures$]' % #(
            @currentSuite.name
            passes
            failures
            testsElapsed
            elapsed
        )
    }

    startTest: --> { |test:Test|
        @out.write:#ANSI'[$#5fd7af;bw[%1$]]   Test $#ab82ff[%2$] ' % #(@currentSuite.name test.name)
    }

    endTest: --> { |result:TestResult|
        @out.write:#ANSI' $#00bfff[%1ms$] ($#00bfff[%2ms$])' % #(
            result.elapsed
            result.wallElapsed
        )
        (result.failures.any?).if:{
            failedTests = result.failures.collect:{ |r|
                #ANSI'$#69ff61[%1$] $#bw[%2$] $#ff6961[%3$] at $#ffffff;bw[%4$]:$#00bfff[%5$]:$#00bfff[%6$]' % #(
                    r.expected.s.replace:#/\s+/ with:' '
                    r.comparison
                    r.actual.s.replace:#/\s+/ with:' '
                    '?' "*r.location.file
                    '?' "*r.location.line
                    '?' "*r.location.column
                )
            };
            @out.writeln:#ANSI' $#ff6961[%1 of %2 assertions failed$]:' % #(
                result.failures.count result.assertions.count
            );
            failedTests.each:{ |ft| @out.write:#ANSI'[$#5fd7af;bw[%$]]     ' % @currentSuite.name; @out.writeln:ft };
        }
        else:{
            @out.writeln:#ANSI' $#69ff61[% passed$]' % result.passes.count
        }
    }

    startAssertion: --> { |assertion| }
    endAssertion:   --> { |assertionResult|
        @out.write:assertionResult.passed?.if:{#ANSI'$#69ff61[.$]'} else:{#ANSI'$#ff6961[!$]'}
    }
};

TestRunner <- { |@suites|
    init -> {
        @suites = #()
    }

    add: -> { |suite:TestSuite| @suites.add:suite }

    suites -> { @suites }

    run: -> { |reporter|
        results = #()
        @suites.each:{|s|
            reporter.startSuite:s;

            tr = nil
            elapsed = Timer.time:{
                tr = s.tests.collect:{ #collectTests |t|
                    reporter.startTest:t;

                    r = t.run:reporter

                    reporter.endTest:r

                    ^r
                }
            };

            reporter.endSuite:s elapsed:elapsed;

            results.add:tr
        }
        ^results.flatten
    }
};
