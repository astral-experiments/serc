mod allocator;
mod cli;
#[cfg(all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"))]
mod compileall_profiles;

use std::process::ExitCode;

#[global_allocator]
static GLOBAL: allocator::MiMalloc = allocator::MiMalloc;

fn main() -> ExitCode {
    cli::main()
}
