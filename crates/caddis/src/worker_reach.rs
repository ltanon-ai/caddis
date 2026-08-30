//! worker_reach.rs — CARD-0327. LAYER 3 of the disconnection ruling:
//! when a dispatch lands DONE, the worker judges the card's CREATED
//! compiled units against the landed tree and posts a talk finding for
//! any unit no repo file calls. The retire-gate law does the rest:
//! enter reports the finding unanswered, the session cannot retire
//! until it is answered, and answers REQUIRE evidence paths (E6).
//! Fail-open by law: reach findings are advisory, never a crash.

use std::fs;
use std::path::{Path, PathBuf};

use crate::lease::write_atomic;
use crate::receipt;

/// Judge the card's created units; post findings for the callerless.
pub(crate) fn judge_and_tell(dir: &Path, card: &str) {
    for path in created_sources(card) {
        let unit = unit_stem(&path);
        if unit.is_empty() || has_caller(&path, &unit) {
            continue;
        }
        let text = format!(
            "reach: `{unit}` landed by {card} with no caller in the tree (CARD-0327 LAYER 3). \
             Wire it, or declare it DORMANT in tools/reach-register.json. \
             Evidence: {}/_card_{}.md",
            std::env::current_dir()
                .unwrap_or_else(|_| ".".into())
                .display(),
            card_num(card).unwrap_or_default()
        );
        if let Err(e) = post_finding(dir, &text) {
            eprintln!("worker_reach: finding not posted (advisory, fail-open): {e}");
        }
    }
}

/// `- create <path>` rows from the card's EXECUTION allowlist: compiled
/// source only (crates/**.rs, never tests/ — a test IS a caller).
fn created_sources(card: &str) -> Vec<String> {
    let Some(num) = card_num(card) else {
        return Vec::new();
    };
    let Ok(text) = fs::read_to_string(format!("_card_{num}.md")) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|l| l.trim().strip_prefix("- create "))
        .map(str::trim)
        .filter(|p| p.starts_with("crates/") && p.ends_with(".rs") && !p.contains("/tests/"))
        .map(str::to_string)
        .collect()
}

fn card_num(card: &str) -> Option<&str> {
    card.split_once("CARD-")
        .and_then(|(_, n)| n.split_whitespace().next())
}

fn unit_stem(path: &str) -> String {
    path.rsplit('/')
        .next()
        .unwrap_or("")
        .trim_end_matches(".rs")
        .to_string()
}

/// Does any repo source file mention the unit outside its own file (and
/// its _tests sibling)? The register's caller test, pointed at the
/// landed tree.
fn has_caller(own_path: &str, unit: &str) -> bool {
    for rel in repo_sources() {
        if rel == own_path || rel == own_path.replace(".rs", "_tests.rs") {
            continue;
        }
        // swallow: best-effort-telemetry — unreadable source skipped, reach is advisory
        if let Ok(txt) = fs::read_to_string(&rel) {
            if txt.contains(&format!("{unit}::")) || txt.contains(&format!(" {unit};")) {
                return true;
            }
        }
    }
    false
}

/// Every crates/**.rs path (posix-shaped), walk-safe and absent-tree-safe.
fn repo_sources() -> Vec<String> {
    let mut out = Vec::new();
    let mut queue: Vec<PathBuf> = match fs::read_dir("crates") {
        Ok(it) => it
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect(),
        Err(_) => return out,
    };
    while let Some(d) = queue.pop() {
        let Ok(entries) = fs::read_dir(&d) else {
            continue;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                queue.push(p);
            } else if p.extension().is_some_and(|e| e == "rs") {
                out.push(p.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    out
}

/// One HMAC-stamped finding turn (the restart compose convention).
fn post_finding(dir: &Path, text: &str) -> Result<(), String> {
    let talk_dir = dir.join("talk");
    fs::create_dir_all(&talk_dir).map_err(|e| format!("mkdir talk: {e}"))?;
    let pane = std::env::var("HERDR_PANE_ID").unwrap_or_default();
    let role = if pane.is_empty() { "past" } else { "present" };
    let ts = receipt::timestamp();
    let key = receipt::load_key(dir).map_err(|e| format!("key: {e}"))?;
    let mac = crate::hmac::hmac_sha256(
        &key,
        format!("{role}|{pane}|finding|{text}|{ts}").as_bytes(),
    );
    let line = format!(
        "{{\"role\":\"{role}\",\"pane\":\"{pane}\",\"kind\":\"finding\",\"text\":\"{}\",\"ts\":\"{ts}\",\"mac\":\"{}\"}}\n",
        text.replace('\\', "\\\\").replace('"', "\\\""),
        receipt::hex_string(&mac)
    );
    let turns = talk_dir.join("turns.jsonl");
    let mut all = fs::read_to_string(&turns).unwrap_or_default();
    all.push_str(&line);
    write_atomic(&talk_dir, "turns.jsonl", all.as_bytes()).map(|_| ())
}
