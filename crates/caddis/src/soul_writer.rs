//! soul_writer.rs — CARD-0263. The production writer that closes the
//! identity loop: soul.jsonl is written where verdicts happen — the
//! worker_done gate. Joy on a green landing; Pain on a withheld halt.
//!
//! The swallow law applies: a soul write must NEVER fail the gate. The
//! soul belongs to the LINEAGE — entries carry `created_by_model =
//! "caddis-worker"` but compose never references it.
//!
//! Split out of worker_done.rs under the 280-line law.

use std::path::Path;

use caddis_organs::soul::{write_entry, SoulEntry, SoulKind};

const WORKER_MODEL: &str = "caddis-worker";
const JOY_EMOTION: u8 = 10;
const PAIN_EMOTION: u8 = 8;

/// Joy: a card landed green. Best-effort — the swallow law applies.
pub fn write_joy(dir: &Path, card: &str) {
    let entry = SoulEntry {
        kind: SoulKind::Joy {
            source: format!("card {card} landed green"),
        },
        lesson: String::new(),
        emotion: JOY_EMOTION,
        epoch: caddis_organs::util::unix_ms() / 1000,
        created_by_model: WORKER_MODEL.into(),
    };
    // swallow: best-effort-telemetry — a failed soul write must never fail the gate
    let _ = write_entry(&dir.join("soul.jsonl"), &entry);
}

/// Pain: done withheld past the halt threshold. The blocker_id matches the
/// blocker the halt filed so the safety-valve can re-remind. Best-effort.
pub fn write_pain(dir: &Path, card: &str, why: &str) {
    let entry = SoulEntry {
        kind: SoulKind::Pain {
            cause: format!("card {card} done withheld 3x"),
            blocker_id: format!("worker:{card}"),
        },
        lesson: why.to_string(),
        emotion: PAIN_EMOTION,
        epoch: caddis_organs::util::unix_ms() / 1000,
        created_by_model: WORKER_MODEL.into(),
    };
    // swallow: best-effort-telemetry — a failed soul write must never fail the gate
    let _ = write_entry(&dir.join("soul.jsonl"), &entry);
}
