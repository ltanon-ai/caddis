//! drain.rs — per-kind drain (CARD-0120) + lineage (CARD-0133).
//!
//! CADDIS_DRAIN_* fixtures keep tests hermetic. Omp production does not
//! require ~/.herdr. ARM pane= scopes live agents to this rotation.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub enum DrainResult {
    Clean,
    LiveAgent(String),
    Unknown(String),
}

pub fn drain(kind: &str, pane: Option<&str>) -> DrainResult {
    match kind {
        "omp" => drain_herdr(pane),
        "claude" => drain_claude(),
        "qpi" => drain_qpi(),
        _ => DrainResult::Unknown(format!("unknown kind {kind}")),
    }
}

fn drain_herdr(pane: Option<&str>) -> DrainResult {
    if let Some(path) = env::var_os("CADDIS_DRAIN_HERDR") {
        return check_evidence_file(&PathBuf::from(path));
    }
    if let Some(path) = env::var_os("CADDIS_HERDR_SNAPSHOT") {
        return check_snapshot(&PathBuf::from(path), pane);
    }
    live_pane_list(pane)
}

/// CARD-0310: production truth is the LIVE daemon (`herdr pane list`,
/// the same view the operator sees). %APPDATA%/herdr/session.json is
/// the desktop client's layout persistence — schema v3 carries no
/// pane_id keys, lacks live panes and keeps stale ones — presence
/// against it can never gate. Unreachable herdr = Unknown, never Clean.
fn live_pane_list(pane: Option<&str>) -> DrainResult {
    match crate::which::herdr(&["pane", "list"]) {
        Some(text) => check_pane_text(&text, pane),
        None => DrainResult::Unknown("herdr unreachable — cannot prove predecessor gone".into()),
    }
}

fn check_snapshot(path: &Path, pane: Option<&str>) -> DrainResult {
    // CARD-0300: a stale state source is Unknown, never Clean — E1 must
    // not be reborn one layer up via a frozen snapshot.
    if let Some(why) = staleness(path) {
        return DrainResult::Unknown(why);
    }
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => return DrainResult::Unknown("snapshot not readable".into()),
    };
    check_pane_text(&text, pane)
}

/// The shared presence verdict: a pane id in pane-list-shaped text at
/// ANY agent_status is a live agent (CARD-0300); no arm pane = Unknown.
fn check_pane_text(text: &str, pane: Option<&str>) -> DrainResult {
    let Some(pane) = pane.filter(|p| !p.is_empty()) else {
        return DrainResult::Unknown("arm has no pane".into());
    };
    if pane_present(text, pane) {
        DrainResult::LiveAgent(format!("live agent in pane {pane}"))
    } else {
        DrainResult::Clean
    }
}

/// CARD-0300: mtime age over the bound names staleness; so does a future
/// mtime (clock skew — fail closed either way). Bound 0 = always stale.
fn staleness(path: &Path) -> Option<String> {
    let bound = env::var("CADDIS_DRAIN_FRESHNESS_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(120);
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    match SystemTime::now().duration_since(modified) {
        Ok(age) if age.as_secs() >= bound => Some(format!(
            "stale herdr state ({}s >= {bound}s bound)",
            age.as_secs()
        )),
        Ok(_) => None,
        Err(_) => Some("herdr state mtime is in the future (clock skew)".into()),
    }
}

/// CARD-0300: pane PRESENCE at any agent_status is live — E1 drained an
/// idle predecessor Clean and verify restamped over it. Structural
/// quoted-value equality: no substring prefix lies, no first-`}`
/// truncation — a nested object cannot hide a live pane.
fn pane_present(text: &str, pane: &str) -> bool {
    let mut rest = text;
    while let Some(i) = rest.find("\"pane_id\"") {
        rest = &rest[i + "\"pane_id\"".len()..];
        if let Some(value) = quoted_value(rest) {
            if value == pane {
                return true;
            }
        }
    }
    false
}

/// The JSON string literal that must follow a key token: optional
/// whitespace, `:`, whitespace, then a quoted string (escape-aware).
fn quoted_value(mut s: &str) -> Option<String> {
    s = s.trim_start().strip_prefix(':')?.trim_start();
    let mut out = String::new();
    let mut chars = s.strip_prefix('"')?.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => out.push(chars.next()?),
            _ => out.push(c),
        }
    }
    None
}

fn drain_claude() -> DrainResult {
    if let Some(path) = env::var_os("CADDIS_DRAIN_CLAUDE_REGISTRY") {
        return check_evidence_file(&PathBuf::from(path));
    }
    let Some(home) = home_dir() else {
        return DrainResult::Unknown("HOME unset".into());
    };
    let claude_dir = home.join(".claude");
    if !claude_dir.is_dir() {
        return DrainResult::Unknown("claude registry not found".into());
    }
    scan_dir_for_live(&claude_dir)
}

fn drain_qpi() -> DrainResult {
    if let Some(path) = env::var_os("CADDIS_DRAIN_QPI") {
        return check_evidence_file(&PathBuf::from(path));
    }
    let Some(home) = home_dir() else {
        return DrainResult::Unknown("HOME unset".into());
    };
    let qpi_dir = home.join(".qpi");
    if !qpi_dir.is_dir() {
        return DrainResult::Unknown("qpi source not found".into());
    }
    scan_dir_for_live(&qpi_dir)
}

fn check_evidence_file(path: &Path) -> DrainResult {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => return DrainResult::Unknown("evidence file not readable".into()),
    };
    if has_live_agent(&text) {
        return DrainResult::LiveAgent("live agent in evidence".into());
    }
    DrainResult::Clean
}

fn scan_dir_for_live(dir: &Path) -> DrainResult {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return DrainResult::Unknown("cannot read source dir".into()),
    };
    for entry in entries {
        // swallow: best-effort-cleanup
        let Ok(entry) = entry else { continue };
        let p = entry.path();
        // swallow: best-effort-cleanup
        if let Ok(text) = fs::read_to_string(&p) {
            if has_live_agent(&text) {
                return DrainResult::LiveAgent(format!("live agent in {}", p.display()));
            }
        }
    }
    DrainResult::Clean
}

fn has_live_agent(text: &str) -> bool {
    for line in text.lines() {
        let l = line.trim().to_ascii_lowercase();
        if l == "live" || l.contains("status=live") {
            return true;
        }
        if l.contains("\"status\": \"live\"") || l.contains("\"alive\": true") {
            return true;
        }
    }
    false
}

fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}
