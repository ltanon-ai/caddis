//! replay_report.rs — how a replay READS, kept apart from how it judges.
//!
//! SPLIT FROM `replay.rs` (CARD-0106): that file passed the 280-line cap once
//! the report learned to state its own coverage, and the two halves were always
//! separate jobs — re-judging rows against the current law, and rendering what
//! came back. The rendering is where honesty lives: a summary that hides what
//! it could not measure is the failure this whole engine exists against.

use caddis_warden::checks;
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
pub fn print_coverage(rows: u64, judged: u64, skipped: u64, reasons: &BTreeMap<&str, u64>) {
    let pct = if rows == 0 {
        0.0
    } else {
        (judged as f64) * 100.0 / (rows as f64)
    };
    println!("coverage: {judged} of {rows} rows re-judged ({pct:.1}%); {skipped} could not be");
    for (reason, count) in reasons {
        println!("  {count} {reason}");
    }
}

/// Itemize a list, and SAY how many were not shown rather than truncating in
/// silence.
pub fn print_capped(items: &[String], shown: usize) {
    for line in items.iter().take(shown) {
        println!("{line}");
    }
    if items.len() > shown {
        println!("... and {} more not shown", items.len() - shown);
    }
}

/// The REPLAY-COUNTS-1 summary: per law id, deny and steer fires over the
/// judged rows, then every REGISTERED law that never fired — coverage the
/// drift ratchet can read, never a claim that unfired means unnecessary.
pub fn print_law_fires(deny_fires: &BTreeMap<String, u64>, steer_fires: &BTreeMap<String, u64>) {
    let mut fired: BTreeSet<&str> = BTreeSet::new();
    for id in deny_fires.keys().chain(steer_fires.keys()) {
        fired.insert(id);
    }
    if fired.is_empty() {
        println!("law fires: none");
    } else {
        println!("law fires (current law over judged rows):");
        for id in &fired {
            println!(
                "  {id} deny={} steer={}",
                deny_fires.get(*id).copied().unwrap_or(0),
                steer_fires.get(*id).copied().unwrap_or(0)
            );
        }
    }
    let never: Vec<&str> = checks::registered_ids()
        .into_iter()
        .filter(|id| !fired.contains(id))
        .collect();
    println!("never fired: {}", never.join(", "));
}
