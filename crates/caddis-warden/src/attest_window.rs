//! attest_window.rs — locating a card's window in the ledger and reading what
//! the card declared, split from `attest.rs` under the 280-line law (CARD-0114).
//!
//! The seam is real: this file answers "WHERE in the ledger did this card live
//! and what did it promise", and `attest.rs` answers "what happened in there".

use crate::card_state::{CLOSE_TYPE, OPEN_TYPE};
use crate::rows::Row;

pub struct Located {
    pub open_idx: usize,
    pub close_idx: usize,
    pub from: String,
    pub path: String,
    pub hash: String,
}

fn card_fields(body: &str) -> (String, String, String) {
    let mut p = body.splitn(4, '|');
    let _verb = p.next().unwrap_or_default();
    (
        p.next().unwrap_or_default().to_string(),
        p.next().unwrap_or_default().to_string(),
        p.next().unwrap_or_default().to_string(),
    )
}

/// Find the card's window by PHYSICAL ROW POSITION. Never by `seq`: CARD-0108
/// left 8678 historical rows sharing 6733 seq values, so ordering by it
/// silently reorders history.
pub fn locate(rows: &[Row], card_id: &str) -> Option<Located> {
    let mut open: Option<(usize, String, String, String)> = None;
    for (i, row) in rows.iter().enumerate() {
        let (id, path, hash) = card_fields(&row.body);
        if id != card_id {
            continue;
        }
        match row.tool.as_str() {
            OPEN_TYPE => open = Some((i, row.from.clone(), path, hash)),
            CLOSE_TYPE => {
                if let Some((open_idx, from, path, hash)) = open.take() {
                    return Some(Located {
                        open_idx,
                        close_idx: i,
                        from,
                        path,
                        hash,
                    });
                }
            }
            _ => {}
        }
    }
    None
}

/// What the card declared: `Some((allowlist, blast))`, or `None` when the card
/// could not be READ at all.
///
/// ⛔ THE `None` AND THE EMPTY VEC ARE DIFFERENT FACTS AND THE BUNDLE MUST NOT
/// CONFLATE THEM (found by the pre-push review, finding #6, and it matters more
/// than its severity suggested). This used to return `(vec![], 0)` for every
/// failure — file deleted, unparsable, no EXECUTION section alike. An empty
/// allowlist makes `attest`'s fold skip the outside-check entirely, so a bundle
/// for a card whose file had been DELETED printed `OUTSIDE: none`, which reads
/// as CLEAN. That is exactly the reassuring artifact this program exists
/// against: the one case where nothing could be checked is the case where a
/// reader most needs to be told so.
///
/// `Some(empty)` still means a legitimate v1 card that bounds nothing.
pub fn declared(card_path: &str) -> Option<(Vec<String>, u32)> {
    let bytes = std::fs::read_to_string(card_path).ok()?;
    let card = caddis_card::Card::parse(&bytes).ok()?;
    Some(
        card.execution()
            .map(|e| (e.allowlist, e.blast))
            .unwrap_or_default(),
    )
}

/// The RED-TEST section's substantial lines, against which a command is
/// checked for an attempt.
///
/// ⚠ THE MATCH IS TWO-WAY, and getting that wrong makes the check silently
/// useless. A RED-TEST line is usually a command WITH prose around it —
/// "cargo test --workspace fails before this change" — so the recorded command
/// is a substring of the line, not the other way round. A first draft compared
/// only `command.contains(line)` and could never be true for any real card.
/// Both directions are tried, and a command shorter than `MIN` is ignored so
/// `ls` does not match every card in the estate.
///
/// Crude, and honestly labelled everywhere it surfaces: a miss reports "not
/// seen", never a false confirmation.
const MIN_MATCH: usize = 12;

pub fn red_test_lines(card_path: &str) -> Vec<String> {
    let Ok(bytes) = std::fs::read_to_string(card_path) else {
        return Vec::new();
    };
    let Ok(card) = caddis_card::Card::parse(&bytes) else {
        return Vec::new();
    };
    card.section("RED-TEST")
        .map(|s| {
            s.body
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| l.len() >= MIN_MATCH && !l.starts_with('#'))
                .collect()
        })
        .unwrap_or_default()
}

pub fn looks_like_red_test(cmd: &str, lines: &[String]) -> bool {
    let c = cmd.trim();
    if c.len() < MIN_MATCH {
        return false;
    }
    lines
        .iter()
        .any(|l| l.contains(c) || c.contains(l.as_str()))
}
