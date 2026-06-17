runner = TestRunner.new

[IO]Stdout.write:'Loading tests';

suite = Runtime.evalFile:'bblib/tests/01-iterate.b';
runner.add:suite;

results = runner.run:AnsiTestReporter.new:{ out = [IO]Stdout }

results.none?:{|tr| tr.failures.any? }
