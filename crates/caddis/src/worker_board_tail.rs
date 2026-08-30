//! worker_board_tail.rs — the board's TAIL READERS + the live EVENT
//! FEED merge, split out of worker_board_state.rs under the 280-line
//! law (CARD-0217 + gate size law). Same organ, same law: read-only,
//! never spawns, never writes. Re-exported from `worker_board_state`
//! so no caller path moved.

use std::fs;
use std::path::Path;

use crate::worker_board_state::{jnum, jstr};

pub fn phase_last(dir: &Path) -> Option<(String, String)> {
    crate::worker_phase::last(dir)
}

pub fn tool_counts(dir: &Path) -> Vec<(String, usize)> {
    let text = fs::read_to_string(dir.join("bee.log")).unwrap_or_default();
    let mut counts: std::collections::BTreeMap<String, usize> = Default::default();
    for line in text.lines() {
        if let Some(a) = jstr(line, "argv0") {
            *counts.entry(a).or_default() += 1;
        }
    }
    counts.into_iter().collect()
}

pub fn scan_live_last(dir: &Path) -> Option<String> {
    let text = fs::read_to_string(dir.join("scan.live")).ok()?;
    let line = text.lines().filter(|l| !l.trim().is_empty()).next_back()?;
    let check = jstr(line, "check")?;
    let state = jstr(line, "state")?;
    Some(format!("{check} {state}"))
}

// ─── CARD-0243: the live EVENT FEED ──────────────────────────────────

/// One merged event. Sources are the lineage's own journals — the
/// operator sees what actually ran, newest first.
pub struct FeedEvent {
    pub key: String,
    pub text: String,
}

/// Last `n` events merged across bee.log, phases.log, scan.live and
/// the eddy tick trail (eddy.jsonl, CARD-0237).
pub fn last_events(dir: &Path, n: usize) -> Vec<FeedEvent> {
    let mut all: Vec<FeedEvent> = Vec::new();
    for line in read_lines(dir.join("bee.log")) {
        if let Some(e) = bee_event(&line) {
            all.push(e);
        }
    }
    for line in read_lines(dir.join("phases.log")) {
        if let Some(e) = phase_event(&line) {
            all.push(e);
        }
    }
    for line in read_lines(dir.join("scan.live")) {
        if let Some(e) = scan_event(&line) {
            all.push(e);
        }
    }
    for tick in eddy_trail(dir) {
        all.push(FeedEvent {
            key: eddy_sort_key(&tick),
            text: format!("{} eddy {}", eddy_hms(&tick), tick.status_class.as_str()),
        });
    }
    all.sort_by(|a, b| b.key.cmp(&a.key));
    all.truncate(n);
    all
}

/// The lineage's eddy trail (eddy.jsonl), file order — the loop organ's
/// judgement input for this lineage (CARD-0237).
pub fn eddy_trail(dir: &Path) -> Vec<caddis_organs::eddy::Tick> {
    caddis_organs::eddy::read_ticks(&dir.join("eddy.jsonl"))
}

fn read_lines(p: std::path::PathBuf) -> Vec<String> {
    std::fs::read_to_string(p)
        .map(|t| t.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

fn bee_event(line: &str) -> Option<FeedEvent> {
    let card = jstr(line, "card")?;
    let exit = jnum(line, "exit").unwrap_or(u64::MAX);
    let ts = jstr(line, "ts").unwrap_or_default();
    let argv0 = jstr(line, "argv0").unwrap_or_default();
    Some(FeedEvent {
        key: format!("{ts}|bee|{card}"),
        text: format!(
            "{} bee {} {} exit={}",
            hms_local(&ts),
            card,
            argv0,
            exit as i64
        ),
    })
}

fn phase_event(line: &str) -> Option<FeedEvent> {
    let card = jstr(line, "card")?;
    let phase = jstr(line, "phase").unwrap_or_default();
    let ts = jstr(line, "ts").unwrap_or_default();
    Some(FeedEvent {
        key: format!("{ts}|phase|{card}"),
        text: format!("{} phase {} {}", hms_local(&ts), card, phase),
    })
}

fn scan_event(line: &str) -> Option<FeedEvent> {
    let check = jstr(line, "check")?;
    let state = jstr(line, "state").unwrap_or_default();
    let ts = jstr(line, "ts").unwrap_or_default();
    Some(FeedEvent {
        key: format!("{ts}|scan|{check}"),
        text: format!("{} scan {} {}", hms_local(&ts), check, state),
    })
}

fn eddy_sort_key(t: &caddis_organs::eddy::Tick) -> String {
    // ISO-form key so eddy ticks sort AGAINST journal ts strings, not
    // beside them (same format family as bee/phase/scan rows).
    format!(
        "{}Z|eddy|{:016x}",
        caddis_organs::util::iso8601_from_unix((t.ts_ms / 1000) as i64),
        t.payload_hash
    )
}

fn eddy_hms(t: &caddis_organs::eddy::Tick) -> String {
    hms_local(&caddis_organs::util::iso8601_from_unix(
        (t.ts_ms / 1000) as i64,
    ))
}

/// TS -> "HH:MM:SS" in Europe/Vilnius (operator order 2026-08-29:
/// the board shows Lithuanian time). Accepts unix-seconds strings
/// (bee/phase/scan journal rows) and ISO-UTC (eddy rows); anything
/// else degrades to the old head-slice. Keys/sorting stay UTC.
pub(crate) fn hms_local(ts: &str) -> String {
    let utc = if ts.chars().all(|c| c.is_ascii_digit()) && !ts.is_empty() {
        ts.parse::<i64>().ok()
    } else {
        caddis_organs::util::unix_from_iso8601(ts)
    };
    match utc {
        Some(secs) => {
            let iso = caddis_organs::util::iso8601_from_unix_vilnius(secs);
            iso[11..19].to_string()
        }
        None => hms(ts),
    }
}

/// ISO ts -> "HH:MM:SS" (best effort; raw head when unparseable).
fn hms(iso: &str) -> String {
    if iso.len() >= 19 && iso.as_bytes().get(10) == Some(&b'T') {
        iso[11..19].to_string()
    } else {
        iso.chars().take(19).collect()
    }
}
