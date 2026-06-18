use new_vm::runner::{VmOptions, VmRunner};

fn main() {
    let args = std::env::args().collect::<Vec<String>>();
    let options = VmOptions::parse(&args);
    let runner = VmRunner::new(options);

    if let Err(e) = runner.run() {
        eprintln!("Execution error: {:?}", e);
        std::process::exit(1);
    }
}
