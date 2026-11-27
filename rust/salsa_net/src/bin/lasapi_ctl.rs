// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

fn main() {
    salsa_wire::unix_sys::reset_sigpipe_default();
    std::process::exit(salsa_wire::ctl_cli::run_cli_from_env());
}
