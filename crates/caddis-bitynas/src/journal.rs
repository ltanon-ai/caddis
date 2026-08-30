//! journal.rs — the BYTES of `pool/leases.jsonl`: one op-tagged JSON object
//! per line. Rows are encoded AND parsed by serde_json (steering
//! 2026-08-30: allowed dep — no hand-rolled scanner, no esc() replica; a
//! raw newline inside a field can never end a line because serde_json
//! escapes it).
//!
//! A torn or unknown row fails `from_str` and is SKIPPED and counted,
//! never fatal — the open law: a crash mid-append leaves a partial tail,
//! and the fold simply stops there.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::lease::{LeaseOwner, LeaseRecord};

/// One journal line, tagged by `"op"`. The `claim` row carries the record
/// flattened to the top level (`{"op":"claim","slot_id":...}`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(crate) enum Row {
    Claim {
        #[serde(flatten)]
        record: LeaseRecord,
    },
    Heartbeat {
        slot_id: String,
        at_utc: String,
    },
    Release {
        slot_id: String,
        at_utc: String,
    },
    Reclaim {
        slot_id: String,
        at_utc: String,
        cause: String,
        new_owner: Option<LeaseOwner>,
        previous: LeaseRecord,
    },
}

/// The `\n`-terminated line for one row. Written with ONE `write_all` per
/// line — the atomicity law (a single line is a single syscall).
pub(crate) fn line(row: &Row) -> String {
    let mut s = serde_json::to_string(row)
        .unwrap_or_else(|e| panic!("bitynas: journal row encode failed: {e}"));
    s.push('\n');
    s
}

/// Parse one line. `None` = unreadable (torn, unknown op, wrong shape) —
/// the caller counts and skips it.
pub(crate) fn parse(line: &str) -> Option<Row> {
    serde_json::from_str(line).ok()
}

/// Fold rows in journal order into the live-lease index: claim installs,
/// heartbeat refreshes, release and reclaim remove (a reclaim's successor,
/// if any, is the claim row written right AFTER it — replaying the pair
/// ends with the new holder installed).
pub(crate) fn fold(rows: impl Iterator<Item = Row>) -> BTreeMap<String, LeaseRecord> {
    let mut live = BTreeMap::new();
    for row in rows {
        match row {
            Row::Claim { record } => {
                live.insert(record.slot_id.clone(), record);
            }
            Row::Heartbeat { slot_id, at_utc } => {
                if let Some(r) = live.get_mut(&slot_id) {
                    r.heartbeat_at_utc = at_utc;
                }
            }
            Row::Release { slot_id, .. } | Row::Reclaim { slot_id, .. } => {
                live.remove(&slot_id);
            }
        }
    }
    live
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(slot: &str) -> LeaseRecord {
        LeaseRecord {
            slot_id: slot.to_string(),
            lane: "premium".to_string(),
            session_id: "ses-x".to_string(),
            host: "host-x".to_string(),
            pid: 42,
            repo: Some("r\ne\"po".to_string()), // control chars + quotes
            card: None,
            taken_at_utc: "2026-08-30T10:00:00Z".to_string(),
            ttl_s: 900,
            heartbeat_at_utc: "2026-08-30T10:05:00Z".to_string(),
            question_hash: None,
        }
    }

    #[test]
    fn claim_line_round_trips_with_control_chars_and_stays_one_line() {
        let row = Row::Claim {
            record: rec("gpu-\n\"0\""),
        };
        let text = line(&row);
        assert!(text.ends_with('\n'));
        assert_eq!(text.lines().count(), 1, "a raw newline must never end a row");
        assert_eq!(parse(text.trim_end_matches('\n')), Some(row));
    }

    #[test]
    fn reclaim_line_round_trips_both_owner_forms() {
        let prev = rec("gpu-0");
        let taker = LeaseOwner {
            session_id: "ses-n".to_string(),
            host: "h".to_string(),
            pid: 7,
        };
        for (owner, expected) in [(Some(taker.clone()), Some(taker)), (None, None)] {
            let row = Row::Reclaim {
                slot_id: "gpu-0".to_string(),
                at_utc: "2026-08-30T11:00:00Z".to_string(),
                cause: "ttl_expired".to_string(),
                new_owner: owner,
                previous: prev.clone(),
            };
            let back = parse(&line(&row));
            let owner_back = back.as_ref().map(|r| match r {
                Row::Reclaim { new_owner, .. } => new_owner.clone(),
                _ => None,
            });
            assert_eq!(owner_back, Some(expected));
            assert_eq!(back, Some(row));
        }
    }

    #[test]
    fn torn_and_unknown_rows_are_none() {
        assert!(parse("{\"op\":\"claim\",\"slot_id\":\"gpu-0\"").is_none()); // torn tail
        assert!(parse("{\"op\":\"banana\",\"slot_id\":\"x\"}").is_none()); // unknown op
        assert!(parse("").is_none());
        assert!(parse("not json at all").is_none());
    }

    #[test]
    fn fold_applies_rows_in_order() {
        let rows = vec![
            Row::Claim { record: rec("gpu-1") },
            Row::Heartbeat {
                slot_id: "gpu-1".to_string(),
                at_utc: "2026-08-30T12:00:00Z".to_string(),
            },
            Row::Claim { record: rec("gpu-2") },
            Row::Release {
                slot_id: "gpu-2".to_string(),
                at_utc: "2026-08-30T12:01:00Z".to_string(),
            },
        ];
        let live = fold(rows.into_iter());
        assert_eq!(live.len(), 1);
        let gpu1 = &live["gpu-1"];
        assert_eq!(gpu1.heartbeat_at_utc, "2026-08-30T12:00:00Z");
        assert_eq!(gpu1.taken_at_utc, "2026-08-30T10:00:00Z"); // taken untouched
    }
}
