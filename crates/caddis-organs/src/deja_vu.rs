//! deja_vu.rs — CARD-0245. Cross-session attention replay.
//!
//! The "memory layers" doctrine (2KB laws → gBrain pointers → vault bulk)
//! has never been measured. Every session's observe.jsonl already records
//! what was actually injected into context (`kind:"context"` events carry
//! a `facts` array of content hashes) and what the model did with those
//! facts (`kind:"cite"` events). This organ replays every trail in a
//! project dir, aggregates those signals, and surfaces the facts nobody
//! cited — the A/B candidates for the host to strip from the injection
//! layer.
//!
//! The organ PROPOSES; the host DISPOSES. `caddis page ab --session <id>
//! --strip <fact>` is the host's verb. We never strip ourselves, never
//! touch the injection layer, never reach across crates.
//!
//! Zero deps. JSONL is OURS (the observe nerve writes it), so minimal
//! substring readers — same house style as `page_report_tally.rs`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// A fact key is a content-hash string identifying an injected memory fact.
/// Laws, gBrain pointers, vault pointers — all share this shape: a stable
/// string the injection layer uses to dedupe and the model uses to cite.
pub type FactKey = String;

/// One fact's aggregated attention across the trails we scanned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FactStats {
    pub fact_key: FactKey,
    pub sessions_seen: u64,
    pub citations: u64,
    pub tokens_burned: u64,
    /// Trail mtime (millis since epoch) of the most recent trail that
    /// referenced this fact. Used by `dead_weight` to honor the window.
    pub last_seen_ms: u64,
}

/// The whole map: every fact we saw, keyed by content hash. BTreeMap so
/// the output is deterministic for ledger diffs.
#[derive(Debug, Default, Clone)]
pub struct AttentionMap {
    pub facts: BTreeMap<FactKey, FactStats>,
}

/// Minimal JSON string-field reader — same shape as `page_report_tally`.
fn json_str<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let pat = format!("\"{key}\":\"");
    let start = line.find(&pat)? + pat.len();
    let end = line[start..].find('"')? + start;
    Some(&line[start..end])
}

/// Minimal JSON number reader. Stops at the first non-digit.
fn json_num(line: &str, key: &str) -> Option<u64> {
    let pat = format!("\"{key}\":");
    let start = line.find(&pat)? + pat.len();
    let digits: String = line[start..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

/// Parse a JSON array of strings (`"facts":["a","b","c"]`). Zero-dep
/// substring walker — the shape is ours, the contents are hash strings.
fn json_facts(line: &str, key: &str) -> Vec<String> {
    let pat = format!("\"{key}\":[");
    let Some(start) = line.find(&pat) else {
        return Vec::new();
    };
    let body = &line[start + pat.len()..];
    let end = body.find(']').unwrap_or(body.len());
    let arr = &body[..end];
    let mut out = Vec::new();
    let mut rest = arr;
    while let Some(i) = rest.find('"') {
        rest = &rest[i + 1..];
        let Some(j) = rest.find('"') else { break };
        out.push(rest[..j].to_string());
        rest = &rest[j + 1..];
    }
    out
}

/// Trail mtime in millis since epoch. 0 if the file vanished between
/// `read_dir` and `metadata` — fail-safe.
fn trail_mtime_ms(path: &Path) -> u64 {
    let Ok(meta) = fs::metadata(path) else {
        return 0;
    };
    let Ok(mtime) = meta.modified() else { return 0 };
    let dur = mtime.duration_since(UNIX_EPOCH).unwrap_or_default();
    dur.as_millis() as u64
}

/// Per-line outcome: a fact appeared (with token share) or was cited.
enum TrailHit {
    Injected { fact: FactKey, tokens: u64 },
    Cited { fact: FactKey },
}

/// Read one observe.jsonl trail into a vector of hits. Missing file is
/// empty (fail-safe). Malformed lines are skipped.
fn read_trail(path: &Path) -> Vec<TrailHit> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        match json_str(line, "kind") {
            Some("context") => {
                let stored = json_num(line, "stored_tokens").unwrap_or(0);
                let facts = json_facts(line, "facts");
                let n = facts.len() as u64;
                let share = if n == 0 { 0 } else { stored / n };
                for f in facts {
                    out.push(TrailHit::Injected {
                        fact: f,
                        tokens: share,
                    });
                }
            }
            Some("cite") => {
                if let Some(f) = json_str(line, "fact") {
                    out.push(TrailHit::Cited {
                        fact: f.to_string(),
                    });
                }
            }
            _ => {}
        }
    }
    out
}

/// Build the attention map from a list of observe trails (one per
/// session). Caller picks which trails are in scope; the map itself
/// carries no time filter — `dead_weight` applies the window.
pub fn build(trails: &[PathBuf]) -> AttentionMap {
    let mut map = AttentionMap::default();
    for trail in trails {
        let seen_ms = trail_mtime_ms(trail);
        let mut seen_in_trail: BTreeMap<FactKey, ()> = BTreeMap::new();
        for hit in read_trail(trail) {
            match hit {
                TrailHit::Injected { fact, tokens } => {
                    let entry = map.facts.entry(fact.clone()).or_insert_with(|| FactStats {
                        fact_key: fact.clone(),
                        sessions_seen: 0,
                        citations: 0,
                        tokens_burned: 0,
                        last_seen_ms: 0,
                    });
                    entry.tokens_burned = entry.tokens_burned.saturating_add(tokens);
                    if seen_ms > entry.last_seen_ms {
                        entry.last_seen_ms = seen_ms;
                    }
                    if seen_in_trail.insert(fact.clone(), ()).is_none() {
                        entry.sessions_seen = entry.sessions_seen.saturating_add(1);
                    }
                }
                TrailHit::Cited { fact } => {
                    let entry = map.facts.entry(fact.clone()).or_insert_with(|| FactStats {
                        fact_key: fact.clone(),
                        sessions_seen: 0,
                        citations: 0,
                        tokens_burned: 0,
                        last_seen_ms: 0,
                    });
                    entry.citations = entry.citations.saturating_add(1);
                    if seen_ms > entry.last_seen_ms {
                        entry.last_seen_ms = seen_ms;
                    }
                }
            }
        }
    }
    map
}

/// Surface facts nobody cited within the lookback window. The window
/// is "facts last touched at most `window_ms` before the most recent
/// observation in the map" — pass `u64::MAX` to include all facts.
pub fn dead_weight(map: &AttentionMap, window_ms: u64) -> Vec<FactKey> {
    let now_ms = map
        .facts
        .values()
        .map(|f| f.last_seen_ms)
        .max()
        .unwrap_or(0);
    let cutoff = now_ms.saturating_sub(window_ms);
    let mut out: Vec<FactKey> = map
        .facts
        .values()
        .filter(|f| f.citations == 0 && f.last_seen_ms >= cutoff)
        .map(|f| f.fact_key.clone())
        .collect();
    out.sort();
    out
}

/// The never-strip list. The host MUST consult this before any `--strip`
/// action — constitution-tier laws, secrets pointers, and the safety
/// gates themselves are not evictable. Fail-closed: anything not on this
/// list is the host's call; anything that IS on this list is a veto.
pub fn never_strip_list() -> &'static [&'static str] {
    &[
        "law:constitution",
        "law:safety_gate",
        "vault:secrets_pointer",
        "warden:never_strip_marker",
    ]
}

/// Returns true if `fact_key` is on the never-strip list. The host
/// calls this before any `caddis page ab --strip` invocation.
pub fn is_protected(fact_key: &str) -> bool {
    never_strip_list().iter().any(|p| fact_key.starts_with(p))
}
