// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa (LASAPI) is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

use std::env;
use std::process::ExitCode;

use salsa_tools::debug_cli::run_debug_cli;
use salsa_wire::unix_sys::reset_sigpipe_default;

fn main() -> ExitCode {
    reset_sigpipe_default();
    let args: Vec<String> = env::args().skip(1).collect();
    ExitCode::from(run_debug_cli(&args))
}
