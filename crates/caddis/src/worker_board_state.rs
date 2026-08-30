//! worker_board_state.rs — CARD-0217. Read-only state gathering for the
//! board organ. Board never spawns, never writes, never drain::.
//! Only artifacts production actually writes: queue, arm.receipt,
//! pace.line, bee.log, fold.at, fold.state, cold.store, observe JSONL.

use std::fs;
use std::path::{Path, PathBuf};

use crate::hmac;
use crate::receipt;

pub struct Arm {
    pub kind: String,
    pub model: String,
    pub pane: String,
    pub pace: String,
}

pub struct Queue {
    pub remaining: Vec<String>,
    pub done: usize,
}

pub struct Bee {
    pub card: String,
    pub argv0: String,
    pub exit: i64,
    pub ts: String,
}

pub struct Page {
    pub session: String,
    pub mode: String,
    pub mark: String,
    pub cold: usize,
    pub stored: Option<u64>,
    pub sent: Option<u64>,
    pub pct: Option<u64>,
    pub window: Option<u64>,
    pub stubbed: Option<u64>,
    pub evicted: Option<u64>,
    pub over: bool,
    pub usage: Vec<(String, u64)>,
}

pub struct Scan {
    pub verdict: String,
    pub checks: Vec<(String, bool)>,
}

pub fn arm_fields(dir: &Path) -> Arm {
    let bytes = fs::read(dir.join("arm.receipt")).unwrap_or_default();
    let f = |n: &str| receipt::extract_field(&bytes, n).unwrap_or_default();
    Arm {
        kind: f("kind"),
        model: f("model"),
        pane: f("pane"),
        pace: if f("pace").is_empty() {
            "run".into()
        } else {
            f("pace")
        },
    }
}

pub fn pace_sentence(dir: &Path) -> Option<String> {
    let bytes = fs::read(dir.join("pace.line")).ok()?;
    let (body, mac) = receipt::split_receipt(&bytes)?;
    let key = receipt::load_key(dir).ok()?;
    if hmac::hmac_sha256(&key, body) != mac {
        return None; // tampered → caller prints unverified
    }
    receipt::extract_field(body, "sentence")
}

pub fn queue(dir: &Path) -> Queue {
    let mut remaining = Vec::new();
    let mut done = 0usize;
    // swallow: fail-safe-by-law
    if let Ok(text) = fs::read_to_string(dir.join("queue")) {
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line.starts_with("done ") || line.starts_with("withheld ") {
                done += 1;
            } else {
                remaining.push(line.to_string());
            }
        }
    }
    Queue { remaining, done }
}

pub fn bee_recent(dir: &Path, n: usize) -> Vec<Bee> {
    let text = fs::read_to_string(dir.join("bee.log")).unwrap_or_default();
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .rev()
        .take(n)
        .filter_map(|line| {
            Some(Bee {
                card: jstr(line, "card")?,
                argv0: jstr(line, "argv0")?,
                exit: jnum(line, "exit").unwrap_or(u64::MAX) as i64,
                ts: jstr(line, "ts").unwrap_or_default(),
            })
        })
        .collect()
}

pub fn fold_state(home: &Path, dir: &Path) -> (u64, &'static str) {
    let at = fs::read_to_string(home.join(".caddis").join("fold.at"))
        .ok()
        .and_then(|t| t.trim().parse().ok())
        .filter(|n| (1..=99).contains(n))
        .unwrap_or(50);
    let state = if dir.join("fold.state").is_file() {
        "warned"
    } else {
        "quiet"
    };
    (at, state)
}

/// Session defaults to the LINEAGE id (same key), never newest-mtime stem.
pub fn page(home: &Path, lineage: &str, session: Option<&str>) -> Page {
    let session = session.unwrap_or(lineage).to_string();
    let pager: PathBuf = home.join(".caddis").join("pager");
    let sdir: PathBuf = pager.join(&session);
    let mut p = Page {
        session: session.clone(),
        mode: "observe".into(),
        mark: String::new(),
        cold: 0,
        stored: None,
        sent: None,
        pct: None,
        window: None,
        stubbed: None,
        evicted: None,
        over: false,
        usage: Vec::new(),
    };
    if !sdir.is_dir() {
        return p; // absent session: session=<name>, zeros — honest
    }
    // swallow: fail-safe-by-law
    if let Ok(m) = fs::read_to_string(sdir.join("mode")) {
        p.mode = if m.trim() == "page" {
            "page".into()
        } else {
            "observe".into()
        };
    }
    // swallow: fail-safe-by-law
    if let Ok(m) = fs::read_to_string(sdir.join("mark")) {
        p.mark = m.trim().to_string();
    }
    // swallow: fail-safe-by-law
    if let Ok(text) = fs::read_to_string(sdir.join("cold.store")) {
        p.cold = text.split("---\n").filter(|b| b.contains("seq=")).count();
    }
    // swallow: fail-safe-by-law
    if let Ok(log) = fs::read_to_string(pager.join(format!("{session}.observe.jsonl"))) {
        for line in log.lines() {
            match jstr(line, "kind").as_deref() {
                Some("context") => apply_context_line(line, &mut p),
                Some("project") => p.evicted = jnum(line, "n_evicted").or(p.evicted),
                Some("message_end") => {
                    if line.contains("\"usage\":{") {
                        p.usage = usage_pairs(line);
                    }
                }
                _ => {}
            }
        }
    }
    p
}

/// CARD-0247: parse a single `"kind":"context"` line into the Page.
/// Stickiness: `over` only ever grows true (never resets mid-loop),
/// so a single true line trips it permanently for this page.
fn apply_context_line(line: &str, p: &mut Page) {
    p.stored = jnum(line, "stored_tokens").or(p.stored);
    p.sent = jnum(line, "sent_est_tokens").or(p.sent);
    p.pct = jnum(line, "stored_pct").or(p.pct);
    p.window = jnum(line, "stored_window").or(p.window);
    p.stubbed = jnum(line, "n_stubbed").or(p.stubbed);
    if line.contains("\"stored_over\":true") {
        p.over = true;
    }
}

pub fn scan_last(dir: &Path) -> Option<Scan> {
    let text = fs::read_to_string(dir.join("scan.log")).ok()?;
    let line = text.lines().filter(|l| !l.trim().is_empty()).next_back()?;
    let mut verdict = "pass".to_string();
    let mut checks = Vec::new();
    for name in ["fmt", "clippy", "test", "census"] {
        let pat = format!("\"{name}\":\"");
        if let Some(i) = line.find(&pat) {
            let ok = line[i + pat.len()..].starts_with("pass");
            if !ok {
                verdict = "fail".into();
            }
            checks.push((name.to_string(), ok));
        }
    }
    Some(Scan { verdict, checks })
}

fn usage_pairs(line: &str) -> Vec<(String, u64)> {
    let start = match line.find("\"usage\":{") {
        Some(i) => i + 8,
        None => return Vec::new(),
    };
    let end = line[start..]
        .find('}')
        .map(|e| start + e)
        .unwrap_or(line.len());
    let obj = &line[start..end];
    [
        "input",
        "cacheRead",
        "cacheWrite",
        "output",
        "reasoningTokens",
    ]
    .iter()
    .filter_map(|k| jnum(obj, k).map(|v| (k.to_string(), v)))
    .collect()
}
// The tail readers moved to worker_board_tail.rs under the 280-line law.
// Re-exported HERE so every existing `state::<fn>` path still resolves —
// a split that silently moves a public symbol is an API break wearing a
// refactor's clothes (blocker.rs precedent).
pub use crate::worker_board_tail::{phase_last, scan_live_last, tool_counts};

pub(crate) fn jstr(line: &str, key: &str) -> Option<String> {
    let pat = format!("\"{key}\":\"");
    let start = line.find(&pat)? + pat.len();
    let end = line[start..].find('"')? + start;
    Some(line[start..end].to_string())
}

pub(crate) fn jnum(line: &str, key: &str) -> Option<u64> {
    let pat = format!("\"{key}\":");
    let start = line.find(&pat)? + pat.len();
    let digits: String = line[start..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}
