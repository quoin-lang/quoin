runner = TestRunner.new;

testFolder = ([IO]Folder.open:'bblib/tests');
"* (testFolder.entries.sort:{|sf|
"*   (sf.name.split:'-').first.to_integer
"* }).each:{|ef|
    testFolder.each:{|ef|
        (ef.is_file? && (ef.ext == 'b')).if:{
            [IO]Stdout.writeln:'Loading ' + ef.name + '...';
            suite = Runtime.evalFile:ef.fullpath;
            runner.add:suite;
        }
};

runner.add:Runtime.evalFile:'bblib/tests/01-iterate.b';

results = runner.run:PlainTestReporter.new:{ out = [IO]Stdout }

results.none?:{|tr| tr.failures.any? }

