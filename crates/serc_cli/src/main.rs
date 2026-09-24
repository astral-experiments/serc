mod allocator;
mod cli;

use std::process::ExitCode;

#[global_allocator]
static GLOBAL: allocator::MiMalloc = allocator::MiMalloc;

fn main() -> ExitCode {
    cli::main()
}
