//! The caddis-router executable (P2 remainder, first slice: the verify CLI).
//!
//! `caddis-router verify [--ledger <path>] [--home <dir>] [--json]`
//!
//! Wires the library's honest verifier to the organ's real state home:
//! `<home>/ledger.jsonl`, default home `~/.caddis/router` (voice-organ
//! convention). Exit code = the finding COUNT (model-voice convention — a
//! ledger tool reports what IS, never silently repairs); usage and IO
//! failures exit 2 loudly. A ledger that does not exist YET is clean and
//! says so: the first route decision simply has not happened.
//!
//! The append path stays in the LIBRARY (F1: no dispatch in the crate); this
//! bin is read-only — the operator's audit surface, the future bee feed's
//! upstream.

use caddis_router::{verify_path, VERSION};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "usage: caddis-router verify [--ledger <path>] [--home <dir>] [--json]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    }
    let mut ledger: Option<PathBuf> = None;
    let mut json = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--version" => {
                println!("caddis-router {VERSION}");
                return ExitCode::SUCCESS;
            }
            "--help" | "-h" => {
                println!("{USAGE}");
                println!("  --ledger <path>  verify this exact ledger file (wins over --home)");
                println!("  --home <dir>     organ state home (default ~/.caddis/router)");
                println!("  --json           machine report on stdout");
                println!("exit code = finding count (model-voice convention)");
                return ExitCode::SUCCESS;
            }
            "verify" => i += 1,
            "--json" => {
                json = true;
                i += 1;
            }
            "--ledger" if i + 1 < args.len() => {
                ledger = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "--home" if i + 1 < args.len() => {
                ledger = Some(PathBuf::from(&args[i + 1]).join("ledger.jsonl"));
                i += 2;
            }
            other => {
                eprintln!("caddis-router: unknown argument {other:?}");
                eprintln!("{USAGE}");
                return ExitCode::from(2);
            }
        }
    }
    let ledger = ledger.unwrap_or_else(|| default_home().join("ledger.jsonl"));
    match verify_path(&ledger) {
        Ok(rep) => {
            let exists = ledger.exists();
            if json {
                print_json(&ledger, exists, &rep);
            } else {
                print_human(&ledger, exists, &rep);
            }
            // rc = finding count, capped where the process contract ends
            // (u8); a ledger with >255 findings reports the cap honestly.
            let rc = rep.rc().min(255) as u8;
            ExitCode::from(rc)
        }
        Err(e) => {
            eprintln!("caddis-router: verify {}: {e}", ledger.display());
            ExitCode::from(2)
        }
    }
}

/// The organ's state home (voice-organ convention: USERPROFILE, no home
/// crate — zero deps is crate law).
fn default_home() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".caddis")
        .join("router")
}

fn print_human(ledger: &Path, exists: bool, rep: &caddis_router::VerifyReport) {
    if exists {
        println!("ledger: {}", ledger.display());
    } else {
        println!(
            "ledger: {} (missing — no decisions recorded yet)",
            ledger.display()
        );
    }
    println!(
        "lines: {} rows_ok: {} findings: {}",
        rep.lines,
        rep.rows_ok,
        rep.findings.len()
    );
    for f in &rep.findings {
        println!("  line {}: {}: {}", f.line, f.code, f.detail);
    }
}

/// Hand-rolled flat JSON (crate law: zero deps; the same two-character
/// escaping discipline as the ledger encoder — free text goes through
/// `esc`, numbers and bools are Raw by construction).
fn print_json(ledger: &Path, exists: bool, rep: &caddis_router::VerifyReport) {
    fn esc(s: &str) -> String {
        let mut out = String::with_capacity(s.len() + 2);
        for c in s.chars() {
            match c {
                '\\' => out.push_str("\\\\"),
                '"' => out.push_str("\\\""),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
                c => out.push(c),
            }
        }
        out
    }
    let findings: Vec<String> = rep
        .findings
        .iter()
        .map(|f| {
            format!(
                "{{\"line\":{},\"code\":\"{}\",\"detail\":\"{}\"}}",
                f.line,
                esc(f.code),
                esc(&f.detail)
            )
        })
        .collect();
    println!(
        "{{\"version\":\"{}\",\"ledger\":\"{}\",\"exists\":{},\"lines\":{},\"rows_ok\":{},\"rc\":{},\"findings\":[{}]}}",
        VERSION,
        esc(&ledger.display().to_string()),
        exists,
        rep.lines,
        rep.rows_ok,
        rep.rc().min(255),
        findings.join(",")
    );
}
