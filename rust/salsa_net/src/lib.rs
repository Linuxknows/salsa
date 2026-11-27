// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

#![deny(unsafe_code)]
#![deny(warnings)]

pub mod audiodg_engine;
pub mod bluez_cap;
pub mod bluez_ffi;
pub mod bluez_media;
pub mod bluez_runtime;
pub mod bluez_wire;
pub mod c_abi;
pub mod client_seam;
pub mod control_ipc;
pub mod ctl_cli;
pub mod debug_server;
pub mod dmabuf_gl;
pub mod dsp;
pub mod failsafe;
pub mod hw_analog;
pub mod json;
pub mod legal;
pub mod midi_hub;
pub mod pa_server;
pub mod pa_wire;
pub mod pw_broker;
pub mod pw_wire;
pub mod settings;
pub mod state_conf;
pub mod unix_sys;
