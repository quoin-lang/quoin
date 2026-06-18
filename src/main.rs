use new_vm::runner::{VmRunner, VmRunnerOptions};

fn main() {
    let args = std::env::args().collect::<Vec<String>>();
    let options = VmRunnerOptions::parse(&args);
    let runner = VmRunner::new(options);

    if let Err(e) = runner.run() {
        eprintln!("Execution error: {:?}", e);
        std::process::exit(1);
    }
}
