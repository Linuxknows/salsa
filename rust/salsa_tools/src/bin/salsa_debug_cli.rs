use std::env;
use std::process::ExitCode;

use salsa_tools::debug_cli::run_debug_cli;
use salsa_wire::unix_sys::reset_sigpipe_default;

fn main() -> ExitCode {
    reset_sigpipe_default();
    let args: Vec<String> = env::args().skip(1).collect();
    ExitCode::from(run_debug_cli(&args))
}
