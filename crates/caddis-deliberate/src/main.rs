//! caddis-deliberate CLI — home bootstrap (P4 s1) + signed SEED path (P4 s3).
//!
//! `seed` — create the home's card stream ONCE from the desktop catalog
//!           (idempotent on identical bytes; NEVER overwrites a diverged
//!           stream — edits ride the warden-gated path), then prove the
//!           cached view against the stream digest (F2).
//! `view` — load + sync the view against the stream truth and print the
//!           view JSON verbatim on stdout: that JSON is the machine
//!           surface the world's bridge (P4 slice 2) reads, so stdout
//!           stays PURE JSON — every human word goes to stderr.
//! `export` — sign the home's stream as a SEED artifact (F13): one flat
//!           JSON object, `sig = HMAC-SHA256(seed.key, canonical)`;
//!           mints the born-once `seed.key` beside the stream at first
//!           export. Artifact to stdout (pure JSON) or `--out <file>`.
//! `verify` — the supply-chain GATE: strict parse + digest + rows +
//!           fingerprint + signature; findings name the broken law.
//! `restore` — CONSTRUCT a home from a seed, ONLY after verify is clean
//!           (tampered seed = refused, nothing written; a diverged
//!           target is never clobbered).
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
        Some("export") => cmd_export(&args[1..]),
        Some("verify") => cmd_verify(&args[1..]),
        Some("restore") => cmd_restore(&args[1..]),
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

/// Shared flags for the seed-artifact verbs: `--home <dir>` (default the
/// organ home) or `--key <file>` (the carry-the-key path); anything else
/// is a usage error, not a guess.
fn parse_seed_args(args: &[String]) -> Result<(PathBuf, Option<PathBuf>), String> {
    let mut dir = default_home_dir();
    let mut key: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--home" => {
                i += 1;
                dir = take_value(args, i, "--home")?;
            }
            "--key" => {
                i += 1;
                key = Some(take_value(args, i, "--key")?);
            }
            other => return Err(format!("unknown argument {other:?}")),
        }
        i += 1;
    }
    Ok((dir, key))
}

fn seed_key_path(dir: &std::path::Path, key: &Option<PathBuf>) -> PathBuf {
    key.clone().unwrap_or_else(|| dir.join("seed.key"))
}

fn cmd_export(args: &[String]) -> ExitCode {
    let mut dir = default_home_dir();
    let mut out: Option<PathBuf> = None;
    let mut i = 0;
    let args = {
        // `--out` is export-only; reuse the shared parser for the rest.
        let mut rest = Vec::new();
        while i < args.len() {
            if args[i] == "--out" {
                i += 1;
                match args.get(i) {
                    Some(v) => out = Some(PathBuf::from(v)),
                    None => {
                        eprintln!("caddis-deliberate export: --out needs a value");
                        return ExitCode::from(2);
                    }
                }
            } else {
                rest.push(args[i].clone());
            }
            i += 1;
        }
        rest
    };
    if let Err(e) = (|| {
        let (d, _) = parse_seed_args(&args)?;
        dir = d;
        Ok::<(), String>(())
    })() {
        eprintln!("caddis-deliberate export: {e}");
        return ExitCode::from(2);
    }
    match caddis_deliberate::seed::export_seed(&dir) {
        Ok(ex) => {
            let minted = if ex.key_minted {
                "key MINTED (born once)"
            } else {
                "key reused"
            };
            eprintln!(
                "exported seed: {} rows, stream {}, fingerprint {}, {minted}",
                ex.rows,
                &ex.stream_sha256[..16],
                ex.fingerprint
            );
            match out {
                Some(path) => match std::fs::write(&path, ex.artifact.as_bytes()) {
                    Ok(_) => {
                        eprintln!("artifact written to {}", path.display());
                        ExitCode::SUCCESS
                    }
                    Err(e) => {
                        eprintln!("caddis-deliberate export: write {}: {e}", path.display());
                        ExitCode::FAILURE
                    }
                },
                None => {
                    print!("{}", ex.artifact);
                    ExitCode::SUCCESS
                }
            }
        }
        Err(e) => {
            eprintln!("caddis-deliberate export: {e}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_verify(args: &[String]) -> ExitCode {
    let (artifact, rest) = match args.split_first() {
        Some((a, r)) if !a.starts_with("--") => (PathBuf::from(a), r.to_vec()),
        _ => {
            eprintln!("caddis-deliberate verify: needs an artifact path");
            return ExitCode::from(2);
        }
    };
    let (dir, key) = match parse_seed_args(&rest) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("caddis-deliberate verify: {e}");
            return ExitCode::from(2);
        }
    };
    let text = match std::fs::read_to_string(&artifact) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("caddis-deliberate verify: read {}: {e}", artifact.display());
            return ExitCode::FAILURE;
        }
    };
    let slot = caddis_deliberate::seed::SeedKeySlot::load(&seed_key_path(&dir, &key));
    let verdict = caddis_deliberate::seed::verify_seed_text(&text, &slot);
    if verdict.clean {
        eprintln!("seed VERIFIED: {verdict}");
        ExitCode::SUCCESS
    } else {
        eprintln!("seed REFUSED: {verdict}");
        ExitCode::from(4)
    }
}

fn cmd_restore(args: &[String]) -> ExitCode {
    let (artifact, rest) = match args.split_first() {
        Some((a, r)) if !a.starts_with("--") => (PathBuf::from(a), r.to_vec()),
        _ => {
            eprintln!("caddis-deliberate restore: needs an artifact path");
            return ExitCode::from(2);
        }
    };
    let mut to: Option<PathBuf> = None;
    let mut shared = Vec::new();
    let mut i = 0;
    while i < rest.len() {
        if rest[i] == "--to" {
            i += 1;
            match rest.get(i) {
                Some(v) => to = Some(PathBuf::from(v)),
                None => {
                    eprintln!("caddis-deliberate restore: --to needs a value");
                    return ExitCode::from(2);
                }
            }
        } else {
            shared.push(rest[i].clone());
        }
        i += 1;
    }
    let Some(to) = to else {
        eprintln!("caddis-deliberate restore: --to <dir> is required");
        return ExitCode::from(2);
    };
    let (dir, key) = match parse_seed_args(&shared) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("caddis-deliberate restore: {e}");
            return ExitCode::from(2);
        }
    };
    let text = match std::fs::read_to_string(&artifact) {
        Ok(t) => t,
        Err(e) => {
            eprintln!(
                "caddis-deliberate restore: read {}: {e}",
                artifact.display()
            );
            return ExitCode::FAILURE;
        }
    };
    let slot = caddis_deliberate::seed::SeedKeySlot::load(&seed_key_path(&dir, &key));
    match caddis_deliberate::seed::restore_seed(&text, &slot, &to) {
        Ok(caddis_deliberate::seed::RestoreOutcome::Constructed { rows }) => {
            eprintln!(
                "home CONSTRUCTED at {} ({rows} rows, view proven)",
                to.display()
            );
            ExitCode::SUCCESS
        }
        Ok(caddis_deliberate::seed::RestoreOutcome::AlreadyIdentical { rows }) => {
            eprintln!("target already identical ({rows} rows, view proven) — idempotent");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("caddis-deliberate restore: {e}");
            ExitCode::from(4)
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
         caddis-deliberate export [--home <dir>] [--out <artifact.json>]\n       \
         caddis-deliberate verify <artifact.json> [--home <dir> | --key <file>]\n       \
         caddis-deliberate restore <artifact.json> --to <dir> [--home <dir> | --key <file>]\n       \
         defaults: catalog ~/.pi/agent/models.json, home ~/.caddis/deliberate\n       \
         exit codes: 0 ok, 1 io error, 2 usage, 4 seed REFUSED (verify gate)"
    );
}
