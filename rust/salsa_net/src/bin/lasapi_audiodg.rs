// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

fn main() {
    salsa_wire::c_abi::run_c_main(salsa_wire::audiodg_engine::salsa_audiodg_main);
}
