//! caddis-deliberate CLI — P4 slice 1: the organ home bootstrap.
//!
//! `seed` — create the home's card stream ONCE from the desktop catalog
//!           (idempotent on identical bytes; NEVER overwrites a diverged
//!           stream — edits ride the warden-gated path), then prove the
//!           cached view against the stream digest (F2).
//! `view` — load + sync the view against the stream truth and print the
//!           view JSON verbatim on stdout: that JSON is the machine
//!           surface the world's bridge (P4 slice 2) reads, so stdout
//!           stays PURE JSON — every human word goes to stderr.
//!
//! Defaults follow the estate home law (caddis-warden identity.rs
//! precedent): catalog `~/.pi/agent/models.json`, home
//! `~/.caddis/deliberate/`, stream `seats.jsonl`, view `seats-view.json`.
//! `USERPROFILE` wins over `HOME` (Windows); an unset profile falls back
//! to "." so the failure is VISIBLE in the path, never silent.

use caddis_deliberate::collector::{seed_once, SeedOutcome};
use caddis_deliberate::registry;
use std::path::PathBuf;
use std::process::ExitCode;

fn home() -> PathBuf {
    let h = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(h)
}

fn default_models() -> PathBuf {
    home().join(".pi").join("agent").join("models.json")
}

fn default_home_dir() -> PathBuf {
    home().join(".caddis").join("deliberate")
}

/// `--models <path>` (seed only) and `--home <dir>` are the whole surface;
/// anything else is a usage error, not a guess.
fn parse_paths(args: &[String], seed: bool) -> Result<(PathBuf, PathBuf, PathBuf), String> {
    let mut models = default_models();
    let mut dir = default_home_dir();
    let mut i = 0;
    while i < args.len() {
        let flag = args[i].as_str();
        match flag {
            "--models" if seed => {
                i += 1;
                models = take_value(args, i, "--models")?;
            }
            "--home" => {
                i += 1;
                dir = take_value(args, i, "--home")?;
            }
            other => return Err(format!("unknown argument {other:?}")),
        }
        i += 1;
    }
    Ok((models, dir.join("seats.jsonl"), dir.join("seats-view.json")))
}

fn take_value(args: &[String], i: usize, flag: &str) -> Result<PathBuf, String> {
    args.get(i)
        .map(PathBuf::from)
        .ok_or_else(|| format!("{flag} needs a value"))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(|s| s.as_str()) {
        Some("seed") => cmd_seed(&args[1..]),
        Some("view") => cmd_view(&args[1..]),
        _ => {
            usage();
            ExitCode::from(2)
        }
    }
}

fn cmd_seed(args: &[String]) -> ExitCode {
    let (models, stream, view) = match parse_paths(args, true) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("caddis-deliberate seed: {e}");
            return ExitCode::from(2);
        }
    };
    let text = match std::fs::read_to_string(&models) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("caddis-deliberate seed: read {}: {e}", models.display());
            return ExitCode::FAILURE;
        }
    };
    match seed_once(&text, &stream, &view) {
        Ok(SeedOutcome::Created {
            rows,
            skipped,
            view_synced,
        }) => {
            let view_word = proven(view_synced);
            eprintln!(
                "seeded {} ({} rows, {} skipped, view {})",
                stream.display(),
                rows,
                skipped,
                view_word
            );
            ExitCode::SUCCESS
        }
        Ok(SeedOutcome::AlreadySeeded { rows, view_synced }) => {
            let view_word = proven(view_synced);
            eprintln!(
                "already seeded: {} ({} rows unchanged — idempotent; view {view_word})",
                stream.display(),
                rows
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("caddis-deliberate seed: {e}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_view(args: &[String]) -> ExitCode {
    let (_, stream, view) = match parse_paths(args, false) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("caddis-deliberate view: {e}");
            return ExitCode::from(2);
        }
    };
    match registry::load_and_sync(&stream, &view) {
        Ok(_) => match std::fs::read_to_string(&view) {
            Ok(v) => {
                println!("{v}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("caddis-deliberate view: read {}: {e}", view.display());
                ExitCode::FAILURE
            }
        },
        Err(e) => {
            eprintln!("caddis-deliberate view: {}: {e}", stream.display());
            ExitCode::FAILURE
        }
    }
}

fn proven(view_synced: bool) -> &'static str {
    if view_synced {
        "proven (rewritten)"
    } else {
        "already current"
    }
}

fn usage() {
    eprintln!(
        "usage: caddis-deliberate seed [--models <catalog.json>] [--home <dir>]\n       \
         caddis-deliberate view [--home <dir>]\n       \
         defaults: catalog ~/.pi/agent/models.json, home ~/.caddis/deliberate"
    );
}
