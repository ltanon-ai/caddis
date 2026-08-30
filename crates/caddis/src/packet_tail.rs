//! packet_tail.rs — CARD-0257. Assembles the valence body-state TAIL
//! from lineage telemetry, fail-soft to zeroed senses when a source is
//! absent (a fresh lineage is a normal state, never an error).
//!
//! Telemetry sources (named by valence_senses.rs):
//! - eddy.jsonl ticks  → the eddy trail (caddis_organs::eddy::read_ticks)
//! - bee.log           → BoardTick list (card + exit)
//! - scan.live         → GateCensus (test_count = scan line count)
//! - <session>.observe.jsonl → PagerObserve (stored_pct from the last
//!   context line; session defaults to the lineage id)
//!
//! TreePulse has no lineage file today — default zeroed (fail-soft).

use std::fs;
use std::path::{Path, PathBuf};

use caddis_organs::eddy;
use caddis_organs::valence::{
    body_state, render_tail, BoardTick, GateCensus, PagerObserve, TreePulse,
};

/// Assemble the valence tail block for the given lineage dir + home.
/// Returns the rendered tail string (one line, ~60 tokens).
pub fn tail(dir: &Path, home: &Path, lineage: &str) -> String {
    let ticks = eddy::read_ticks(&dir.join("eddy.jsonl"));
    let gates = gate_census(dir);
    let pager = pager_observe(home, lineage);
    let board = board_ticks(dir);
    let tree = TreePulse {
        goal_urgency: 0,
        pace_verdict: "unknown".into(),
    };
    let state = body_state(&ticks, &gates, &pager, &board, &tree);
    render_tail(&state)
}

/// GateCensus from scan.live: test_count = the number of scan lines
/// (each is one check result). file_size and max_ccn are zeroed —
/// the gate census file is not a lineage artifact today, so fail-soft.
fn gate_census(dir: &Path) -> GateCensus {
    let text = fs::read_to_string(dir.join("scan.live")).unwrap_or_default();
    let test_count = text.lines().filter(|l| !l.trim().is_empty()).count() as u32;
    GateCensus {
        file_size: 0,
        max_ccn: 0,
        test_count,
    }
}

/// BoardTick list from bee.log: one per line, card + exit.
fn board_ticks(dir: &Path) -> Vec<BoardTick> {
    let text = fs::read_to_string(dir.join("bee.log")).unwrap_or_default();
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|line| {
            let card = jstr(line, "card")?;
            let exit = jnum(line, "exit").map(|n| n as i64).unwrap_or(0);
            Some(BoardTick { card, exit })
        })
        .collect()
}

/// PagerObserve from the session's observe JSONL: stored_pct from the
/// last `context` line. Session defaults to the lineage id (same key the
/// board uses). Absent = zeroed.
fn pager_observe(home: &Path, lineage: &str) -> PagerObserve {
    let path: PathBuf = home
        .join(".caddis")
        .join("pager")
        .join(format!("{lineage}.observe.jsonl"));
    let text = fs::read_to_string(&path).unwrap_or_default();
    let mut pct = 0u64;
    let mut tokens = 0u64;
    let mut evicted = 0u64;
    for line in text.lines() {
        if jstr(line, "kind").as_deref() == Some("context") {
            pct = jnum(line, "stored_pct").unwrap_or(pct);
            tokens = jnum(line, "stored_tokens").unwrap_or(tokens);
        }
        if jstr(line, "kind").as_deref() == Some("project") {
            evicted = jnum(line, "n_evicted").unwrap_or(evicted);
        }
    }
    PagerObserve {
        stored_pct: pct,
        stored_tokens: tokens,
        evicted,
    }
}

/// Minimal JSON string field extractor (same shape as worker_board_state).
fn jstr(line: &str, key: &str) -> Option<String> {
    let pat = format!("\"{key}\":\"");
    let start = line.find(&pat)? + pat.len();
    let end = line[start..].find('"')? + start;
    Some(line[start..end].to_string())
}

/// Minimal JSON number field extractor (same shape as worker_board_state).
fn jnum(line: &str, key: &str) -> Option<u64> {
    let pat = format!("\"{key}\":");
    let start = line.find(&pat)? + pat.len();
    let digits: String = line[start..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}
