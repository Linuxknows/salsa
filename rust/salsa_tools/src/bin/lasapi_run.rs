// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa (LASAPI) is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

use std::env;
use std::os::unix::process::CommandExt;
use std::process::{Command, ExitCode};

use salsa_tools::host::install_root;
use salsa_wire::unix_sys::reset_sigpipe_default;

fn set_default_env(key: &str, val: &str) {
    let empty = env::var_os(key).map(|v| v.is_empty()).unwrap_or(true);
    if empty {
        env::set_var(key, val);
    }
}

fn main() -> ExitCode {
    reset_sigpipe_default();
    let args: Vec<String> = env::args().skip(1).collect();
    if let Some(first) = args.first() {
        if matches!(first.as_str(), "--version" | "-V" | "--license" | "--legal") {
            if let Some(rc) = salsa_wire::legal::handle_common_cli_flag("lasapi-run", first) {
                return ExitCode::from(rc as u8);
            }
        }
    }
    if args.is_empty() || matches!(args[0].as_str(), "-h" | "--help") {
        eprintln!(
            "Usage: lasapi-run <command> [args...]\n\
             Options: --version, --license, -h/--help\n{}",
            salsa_wire::legal::one_line_legal_notice("lasapi-run")
        );
        return if args.is_empty() {
            ExitCode::from(1)
        } else {
            ExitCode::SUCCESS
        };
    }

    let root = install_root();
    let build_dir = root.join("build");
    let shims_dir = build_dir.join("shims");
    let mut ld_path = format!("{}:{}", shims_dir.display(), build_dir.display());
    if let Ok(prev) = env::var("LD_LIBRARY_PATH") {
        if !prev.is_empty() {
            ld_path.push(':');
            ld_path.push_str(&prev);
        }
    }
    env::set_var("LD_LIBRARY_PATH", ld_path);

    set_default_env("PROTON_USE_PIPEWIRE", "0");
    set_default_env("SDL_AUDIODRIVER", "pulse");
    set_default_env("ALSOFT_DRIVERS", "alsa,pulse");

    let err = Command::new(&args[0]).args(&args[1..]).exec();
    eprintln!("lasapi-run: failed to exec '{}': {err}", args[0]);
    ExitCode::from(127)
}
