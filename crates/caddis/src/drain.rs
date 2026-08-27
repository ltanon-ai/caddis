//! drain.rs — per-kind drain (CARD-0120) + lineage (CARD-0133).
//!
//! CADDIS_DRAIN_* fixtures keep tests hermetic. Omp production does not
//! require ~/.herdr. ARM pane= scopes live agents to this rotation.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

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
    if let Some(home) = herdr_home() {
        let session = home.join("session.json");
        if session.is_file() {
            return check_snapshot(&session, pane);
        }
        return scan_dir_for_live(&home);
    }
    DrainResult::Unknown("herdr source not found".into())
}

fn herdr_home() -> Option<PathBuf> {
    if let Some(p) = env::var_os("CADDIS_HERDR_HOME") {
        let p = PathBuf::from(p);
        if p.is_dir() {
            return Some(p);
        }
    }
    if let Some(app) = env::var_os("APPDATA") {
        let p = PathBuf::from(app).join("herdr");
        if p.is_dir() {
            return Some(p);
        }
    }
    let home = home_dir()?;
    let p = home.join(".herdr");
    if p.is_dir() {
        Some(p)
    } else {
        None
    }
}

fn check_snapshot(path: &Path, pane: Option<&str>) -> DrainResult {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => return DrainResult::Unknown("snapshot not readable".into()),
    };
    let Some(pane) = pane.filter(|p| !p.is_empty()) else {
        return DrainResult::Unknown("arm has no pane".into());
    };
    if pane_is_working(&text, pane) {
        DrainResult::LiveAgent(format!("live agent in pane {pane}"))
    } else {
        DrainResult::Clean
    }
}

fn pane_is_working(text: &str, pane: &str) -> bool {
    let a = format!("\"pane_id\":\"{pane}\"");
    let b = format!("\"pane_id\": \"{pane}\"");
    for n in [a.as_str(), b.as_str()] {
        if let Some(i) = text.find(n) {
            let rest = &text[i..];
            let end = rest.find('}').unwrap_or(rest.len());
            let obj = &rest[..end];
            if obj.contains("\"agent_status\":\"working\"")
                || obj.contains("\"agent_status\": \"working\"")
            {
                return true;
            }
        }
    }
    false
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
