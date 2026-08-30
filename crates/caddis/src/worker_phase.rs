//! worker_phase.rs — CARD-0221. Task-line workflow + phase journal.
//! The mechanical spine worker v3 stands on: queue carries TASKS when
//! worker.cfg says `workflow=tasks`; every phase transition appends one
//! JSONL line to phases.log. No LLM in Rust. Execution organ lands next.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

/// True when `lines/<id>/worker.cfg` carries `workflow=tasks`.
pub(crate) fn tasks_mode(dir: &Path) -> bool {
    fs::read_to_string(dir.join("worker.cfg"))
        .map(|t| {
            t.lines()
                .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
                .any(|l| l.trim() == "workflow=tasks")
        })
        .unwrap_or(false)
}

/// Append one phase line: {"card":..,"phase":..,"ts":..}
pub(crate) fn journal(dir: &Path, card: &str, phase: &str) {
    let ts = crate::receipt::timestamp();
    let line = format!("{{\"card\":\"{card}\",\"phase\":\"{phase}\",\"ts\":\"{ts}\"}}\n");
    // swallow: best-effort-telemetry
    if let Ok(mut f) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("phases.log"))
    {
        let _ = writeln!(f, "{line}"); // swallow: best-effort-telemetry
    }
}

/// Last (card, phase) from phases.log; None when absent.
pub(crate) fn last(dir: &Path) -> Option<(String, String)> {
    let text = fs::read_to_string(dir.join("phases.log")).ok()?;
    let line = text.lines().filter(|l| !l.trim().is_empty()).next_back()?;
    let card = field(line, "card")?;
    let phase = field(line, "phase")?;
    Some((card, phase))
}

fn field(line: &str, key: &str) -> Option<String> {
    let pat = format!("\"{key}\":\"");
    let start = line.find(&pat)? + pat.len();
    let end = line[start..].find('"')? + start;
    Some(line[start..end].to_string())
}
