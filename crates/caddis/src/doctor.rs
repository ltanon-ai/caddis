//! doctor.rs — CARD-0307. Find, fix the safe set, escalate the rest.
//!
//! The operator's auto-repair loop: the find half is CARD-0306's talk
//! organ; this organ consumes it. Laws (quorum): every applied repair is
//! idempotent and logged as a `fix` turn WITH an evidence path; unsafe
//! repairs ESCALATE-ONLY; the doctor never answers findings it raised;
//! process-level actions (keeper restart) are report-only in v1.

use std::fs;
use std::path::Path;
use std::time::SystemTime;

use crate::hmac;
use crate::lease::write_atomic;

use crate::lineage;
use crate::receipt;

pub enum Error {
    Usage(String),
    Fail(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Usage(s) | Error::Fail(s) => write!(f, "{s}"),
        }
    }
}

pub fn run(args: &[String]) -> Result<(), Error> {
    let (id, rest) = lineage::take(args).map_err(Error::Usage)?;
    if let Some(a) = rest.first() {
        return Err(Error::Usage(format!("unknown argument {a}")));
    }
    let dir = lineage::dir(&id).map_err(Error::Fail)?;
    if !dir.join("arm.receipt").is_file() {
        return Err(Error::Fail(format!("lineage {id} has no arm receipt")));
    }
    println!("LINEAGE {id}");
    let fixed = usize::from(fix_stale_marker(&dir)?);
    escalate_unanswered(&dir)?;
    report_dead_keeper(&dir)?;
    println!("doctor: {fixed} fixed, rest escalated/reported");
    Ok(())
}

/// S1 (the only v1 FIX): a stale armed-never-woke marker WITH proof of
/// life is garbage — remove it, log the fix with the evidence path.
fn fix_stale_marker(dir: &Path) -> Result<bool, Error> {
    let marker = dir.join("armed-never-woke.lease");
    if !marker.is_file() || !dir.join("heartbeat.receipt").is_file() {
        return Ok(false);
    }
    let evidence = marker.display().to_string();
    fs::remove_file(&marker).map_err(|e| Error::Fail(format!("remove marker: {e}")))?;
    post_turn(
        dir,
        "fix",
        &format!("stale armed-never-woke cleared; wake proven {evidence}"),
    )?;
    println!("doctor: fixed stale-marker ({evidence})");
    Ok(true)
}

/// S2: unanswered findings escalate. The doctor NEVER answers findings.
fn escalate_unanswered(dir: &Path) -> Result<(), Error> {
    let open = count_unanswered(dir);
    if open == 0 {
        return Ok(());
    }
    let evidence = dir.join("talk/turns.jsonl").display().to_string();
    post_turn(
        dir,
        "escalate",
        &format!("operator attention: {open} unanswered finding(s) {evidence}"),
    )?;
    println!("doctor: escalate — {open} unanswered finding(s)");
    Ok(())
}

/// S3: a keeper whose bee.log is stale beyond the bound is dead —
/// report + escalate (restart stays the operator's call in v1).
fn report_dead_keeper(dir: &Path) -> Result<(), Error> {
    let log = dir.join("bee.log");
    if !log.is_file() {
        return Ok(());
    }
    let bound = std::env::var("CADDIS_DOCTOR_KEEPER_STALE_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(12600);
    let Ok(modified) = fs::metadata(&log).and_then(|m| m.modified()) else {
        return Ok(());
    };
    let Ok(age) = SystemTime::now().duration_since(modified) else {
        return Ok(());
    };
    if age.as_secs() < bound {
        return Ok(());
    }
    let (age_s, evidence) = (age.as_secs(), log.display().to_string());
    post_turn(
        dir,
        "escalate",
        &format!("beekeeper not cycling (bee.log {age_s}s stale) — restart needed {evidence}"),
    )?;
    println!("doctor: keeper dead (bee.log {age_s}s stale) — escalate");
    Ok(())
}

fn count_unanswered(dir: &Path) -> usize {
    let turns = fs::read_to_string(dir.join("talk/turns.jsonl")).unwrap_or_default();
    let mut open: usize = 0;
    for line in turns.lines() {
        if line.contains("\"kind\":\"finding\"") {
            open += 1;
        } else if line.contains("\"kind\":\"answer\"") || line.contains("\"kind\":\"fix\"") {
            open = open.saturating_sub(1);
        }
    }
    open
}

/// One HMAC-stamped JSONL turn (CARD-0306 wire shape; self-contained
/// until lease promotes to a root module and dedupes the writers).
fn post_turn(dir: &Path, kind: &str, text: &str) -> Result<(), Error> {
    let talk_dir = dir.join("talk");
    fs::create_dir_all(&talk_dir).map_err(|e| Error::Fail(format!("mkdir talk: {e}")))?;
    let pane = std::env::var("HERDR_PANE_ID").unwrap_or_default();
    let role = if pane.is_empty() { "past" } else { "present" };
    let ts = receipt::timestamp();
    let key = receipt::load_key(dir).unwrap_or_default();
    let mac = hmac::hmac_sha256(&key, format!("{role}|{pane}|{kind}|{text}|{ts}").as_bytes());
    let line = format!(
        "{{\"role\":\"{role}\",\"pane\":\"{pane}\",\"kind\":\"{kind}\",\"text\":\"{}\",\"ts\":\"{ts}\",\"mac\":\"{}\"}}\n",
        text.replace('\\', "\\\\").replace('"', "\\\""),
        receipt::hex_string(&mac)
    );
    let mut all = fs::read_to_string(talk_dir.join("turns.jsonl")).unwrap_or_default();
    all.push_str(&line);
    write_atomic(&talk_dir, "turns.jsonl", all.as_bytes())
        .map_err(Error::Fail)
        .map(|_| ())
}
