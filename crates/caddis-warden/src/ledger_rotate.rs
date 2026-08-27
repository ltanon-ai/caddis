//! ledger_rotate.rs — CARD-0130. Archive the live ledger; never rewrite it.

use std::fs;
use std::path::{Path, PathBuf};

use crate::identity::{ledger_path, unix_seconds};

pub fn run(args: &[String]) -> i32 {
    match args.get(2).map(String::as_str) {
        Some("rotate") => rotate(),
        _ => {
            eprintln!("usage: caddis-warden ledger rotate");
            2
        }
    }
}

fn rotate() -> i32 {
    let live = ledger_path();
    if !live.is_file() {
        eprintln!("ledger rotate: no ledger at {}", live.display());
        return 2;
    }
    let archive = unique_archive(&live);
    if let Err(e) = fs::rename(&live, &archive) {
        eprintln!("ledger rotate: rename failed: {e}");
        return 1;
    }
    if let Err(e) = fs::write(&live, b"") {
        eprintln!(
            "ledger rotate: archived {} but empty live file failed: {e}",
            archive.display()
        );
        return 1;
    }
    println!("ledger rotate: archived {}", archive.display());
    0
}

fn unique_archive(live: &Path) -> PathBuf {
    let ts = unix_seconds();
    let mut n = 0u32;
    loop {
        let suffix = if n == 0 {
            format!(".{ts}")
        } else {
            format!(".{ts}-{n}")
        };
        let name = match live.file_name() {
            Some(s) => {
                let mut o = s.to_os_string();
                o.push(suffix);
                o
            }
            None => return live.with_extension(format!("{ts}")),
        };
        let cand = live.with_file_name(name);
        if !cand.exists() {
            return cand;
        }
        n += 1;
        if n > 1000 {
            return cand;
        }
    }
}
