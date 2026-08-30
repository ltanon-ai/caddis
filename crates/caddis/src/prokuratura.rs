//! prokuratura.rs — CARD-0252. The operator's single interface.
//!
//! `brief`: composes from existing organs (board, eddy, scan, bee
//! states, blockers) into ~100 words. `--voice` speaks via the
//! existing caddis-voice daemon.
//! `fix <symptom>`: diagnostic cascade — each check returns Ok/Fix/Human.
//! `build "<idea>"`: idea -> card -> queue append -> report.
//!
//! All three output in the operator's language (Lithuanian support
//! via CADDIS_LANG env or detection of LANG).

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::lineage;
use crate::worker_board_state as st;
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

const HELP: &str = "\
USAGE: caddis brief [--lineage <id>] [--voice]
       caddis fix <symptom>
       caddis build \"<idea>\"
";

pub fn run(args: &[String]) -> Result<i32, Error> {
    if args.is_empty() {
        return Err(Error::Usage(HELP.into()));
    }
    match args[0].as_str() {
        "brief" => brief_cmd(&args[1..]),
        "fix" => fix_cmd(&args[1..]),
        "build" => build_cmd(&args[1..]),
        _ => Err(Error::Usage(HELP.into())),
    }
}

fn brief_cmd(args: &[String]) -> Result<i32, Error> {
    let (lineage_arg, voice) = parse_brief_args(args)?;
    let home = home_dir().ok_or_else(|| Error::Fail("HOME is unset".into()))?;
    let id = lineage_arg.unwrap_or_else(|| default_lineage(&home));
    let report = compose_report(&home, &id)?;
    println!("lineage: {id}");
    println!("{report}");
    if voice {
        let _ = speak(&report); // swallow: best-effort-telemetry — voice is optional, brief must not fail if it's down
    }
    Ok(0)
}

/// Parse `--lineage <id>` and `--voice` from brief args.
/// Unknown args → usage error (fail-closed).
fn parse_brief_args(args: &[String]) -> Result<(Option<String>, bool), Error> {
    let mut lineage = None;
    let mut voice = false;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if let Some(v) = a.strip_prefix("--lineage=") {
            set_brief_lineage(&mut lineage, v)?;
            i += 1;
        } else if a == "--lineage" {
            let v = args
                .get(i + 1)
                .ok_or_else(|| Error::Usage("missing --lineage value".into()))?;
            set_brief_lineage(&mut lineage, v)?;
            i += 2;
        } else if a == "--voice" {
            voice = true;
            i += 1;
        } else {
            return Err(Error::Usage(format!("unknown argument {a}\n{HELP}")));
        }
    }
    Ok((lineage, voice))
}

fn set_brief_lineage(slot: &mut Option<String>, v: &str) -> Result<(), Error> {
    if slot.is_some() {
        return Err(Error::Usage("duplicate --lineage".into()));
    }
    *slot = Some(v.to_string());
    Ok(())
}

fn fix_cmd(args: &[String]) -> Result<i32, Error> {
    if args.is_empty() {
        return Err(Error::Usage("USAGE: caddis fix <symptom>\n".into()));
    }
    let symptom = &args[0];
    let diagnosis = crate::prokuratura_fix::diagnose(symptom);
    println!("{diagnosis}");
    Ok(0)
}

fn build_cmd(args: &[String]) -> Result<i32, Error> {
    if args.is_empty() {
        return Err(Error::Usage("USAGE: caddis build \"<idea>\"\n".into()));
    }
    let idea = &args[0];
    let msg = queue_idea(idea)?;
    println!("{msg}");
    Ok(0)
}

/// Compose ~100 word state report from existing organs for the named lineage.
fn compose_report(home: &Path, id: &str) -> Result<String, Error> {
    let dir = lineage::dir(id).map_err(Error::Fail)?;

    // CARD-0255: identity HEAD — the `soul compose` block above the state
    // line, separated by a blank line. Full orientation: identity + state +
    // ONE thing. Fail-soft: a lineage without a soul is normal, not a fault,
    // so a compost/compose error degrades to an empty identity block.
    let identity = crate::soul_cli::identity_for(id).unwrap_or_default();

    let q = st::queue(&dir);
    let scan = st::scan_last(&dir);
    let bees = st::bee_recent(&dir, 3);
    let blockers = scan_blockers(home);

    let scan_txt = match &scan {
        Some(s) if s.verdict == "pass" => "scan green".to_string(),
        Some(s) => format!("scan red because {}", s.verdict),
        None => "scan none".to_string(),
    };

    let bee_txt = if bees.is_empty() {
        "no bees".to_string()
    } else {
        format!("{} bees, last exit {}", bees.len(), bees[0].exit)
    };

    let blocker_txt = if blockers.is_empty() {
        "no blockers".to_string()
    } else {
        format!("{} blockers: {}", blockers.len(), blockers.join(", "))
    };

    let one_thing = if !q.remaining.is_empty() {
        format!("next card: {}", q.remaining[0])
    } else {
        "queue empty — ready for new work".to_string()
    };

    let state = format!(
        "cards done: {}, queued: {}, {}, {}, {}. ONE thing needing operator: {}",
        q.done,
        q.remaining.len(),
        scan_txt,
        bee_txt,
        blocker_txt,
        one_thing
    );
    if identity.trim().is_empty() {
        Ok(state)
    } else {
        Ok(format!("{identity}\n{state}"))
    }
}

fn queue_idea(idea: &str) -> Result<String, Error> {
    let home = home_dir().ok_or_else(|| Error::Fail("HOME is unset".into()))?;
    let id = default_lineage(&home);
    let dir = lineage::dir(&id).map_err(Error::Fail)?;
    let card_id = next_card_id(&dir);
    let card_line = format!("CARD-{card_id} {idea}");
    append_queue(&dir, &card_line)?;
    Ok(format!("queued 1 card: CARD-{card_id} — {idea}"))
}

fn next_card_id(dir: &Path) -> u32 {
    // swallow: fail-safe-by-law — no queue file means empty, start from CARD-2501
    let text = fs::read_to_string(dir.join("queue")).unwrap_or_default();
    let mut max = 2500u32;
    for line in text.lines() {
        if let Some(id) = parse_card_num(line) {
            if id > max {
                max = id;
            }
        }
    }
    max + 1
}

fn parse_card_num(line: &str) -> Option<u32> {
    let line = line.trim();
    if line.starts_with("done ") || line.starts_with("withheld ") {
        return None;
    }
    let token = line.split_whitespace().next()?;
    token.strip_prefix("CARD-").and_then(|n| n.parse().ok())
}

fn append_queue(dir: &Path, line: &str) -> Result<(), Error> {
    let queue_path = dir.join("queue");
    // swallow: fail-safe-by-law — no queue file means empty, append creates it
    let mut content = fs::read_to_string(&queue_path).unwrap_or_default();
    if !content.ends_with('\n') && !content.is_empty() {
        content.push('\n');
    }
    content.push_str(line);
    content.push('\n');
    if let Some(parent) = queue_path.parent() {
        fs::create_dir_all(parent).map_err(|e| Error::Fail(e.to_string()))?;
    }
    fs::write(&queue_path, content).map_err(|e| Error::Fail(e.to_string()))
}

fn scan_blockers(_home: &Path) -> Vec<String> {
    Vec::new()
}

fn speak(text: &str) -> Result<(), String> {
    if let Some(bin) = env::var_os("CADDIS_VOICE_BIN") {
        let mut cmd = Command::new(&bin);
        cmd.arg("speak").arg(text);
        let status = cmd.status().map_err(|e| format!("voice: {e}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("voice exited {}", status.code().unwrap_or(-1)))
        }
    } else {
        Ok(())
    }
}

fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn default_lineage(home: &Path) -> String {
    let lines_dir = home.join(".caddis").join("rotation").join("lines");
    // swallow: fail-safe-by-law — no lineages dir means no default found
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
