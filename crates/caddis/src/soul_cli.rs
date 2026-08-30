//! soul_cli.rs — CARD-0255. Wires the soul organ (CARD-0253) into the
//! kernel CLI layer. `caddis soul compose [--lineage <id>]` runs ONE compost
//! pass over the lineage's soul.jsonl against its blockers.jsonl, then prints
//! `soul::compose`. The library organ stays pure — all wiring lives here.
//!
//! Epoch source: `util::unix_ms()` is the wall clock that exists today (the
//! card says "epoch from util::epoch_now() or fold epoch — pick what exists";
//! no epoch_now exists, so unix seconds it is). max_age is the card's 3.
//!
//! `--lineage` defaults via the same default_lineage the other subcommands
//! use (prokuratura family). Archetype: `lines/<id>/archetype.md`, fail-soft
//! with the literal default one-liner — a lineage without a birth certificate
//! is a normal state, never a fault.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use caddis_organs::soul;

use crate::lineage;

pub enum Error {
    Usage(String),
    Fail(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Usage(s) => write!(f, "{s}"),
            Error::Fail(s) => write!(f, "{s}"),
        }
    }
}

const HELP: &str = "USAGE: caddis soul compose [--lineage <id>]\n";
const ARCHETYPE_DEFAULT: &str = "ARCHETYPE: unnamed careful builder.\n";

pub fn run(args: &[String]) -> Result<i32, Error> {
    let sub = args
        .first()
        .ok_or_else(|| Error::Usage("soul requires a subcommand".into()))?;
    if sub != "compose" {
        return Err(Error::Usage(format!("unknown soul subcommand {sub}")));
    }
    compose_cmd(&args[1..])
}

fn compose_cmd(args: &[String]) -> Result<i32, Error> {
    let id = take_optional_lineage(args)?;
    let home = home_dir().ok_or_else(|| Error::Fail("HOME is unset".into()))?;
    let id = id.unwrap_or_else(|| default_lineage(&home));
    lineage::validate(&id).map_err(Error::Usage)?;
    let identity = identity_for(&id)?;
    print!("{identity}");
    Ok(0)
}

/// Build the identity block (compost pass + compose + fail-soft archetype)
/// for the given lineage. Public so `brief` prepends it as the orientation
/// HEAD — identity + state + ONE thing (CARD-0255).
pub fn identity_for(id: &str) -> Result<String, Error> {
    lineage::validate(id).map_err(Error::Usage)?;
    let dir = lineage::dir(id).map_err(Error::Fail)?;
    let soul_path = dir.join("soul.jsonl");
    let blockers_path = dir.join("blockers.jsonl");
    let archetype_path = dir.join("archetype.md");
    // One compost pass: decay pain older than max_age, file reminders for open
    // blockers. Epoch = wall-clock unix seconds (the clock that exists today).
    let epoch = caddis_organs::util::unix_ms() / 1000;
    soul::compost(&soul_path, &blockers_path, epoch, 3).map_err(|e| Error::Fail(e.to_string()))?;
    let composed = soul::compose(&soul_path, &archetype_path);
    Ok(ensure_archetype(&composed, &archetype_path))
}
/// Fail-soft archetype: if the archetype file is absent (or empty), prepend the
/// default one-liner so compose output always carries an ARCHETYPE line. A
/// lineage without a birth certificate is normal, never a fault.
fn ensure_archetype(composed: &str, archetype_path: &Path) -> String {
    let has_archetype = fs::read_to_string(archetype_path)
        .map(|t| !t.trim().is_empty())
        .unwrap_or(false);
    if has_archetype {
        composed.to_string()
    } else {
        format!("{ARCHETYPE_DEFAULT}{composed}")
    }
}

/// Take an optional `--lineage <id>` out of args (allows the default).
fn take_optional_lineage(args: &[String]) -> Result<Option<String>, Error> {
    let mut id = None;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if let Some(v) = a.strip_prefix("--lineage=") {
            set_id(&mut id, v.to_string())?;
            i += 1;
        } else if a == "--lineage" {
            let v = args
                .get(i + 1)
                .ok_or_else(|| Error::Usage("missing --lineage value".into()))?;
            set_id(&mut id, v.clone())?;
            i += 2;
        } else {
            return Err(Error::Usage(format!("unknown argument {a}\n{HELP}")));
        }
    }
    Ok(id)
}

fn set_id(slot: &mut Option<String>, v: String) -> Result<(), Error> {
    if slot.is_some() {
        return Err(Error::Usage("duplicate --lineage".into()));
    }
    *slot = Some(v);
    Ok(())
}

fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn default_lineage(home: &Path) -> String {
    let lines_dir = home.join(".caddis").join("rotation").join("lines");
    // swallow: fail-safe-by-law — no lineages dir means default
    if let Ok(entries) = fs::read_dir(&lines_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name
                .chars()
                .next()
                .map(|c| c.is_ascii_lowercase())
                .unwrap_or(false)
            {
                return name;
            }
        }
    }
    "default".to_string()
}
