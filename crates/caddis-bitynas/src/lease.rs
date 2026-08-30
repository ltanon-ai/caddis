//! lease.rs — the lease data contract (CARD-BITYNAS-1): the record as
//! journaled in `pool/leases.jsonl`, its owner identity, the busy failure
//! carrying the holder, and the never-silent preemption event.

use serde::{Deserialize, Serialize};

/// TTL in seconds stamped on every fresh claim (15 min). Configurability
/// belongs to the daemon organ, not the core — the card fixes `claim`'s
/// signature and leaves the TTL to the store.
pub const DEFAULT_TTL_S: u64 = 900;

/// Who holds a lease — the identity triple every mutation must match.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaseOwner {
    pub session_id: String,
    pub host: String,
    pub pid: u32,
}

/// One slot lease as journaled in `pool/leases.jsonl`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LeaseRecord {
    pub slot_id: String,
    pub lane: String,
    pub session_id: String,
    pub host: String,
    pub pid: u32,
    /// Set by the registry organ, not the lease core — fields exist for the
    /// journal/wire contract; `claim` leaves them `None`.
    pub repo: Option<String>,
    pub card: Option<String>,
    /// RFC3339 UTC (`YYYY-MM-DDTHH:MM:SSZ`).
    pub taken_at_utc: String,
    pub ttl_s: u64,
    /// RFC3339 UTC — refreshed by [`LeaseStore::heartbeat`](crate::store::LeaseStore::heartbeat).
    pub heartbeat_at_utc: String,
    /// Set only by quorum claims: two clients asking the SAME question
    /// JOIN one live consultation instead of racing two (see
    /// [`resource`](crate::resource)). Absent on every legacy row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question_hash: Option<String>,
}

impl LeaseRecord {
    /// The holder's identity triple.
    pub fn owner(&self) -> LeaseOwner {
        LeaseOwner {
            session_id: self.session_id.clone(),
            host: self.host.clone(),
            pid: self.pid,
        }
    }

    /// Stale iff `now_utc - now_hb > ttl_s` — STRICTLY older than the TTL,
    /// so a lease at exactly its TTL is still live.
    ///
    /// Any unparsable timestamp yields `false`: a corrupt clock must never
    /// cause a WRONGFUL preemption — the cost of a wrong reclaim (two
    /// writers on one GPU) dwarfs the cost of waiting one more heartbeat.
    pub fn is_stale(&self, now_utc: &str, now_hb: &str) -> bool {
        let age = match (
            caddis_organs::util_time::unix_from_iso8601(now_utc),
            caddis_organs::util_time::unix_from_iso8601(now_hb),
        ) {
            (Some(now), Some(hb)) => now - hb,
            _ => return false,
        };
        age > self.ttl_s as i64
    }
}

/// The slot is actively held — by exactly this holder. The record travels
/// WITH the error so the caller can name the squatter, never just "busy".
#[derive(Debug, Clone, PartialEq)]
pub struct BusyError {
    pub holder: LeaseRecord,
}

impl std::fmt::Display for BusyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let h = &self.holder;
        write!(
            f,
            "slot '{}' busy: held by session '{}' on {} pid {} (lane '{}', heartbeat {})",
            h.slot_id, h.session_id, h.host, h.pid, h.lane, h.heartbeat_at_utc
        )
    }
}

impl std::error::Error for BusyError {}

/// A preemption that HAPPENED. Perėmimas NIEKADA tylus — every reclaim
/// emits one, through `events()` (claim-preemption) or `sweep`'s return
/// value, never silently and never twice.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PeremptionEvent {
    pub slot_id: String,
    pub lane: String,
    pub previous: LeaseRecord,
    /// `Some(claimer)` when a claimer took the slot, `None` when a sweep
    /// merely freed it.
    pub new_owner: Option<LeaseOwner>,
    /// RFC3339 UTC.
    pub at_utc: String,
    /// Always `"ttl_expired"` in this unit.
    pub cause: String,
}
