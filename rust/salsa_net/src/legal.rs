// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa (LWASAPI) is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

#![forbid(unsafe_code)]

use std::fmt::Write as _;

pub const COPYRIGHT_NOTICE: &str = "Copyright (C) 2026 Linuxknows";
pub const SPDX_ID: &str = "LGPL-3.0-only";
pub const LICENSE_NAME: &str = "GNU Lesser General Public License, version 3.0 only";
pub const REPO_URL: &str = "https://github.com/Linuxknows/salsa";

pub const COPY_TEXT: &str = include_str!("../../../COPYING");
pub const LESSER_COPY_TEXT: &str = include_str!("../../../COPYING.LESSER");
pub const THIRD_PARTY_TEXT: &str = include_str!("../../../THIRD_PARTY_LICENSES.md");

pub fn salsa_version() -> &'static str {
    include_str!("../../../VERSION").trim()
}

pub fn short_legal_notice(app_name: &str) -> String {
    let ver = salsa_version();
    format!(
        "{app_name} — Salsa (LWASAPI) {ver}\n\
         {COPYRIGHT_NOTICE}\n\
         License: {LICENSE_NAME} ({SPDX_ID})\n\
         Source:  {REPO_URL}\n\
         \n\
         This library and software suite is distributed in the hope that it will be useful,\n\
         but WITHOUT ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or\n\
         FITNESS FOR A PARTICULAR PURPOSE. See the GNU Lesser General Public License v3.0\n\
         (COPYING.LESSER) and GNU General Public License v3.0 (COPYING) for details.\n\
         \n\
         Library Relinking & Source (LGPL-3.0 §4): Full Corresponding Source and build rules\n\
         (rust/, qt6/, Makefile) are distributed alongside Salsa so shared and static\n\
         libraries (liblwasapi.so, build/shims/*.so, libsalsa_wire.a, libsalsa_ui.a) and\n\
         dynamically linked Qt6 components can be modified, rebuilt, and replaced.\n\
         \n\
         Optional Host Libraries (Runtime dlopen Only): Salsa does NOT bundle or statically\n\
         link libfdk-aac, libldac, libfreeaptx, libsbc, librnnoise, or libdbus-1. If present\n\
         on the host, optional codecs (including libfdk-aac.so.2 under Fraunhofer terms)\n\
         are loaded solely via dlopen() at runtime and fall back safely when absent.\n\
         See THIRD_PARTY_LICENSES.md or ~/.local/share/licenses/salsa/."
    )
}

pub fn one_line_legal_notice(app_name: &str) -> String {
    format!(
        "{app_name} ({}) — {} — {SPDX_ID}, WITHOUT ANY WARRANTY.",
        salsa_version(),
        COPYRIGHT_NOTICE
    )
}

pub fn full_license_bundle(app_name: &str) -> String {
    let mut out = short_legal_notice(app_name);
    let _ = write!(
        out,
        "\n\n================================================================================\n\
         THIRD-PARTY LIBRARIES & RUNTIME DLOPEN DISCLOSURES\n\
         ================================================================================\n\n\
         {THIRD_PARTY_TEXT}\n\
         ================================================================================\n\
         GNU LESSER GENERAL PUBLIC LICENSE v3.0 (COPYING.LESSER)\n\
         ================================================================================\n\n\
         {LESSER_COPY_TEXT}\n\
         ================================================================================\n\
         GNU GENERAL PUBLIC LICENSE v3.0 (COPYING)\n\
         ================================================================================\n\n\
         {COPY_TEXT}"
    );
    out
}

pub fn handle_common_cli_flag(app_name: &str, arg: &str) -> Option<i32> {
    match arg {
        "--version" | "-V" => {
            println!("{}", short_legal_notice(app_name));
            Some(0)
        }
        "--license" | "--legal" | "license" => {
            print!("{}", full_license_bundle(app_name));
            Some(0)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_lgpl_and_gpl_texts_are_verbatim_and_non_empty() {
        assert!(salsa_version().starts_with('V'));
        assert!(COPY_TEXT.contains("GNU GENERAL PUBLIC LICENSE"));
        assert!(COPY_TEXT.contains("Version 3, 29 June 2007"));
        assert!(LESSER_COPY_TEXT.contains("GNU LESSER GENERAL PUBLIC LICENSE"));
        assert!(LESSER_COPY_TEXT.contains("Version 3, 29 June 2007"));
        assert!(THIRD_PARTY_TEXT.contains("libfdk-aac.so.2"));
        let notice = short_legal_notice("salsa-test");
        assert!(notice.contains("Copyright (C) 2026 Linuxknows"));
        assert!(notice.contains("LGPL-3.0-only"));
        assert!(notice.contains("WITHOUT ANY WARRANTY"));
        assert!(notice.contains("COPYING.LESSER"));
        assert!(notice.contains("COPYING"));
    }
}
