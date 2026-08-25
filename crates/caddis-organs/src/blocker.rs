//! blocker.rs — the blocker RECORD and its JSONL file, split out of
//! watchdog.rs under the 280-line law.
//!
//! The seam is ownership of a persisted fact: the watchdog decides WHEN a
//! service has failed enough to be flagged, and this module owns what a
//! blocker IS and how it survives a restart. A blocker outlives the process
//! that filed it — that is the whole point of writing it down — so its
//! encoding belongs with the file, not with the state machine.
//!
//! One JSONL line per blocker:
//! `{"source":"watchdog:<label>","reason":"...","ts":"..."}` — resolving is
//! deleting the line (the operator's act, or an automation with sanction).

use std::io::{self, Write};
use std::path::Path;

use crate::util::{json_escape, json_str_field};

/// One filed blocker (a self-flag the operator must resolve).
#[derive(Debug, Clone, PartialEq)]
pub struct Blocker {
    pub source: String,
    pub reason: String,
    pub ts: String,
}

impl Blocker {
    fn to_jsonl(&self) -> String {
        format!(
            "{{\"source\":\"{}\",\"reason\":\"{}\",\"ts\":\"{}\"}}",
            json_escape(&self.source),
            json_escape(&self.reason),
            json_escape(&self.ts)
        )
    }
}

/// Append a blocker line (best-effort file create).
pub(crate) fn file_blocker(path: &Path, b: &Blocker) -> io::Result<()> {
    use std::fs::OpenOptions;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut f = OpenOptions::new().create(true).append(true).open(path)?;
    f.write_all(b.to_jsonl().as_bytes())?;
    f.write_all(b"\n")
}

/// Read all open blockers from the JSONL file (absent file = none).
pub fn list_open_blockers(path: &Path) -> Vec<Blocker> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines().filter_map(parse_blocker_line).collect()
}

/// Minimal JSONL reader for the three-field blocker object.
fn parse_blocker_line(line: &str) -> Option<Blocker> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    Some(Blocker {
        source: json_str_field(line, "source")?,
        reason: json_str_field(line, "reason").unwrap_or_default(),
        ts: json_str_field(line, "ts").unwrap_or_default(),
    })
}

/// Delete every blocker line for `source`; returns the number removed.
/// Absent file = 0.
pub(crate) fn resolve_source(path: &Path, source: &str) -> io::Result<usize> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Ok(0);
    };
    let mut kept = String::new();
    let mut removed = 0;
    for line in text.lines() {
        let drop = parse_blocker_line(line)
            .map(|b| b.source == source)
            .unwrap_or(false);
        if drop {
            removed += 1;
        } else {
            kept.push_str(line);
            kept.push('\n');
        }
    }
    if removed > 0 {
        std::fs::write(path, kept)?;
    }
    Ok(removed)
}
