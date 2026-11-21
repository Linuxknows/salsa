// SPDX-License-Identifier: LGPL-3.0-only
// Copyright (C) 2026 Linuxknows
//
// Salsa (LWASAPI) is free software: you can redistribute it and/or modify it
// under the terms of the GNU Lesser General Public License as published by
// the Free Software Foundation, version 3 of the License.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use salsa_tools::host::{install_root, read_text_lossy, write_atomic_secure};
use salsa_tools::zlib_png::ZipWriter;
use salsa_wire::unix_sys::reset_sigpipe_default;

const SKIP_DIRS: &[&str] = &[
    "__pycache__",
    ".git",
    ".pytest_cache",
    "qt6_obj",
    "target",
    "rust_target",
    "rust_target32",
];

const SKIP_NAMES: &[&str] = &[
    "fuzz_wire",
    "fuzz_wire_diff",
    "c_wire_ref_fuzz.o",
    "c_dsp_ref_fuzz.o",
    "salsa-gen-pa-proxy",
    "salsa_pa_proxy_stubs.S",
    "salsa_pa_proxy_stubs32.S",
    "salsa-dist",
    "salsa-regression",
];

const SKIP_SUFFIXES: &[&str] = &[".pyc", ".zip", ".tmp", ".a", ".o", ".moc"];

fn collect_entries(
    root: &Path,
    cur: &Path,
    out: &mut Vec<(String, PathBuf, fs::Metadata)>,
) -> Result<(), String> {
    let rd = fs::read_dir(cur).map_err(|e| format!("read_dir {}: {e}", cur.display()))?;
    let mut children = Vec::new();
    for ent in rd {
        let ent = ent.map_err(|e| format!("dir entry in {}: {e}", cur.display()))?;
        children.push(ent.path());
    }
    children.sort();

    for path in children {
        let Some(name_os) = path.file_name() else {
            continue;
        };
        let name = name_os.to_string_lossy();
        let meta =
            fs::symlink_metadata(&path).map_err(|e| format!("stat {}: {e}", path.display()))?;
        if meta.file_type().is_dir() && !meta.file_type().is_symlink() {
            if SKIP_DIRS.contains(&name.as_ref()) {
                continue;
            }
            collect_entries(root, &path, out)?;
        } else {
            if SKIP_NAMES.contains(&name.as_ref()) {
                continue;
            }
            if SKIP_SUFFIXES.iter().any(|suf| name.ends_with(suf)) {
                continue;
            }
            if !meta.file_type().is_file() && !meta.file_type().is_symlink() {
                continue;
            }
            let rel = path
                .strip_prefix(root)
                .map_err(|e| format!("strip_prefix {}: {e}", path.display()))?;
            let archive_name = format!("salsa-lwasapi/{}", rel.to_string_lossy());
            out.push((archive_name, path, meta));
        }
    }
    Ok(())
}

fn print_help() {
    println!(
        "Usage: salsa-dist [REV] [--out FILE] [--check]\n\
         Packages the Salsa repository and built runtime artifacts into a distributable zip."
    );
}

fn main() -> ExitCode {
    reset_sigpipe_default();
    let mut out_override: Option<PathBuf> = None;
    let mut rev_arg: Option<String> = None;
    let mut check_only = false;

    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print_help();
                return ExitCode::SUCCESS;
            }
            "--check" => check_only = true,
            "--out" => {
                let Some(val) = args.next() else {
                    eprintln!("--out requires a path argument");
                    return ExitCode::from(2);
                };
                out_override = Some(PathBuf::from(val));
            }
            "--rev" => {
                let Some(val) = args.next() else {
                    eprintln!("--rev requires a version string");
                    return ExitCode::from(2);
                };
                rev_arg = Some(val);
            }
            other if !other.starts_with('-') && rev_arg.is_none() => {
                rev_arg = Some(other.to_string());
            }
            other => {
                eprintln!("Unknown argument: {other}");
                return ExitCode::from(2);
            }
        }
    }

    let root = install_root();
    let ver_path = root.join("VERSION");
    let current_ver = read_text_lossy(&ver_path, 256).trim().to_string();
    let rev = match rev_arg {
        Some(r) if !r.trim().is_empty() => {
            let trimmed = r.trim().to_string();
            if trimmed.contains('/') || trimmed.contains('\\') || trimmed.contains("..") {
                eprintln!("salsa-dist invalid revision string: {trimmed}");
                return ExitCode::from(2);
            }
            if out_override.is_none() && !check_only && trimmed != current_ver {
                let _ = write_atomic_secure(&ver_path, format!("{trimmed}\n").as_bytes(), 0o644);
            }
            trimmed
        }
        _ => {
            if current_ver.is_empty() {
                "V1.21.12".to_string()
            } else {
                current_ver
            }
        }
    };

    let tag = if rev.starts_with(['v', 'V']) {
        rev.clone()
    } else {
        format!("v{rev}")
    };
    let out_path = out_override
        .unwrap_or_else(|| root.join(format!("salsa-lwasapi-installer ({tag}).zip")));

    let mut entries = Vec::new();
    if let Err(e) = collect_entries(&root, &root, &mut entries) {
        eprintln!("salsa-dist error: {e}");
        return ExitCode::from(1);
    }

    if check_only {
        println!(
            "salsa-dist check: {} entries ready -> {}",
            entries.len(),
            out_path.display()
        );
        return ExitCode::SUCCESS;
    }

    let mut zw = ZipWriter::new();
    for (archive_name, disk_path, meta) in &entries {
        if let Err(e) = zw.add_fs_entry(archive_name, disk_path, meta) {
            eprintln!("salsa-dist add {archive_name}: {e}");
            return ExitCode::from(1);
        }
    }

    match zw.write_to_path(&out_path) {
        Ok(n) => {
            println!("Built {} ({n} entries)", out_path.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("salsa-dist write {}: {e}", out_path.display());
            ExitCode::from(1)
        }
    }
}
