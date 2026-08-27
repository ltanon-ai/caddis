//! card_state.rs — "which card is open" DERIVED FROM THE LEDGER (CARD-0110).
//!
//! The quorum ruled this state must live in the ledger and nowhere else. A side
//! state file drifts from the artifact the whole trust argument rests on, and
//! an environment variable leaves no trace in an attest bundle — the one place
//! a later reader has to be able to see that a card was open at all.
//!
//! ⛔ ORDER IS PHYSICAL ROW POSITION, NEVER `seq`. CARD-0108 fixed the ledger's
//! counter, and it could not repair what was already written: 8678 of 15411
//! rows share only 6733 distinct seq values, because a torn row used to reset
//! the counter to zero. Ordering by seq over that history silently reorders it.

use crate::rows::parse_row;

/// The card a caller currently holds open.
#[derive(Debug, Clone, PartialEq)]
pub struct ActiveCard {
    pub id: String,
    pub path: String,
    /// Hash of the card's bytes as they were when it was opened.
    pub hash: String,
}

/// What the ledger says about one caller's cards, INCLUDING what it could not
/// read. An empty answer and an unreadable one must never look alike.
#[derive(Debug, Clone, PartialEq)]
pub struct CardState {
    pub active: Option<ActiveCard>,
    pub unreadable: usize,
}

pub const OPEN_TYPE: &str = "card.open";
pub const CLOSE_TYPE: &str = "card.close";
pub const VERIFY_TYPE: &str = "card.verify";

/// Body shape shared by both row types: `verb|id|path|hash`.
///
/// Four fields on purpose — it is the same arity as a verdict row, so the
/// existing right-to-left `split_body` reads it without a second parser, and a
/// path containing a pipe is not representable on either platform we run on.
pub fn body(verb: &str, id: &str, path: &str, hash: &str) -> String {
    format!("{verb}|{id}|{path}|{hash}")
}

fn fields(b: &str) -> Option<(String, String, String)> {
    let mut parts = b.splitn(4, '|');
    let _verb = parts.next()?;
    let id = parts.next()?.to_string();
    let path = parts.next()?.to_string();
    let hash = parts.next()?.to_string();
    Some((id, path, hash))
}

/// The newest `card.open` for this caller with no later `card.close`.
///
/// ⛔ THE CALLER MATCH IS EXACT, not the dot-boundary prefix `--from` uses. A
/// report asking about a LANE wants every session in it; a card belongs to ONE
/// session, and a prefix match here would hand session A's card to session B —
/// precisely the crossing CARD-0109 exists to prevent.
pub fn active_for(ledger: &str, caller: &str) -> CardState {
    let mut active = None;
    let mut unreadable = 0usize;
    for line in ledger.lines().filter(|l| !l.trim().is_empty()) {
        // ⚡ CHEAP TEST FIRST, AND IT IS NOT A MICRO-OPTIMISATION. This runs on
        // EVERY write the warden judges, and the warden is spawned once per
        // tool call. Fully parsing all 15k rows to find the handful that are
        // card rows measured 21.4ms per call against a 5.2MB ledger — a 2.3x
        // slowdown of the whole call (16.7ms -> 38.1ms). A substring test skips
        // 99.9% of rows before any allocation happens.
        let t = line.trim();
        if !t.contains(CARD_TYPE_MARK) {
            if !caddis_core::ledger::is_intact_row(t) {
                unreadable += 1;
            }
            continue;
        }
        let Some(row) = parse_row(t) else {
            unreadable += 1;
            continue;
        };
        if row.from != caller {
            continue;
        }
        match row.tool.as_str() {
            OPEN_TYPE => {
                if let Some((id, path, hash)) = fields(&row.body) {
                    active = Some(ActiveCard { id, path, hash });
                }
            }
            CLOSE_TYPE => active = None,
            _ => {}
        }
    }
    CardState { active, unreadable }
}

/// The substring every card row carries and no verdict row does.
const CARD_TYPE_MARK: &str = "\"type\":\"card.";

#[cfg(test)]
#[path = "card_state_tests.rs"]
mod tests;
