// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

#![deny(unsafe_code)]

use std::process::ExitCode;

fn main() -> ExitCode {
    ExitCode::from(salsa_wire::audiodg_engine::run_audiodg_from_env() as u8)
}
