use quoin::runner::{VmRunner, VmRunnerOptions};

use std::{env, process};

// Default-on for optimized builds (release/profiling); debug builds use the
// system allocator so malloc-debug tooling (leaks, ASan, MallocScribble) works.
// Turn off entirely with `--no-default-features`.
#[cfg(all(feature = "mimalloc", not(debug_assertions)))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() {
    let args = env::args().collect::<Vec<String>>();
    let mut options = VmRunnerOptions::parse(&args);

    let supports_color = if env::var("NO_COLOR").is_ok() {
        false
    } else {
        console::colors_enabled()
    };

    options.vm_options.supports_color = supports_color;

    let console_width = console::Term::stdout().size_checked().map(|(_, cols)| cols);
    options.vm_options.console_width = console_width;

    let runner = VmRunner::new(options);

    if let Err(e) = runner.run() {
        eprintln!("Execution error: {:?}", e);
        process::exit(1);
    }
}
