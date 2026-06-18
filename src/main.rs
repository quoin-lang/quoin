use new_vm::runner::{VmRunner, VmRunnerOptions};

use std::{env, process};

fn main() {
    let args = env::args().collect::<Vec<String>>();
    let mut options = VmRunnerOptions::parse(&args);

    let supports_color = if env::var("NO_COLOR").is_ok() {
        false
    } else {
        console::colors_enabled()
    };

    options.vm_options.supports_color = supports_color;

    let runner = VmRunner::new(options);

    if let Err(e) = runner.run() {
        eprintln!("Execution error: {:?}", e);
        process::exit(1);
    }
}
