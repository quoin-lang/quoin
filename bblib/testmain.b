"* runner = TestRunner.new
"*
"* [IO]Stdout.write:'Loading tests';
"*
"* lib = [IO]Folder.path:'../../../bblib/tests';
"* lib.entries.each:{|f|
"*     (f.is_file? && f.ext == 'b').if:{
"*         suite = Runtime.evalFile:f.fullpath;
"*         runner.add:suite;
"*         [IO]Stdout.write:'.';
"*     }
"* };
"* [IO]Stdout.writeln:'';
"*
"* results = runner.run:AnsiTestReporter.new:{ out = [IO]Stdout }
"*
"* results.none?:{|tr| tr.failures.any? }
nil
