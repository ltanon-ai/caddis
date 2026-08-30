//! bee.rs — CARD-0145. Spawn a child stamped with the launcher harness.
//!
//! The bee is a Caddis organ. It does not guess the chair. `--harness
//! omp|claude|qpi` is required. The child inherits CADDIS_HARNESS and
//! CADDIS_WARDEN_FROM. OMP launch → OMP bee. Claude launch → Claude bee.

use std::process::Command;

pub enum Error {
    Usage(String),
    Fail(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Usage(s) | Self::Fail(s) => write!(f, "{s}"),
        }
    }
}

pub fn run(args: &[String]) -> Result<i32, Error> {
    if args.first().map(String::as_str) != Some("spawn") {
        return Err(Error::Usage(
            "usage: caddis bee spawn --harness omp|claude|qpi -- <cmd>".into(),
        ));
    }
    let (harness, cmd) = parse(&args[1..])?;
    if cmd.is_empty() {
        return Err(Error::Usage("bee spawn requires a command after --".into()));
    }
    if !crate::which::warden_on_path() {
        return Err(Error::Fail(
            "CONSCIENCE OFFLINE: caddis-warden is not on PATH".into(),
        ));
    }
    let status = Command::new(&cmd[0])
        .args(&cmd[1..])
        .env("CADDIS_HARNESS", &harness)
        .env("CADDIS_WARDEN_FROM", &harness)
        .status()
        .map_err(|e| Error::Fail(format!("spawn {}: {e}", cmd[0])))?;
    Ok(status.code().unwrap_or(1))
}

fn parse(args: &[String]) -> Result<(String, Vec<String>), Error> {
    let mut harness = None;
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--" {
            let kind = harness.ok_or_else(|| {
                Error::Usage("bee spawn requires --harness omp|claude|qpi".into())
            })?;
            return Ok((kind, args[i + 1..].to_vec()));
        }
        if args[i] == "--harness" {
            i += 1;
            let v = args
                .get(i)
                .ok_or_else(|| Error::Usage("missing --harness value".into()))?;
            if !matches!(v.as_str(), "omp" | "claude" | "qpi") {
                return Err(Error::Usage(format!("unknown harness {v}")));
            }
            harness = Some(v.clone());
            i += 1;
            continue;
        }
        return Err(Error::Usage(format!("unknown argument {}", args[i])));
    }
    Err(Error::Usage(
        "bee spawn requires --harness omp|claude|qpi and a command after --".into(),
    ))
}
