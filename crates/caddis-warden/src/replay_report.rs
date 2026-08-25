//! replay_report.rs — how a replay READS, kept apart from how it judges.
//!
//! SPLIT FROM `replay.rs` (CARD-0106): that file passed the 280-line cap once
//! the report learned to state its own coverage, and the two halves were always
//! separate jobs — re-judging rows against the current law, and rendering what
//! came back. The rendering is where honesty lives: a summary that hides what
//! it could not measure is the failure this whole engine exists against.
//!
//! EVERY RENDERER HERE IS A PURE FUNCTION RETURNING LINES, wrapped by a thin
//! printer (CARD-0107). A function whose only output is `println!` cannot be
//! asserted against in-process without capturing a global stream, so it gets
//! either no test or a test that asserts nothing — and `quality:test-health`
//! is right to call the latter what it is. The seam costs three wrappers and
//! makes the rendering, which is the part that can lie, directly checkable.

use crate::checks;
use std::collections::{BTreeMap, BTreeSet};

/// How many soft-finding changes are itemized before the list is summarised.
pub const DRIFT_SHOWN: usize = 10;

/// What fraction of the ledger this replay actually re-judged, and why the rest
/// could not be.
///
/// WITHOUT THIS LINE THE REPORT MISLEADS BY OMISSION: "new-denies: 0" over a
/// ledger where seven eighths of the rows were unreadable looks exactly like
/// "no law change would have denied anything you did", and a reader has no way
/// to tell the two apart from a bare skip count.
pub fn coverage_lines(
    rows: u64,
    judged: u64,
    skipped: u64,
    reasons: &BTreeMap<&str, u64>,
) -> Vec<String> {
    let pct = if rows == 0 {
        0.0
    } else {
        (judged as f64) * 100.0 / (rows as f64)
    };
    let mut out = vec![format!(
        "coverage: {judged} of {rows} rows re-judged ({pct:.1}%); {skipped} could not be"
    )];
    out.extend(reasons.iter().map(|(r, c)| format!("  {c} {r}")));
    out
}

pub fn print_coverage(rows: u64, judged: u64, skipped: u64, reasons: &BTreeMap<&str, u64>) {
    print_all(&coverage_lines(rows, judged, skipped, reasons));
}

/// Itemize a list, and SAY how many were not shown rather than truncating in
/// silence.
pub fn capped_lines(items: &[String], shown: usize) -> Vec<String> {
    let mut out: Vec<String> = items.iter().take(shown).cloned().collect();
    if items.len() > shown {
        out.push(format!("... and {} more not shown", items.len() - shown));
    }
    out
}

pub fn print_capped(items: &[String], shown: usize) {
    print_all(&capped_lines(items, shown));
}

/// The REPLAY-COUNTS-1 summary: per law id, deny and steer fires over the
/// judged rows, then every REGISTERED law that never fired — coverage the
/// drift ratchet can read, never a claim that unfired means unnecessary.
pub fn law_fire_lines(
    deny_fires: &BTreeMap<String, u64>,
    steer_fires: &BTreeMap<String, u64>,
) -> Vec<String> {
    let mut fired: BTreeSet<&str> = BTreeSet::new();
    for id in deny_fires.keys().chain(steer_fires.keys()) {
        fired.insert(id);
    }
    let mut out = Vec::new();
    if fired.is_empty() {
        out.push("law fires: none".to_string());
    } else {
        out.push("law fires (current law over judged rows):".to_string());
        for id in &fired {
            out.push(format!(
                "  {id} deny={} steer={}",
                deny_fires.get(*id).copied().unwrap_or(0),
                steer_fires.get(*id).copied().unwrap_or(0)
            ));
        }
    }
    let never: Vec<&str> = checks::registered_ids()
        .into_iter()
        .filter(|id| !fired.contains(id))
        .collect();
    out.push(format!("never fired: {}", never.join(", ")));
    out
}

pub fn print_law_fires(deny_fires: &BTreeMap<String, u64>, steer_fires: &BTreeMap<String, u64>) {
    print_all(&law_fire_lines(deny_fires, steer_fires));
}

fn print_all(lines: &[String]) {
    for line in lines {
        println!("{line}");
    }
}

#[cfg(test)]
#[path = "replay_report_tests.rs"]
mod tests;
