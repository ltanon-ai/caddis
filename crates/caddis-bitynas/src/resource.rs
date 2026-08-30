//! resource.rs — CARD-BITYNAS-6: typed resources on the H-1 lease core.
//! Bees were the first resource type; Council, Quorum and Voice ride the
//! SAME atom (check-then-act on the one index), the SAME journal and the
//! SAME preemption law — additively, nothing live breaks.
//!
//! NAMESPACE LAW: every non-bee slot id carries its type prefix
//! (`council:panel`, `quorum:main`, `voice:main`), so types never collide.
//! Bee ids stay BARE: `claim()` and `claim_typed(BeeSlot, …)` share one
//! namespace with H-1's on-disk rows — prefixing bee writes would blind a
//! typed bee to a live legacy lease (a silent double-lease).
//!
//! CAPACITY POLICY: capacity is enforced by namespaced slot identity —
//! Council/Quorum/Voice are capacity-1 per named resource (a second
//! claimer of the same prefixed id gets Busy naming the holder); BeeSlot
//! stays per-lane dynamic (distinct caller-chosen ids, as in H-1).
//!
//! TTL POLICY: Voice = 60 s (a spoken report is message-length short, no
//! heartbeat), Council = 30 min with heartbeat, Quorum = 2 h with
//! heartbeat, BeeSlot = [`DEFAULT_TTL_S`]. "With heartbeat" needs no new
//! mechanism — [`heartbeat`](crate::store::LeaseStore::heartbeat) already
//! refreshes any lease and staleness reads each record's own `ttl_s`.

use std::collections::BTreeMap;

use caddis_organs::util_time::iso8601_now;

use crate::journal::{self, Row};
use crate::lease::{BusyError, LeaseOwner, LeaseRecord, PeremptionEvent, DEFAULT_TTL_S};
use crate::store::{fresh_record, LeaseStore, CAUSE_TTL};

/// Voice lease TTL: message-length short, no heartbeat expected.
pub const VOICE_TTL_S: u64 = 60;
/// Council lease TTL: 30 min, kept alive by heartbeat.
pub const COUNCIL_TTL_S: u64 = 1800;
/// Quorum lease TTL: 2 h, kept alive by heartbeat.
pub const QUORUM_TTL_S: u64 = 7200;

/// The resource types a lease can cover (CARD-BITYNAS-6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResourceType {
    /// A GPU-pool slot — the H-1 original; bare ids, per-lane dynamic.
    BeeSlot,
    /// A council consultation — capacity 1 per named council.
    Council,
    /// A quorum ruling — capacity 1; same question JOINS (see
    /// [`ClaimOutcome::Join`]).
    Quorum,
    /// A spoken report lane — capacity 1, TTL message-length short.
    Voice,
}

impl ResourceType {
    /// The slot-id namespace prefix. `bee:` exists for `from_slot_id`
    /// symmetry, but bee WRITES stay bare (the namespace law above).
    pub fn prefix(self) -> &'static str {
        match self {
            ResourceType::BeeSlot => "bee:",
            ResourceType::Council => "council:",
            ResourceType::Quorum => "quorum:",
            ResourceType::Voice => "voice:",
        }
    }

    /// The per-type TTL policy in seconds.
    pub fn ttl_s(self) -> u64 {
        match self {
            ResourceType::BeeSlot => DEFAULT_TTL_S,
            ResourceType::Council => COUNCIL_TTL_S,
            ResourceType::Quorum => QUORUM_TTL_S,
            ResourceType::Voice => VOICE_TTL_S,
        }
    }

    /// The type of a journaled slot id. A BARE id (every H-1 row ever
    /// written) parses as [`BeeSlot`](ResourceType::BeeSlot); a
    /// `bee:`-prefixed id parses as BeeSlot too (forward compat — a
    /// future writer may prefix; today's bare readers must not misread).
    pub fn from_slot_id(slot_id: &str) -> ResourceType {
        for ty in [
            ResourceType::Council,
            ResourceType::Quorum,
            ResourceType::Voice,
        ] {
            if slot_id.starts_with(ty.prefix()) {
                return ty;
            }
        }
        ResourceType::BeeSlot
    }
}

/// The outcome of [`claim_typed`](LeaseStore::claim_typed).
#[derive(Debug, Clone, PartialEq)]
pub enum ClaimOutcome {
    /// The caller holds a fresh (or preempted-stale) lease.
    Claimed(LeaseRecord),
    /// The resource is actively held — by exactly this holder.
    Busy(BusyError),
    /// A LIVE consultation with the SAME `question_hash` already exists:
    /// the caller JOINS it — waits for that one answer — instead of
    /// racing a second consultation. `existing` is the holder's lease.
    Join { existing: LeaseRecord },
}

/// The journaled slot id: bees BARE (one namespace with H-1 rows), every
/// other type prefixed (types never collide).
pub(crate) fn namespaced_id(resource: ResourceType, id: &str) -> String {
    match resource {
        ResourceType::BeeSlot => id.to_string(),
        _ => format!("{}{}", resource.prefix(), id),
    }
}

/// The live quorum lease (ANY slot — the consultation, not the slot name,
/// is the resource) consulting exactly this question, if any. Stale
/// consultations do not join: staleness is evaluated here, BEFORE any
/// claim decision, so a zombie consultation is preempted, never joined.
fn find_active_quorum_by_hash(
    index: &BTreeMap<String, LeaseRecord>,
    hash: &str,
    now: &str,
) -> Option<LeaseRecord> {
    index
        .values()
        .filter(|r| ResourceType::from_slot_id(&r.slot_id) == ResourceType::Quorum)
        .filter(|r| r.question_hash.as_deref() == Some(hash))
        .filter(|r| !r.is_stale(now, &r.heartbeat_at_utc))
        .cloned()
        .next()
}

/// Reclaim a stale lease and hand the slot to `owner` — the APPEND LAW
/// verbatim: decide on the index, journal the reclaim row then the claim
/// row (one `write_all` each), queue the [`PeremptionEvent`] (a reclaim
/// is never silent), then update the index.
fn preempt_stale(
    store: &mut LeaseStore,
    slot_id: &str,
    lane: &str,
    resource: ResourceType,
    owner: &LeaseOwner,
    held: LeaseRecord,
    now: &str,
    question_hash: Option<&str>,
) -> LeaseRecord {
    store.append(&journal::line(&Row::Reclaim {
        slot_id: slot_id.to_string(),
        at_utc: now.to_string(),
        cause: CAUSE_TTL.to_string(),
        new_owner: Some(owner.clone()),
        previous: held.clone(),
    }));
    let rec = fresh_record(slot_id, lane, owner, resource.ttl_s(), question_hash);
    store.append(&journal::line(&Row::Claim { record: rec.clone() }));
    store.pending.push(PeremptionEvent {
        slot_id: slot_id.to_string(),
        lane: held.lane.clone(),
        previous: held,
        new_owner: Some(owner.clone()),
        at_utc: now.to_string(),
        cause: CAUSE_TTL.to_string(),
    });
    store.index.insert(slot_id.to_string(), rec.clone());
    rec
}

impl LeaseStore {
    /// Atomically claim `id` as `resource` — the SAME check-then-act path,
    /// journal and preemption law as
    /// [`claim`](crate::store::LeaseStore::claim), with the slot id
    /// namespaced by type and the TTL taken from the type's policy.
    ///
    /// Free (or never taken) → [`Claimed`](ClaimOutcome::Claimed). Held
    /// and fresh → [`Busy`](ClaimOutcome::Busy) naming the holder. Held
    /// but stale → preempted, never silently, → `Claimed`. Quorum only:
    /// a `question_hash` joins a LIVE consultation asking the same
    /// question → [`Join`](ClaimOutcome::Join) — two clients wait for one
    /// answer, not two consultations (the joiner holds nothing and no row
    /// is journaled; the daemon organ wires the waiting).
    ///
    /// # Panics
    ///
    /// On an empty (or whitespace-only) `id`/`lane`, a `question_hash` on
    /// a non-quorum claim, or an empty hash: caller bugs, not busy slots.
    pub fn claim_typed(
        &mut self,
        resource: ResourceType,
        id: &str,
        lane: &str,
        owner: LeaseOwner,
        question_hash: Option<&str>,
    ) -> ClaimOutcome {
        assert!(
            !id.trim().is_empty() && !lane.trim().is_empty(),
            "bitynas: claim needs a non-empty slot_id and lane"
        );
        if let Some(hash) = question_hash {
            assert!(
                resource == ResourceType::Quorum && !hash.trim().is_empty(),
                "bitynas: question_hash is a quorum-only, non-empty join key"
            );
        }
        let slot_id = namespaced_id(resource, id);
        let now = iso8601_now();
        if let (ResourceType::Quorum, Some(hash)) = (resource, question_hash) {
            if let Some(existing) = find_active_quorum_by_hash(&self.index, hash, &now) {
                return ClaimOutcome::Join { existing };
            }
        }
        match self.index.get(&slot_id).cloned() {
            None => {
                let rec = fresh_record(&slot_id, lane, &owner, resource.ttl_s(), question_hash);
                self.append(&journal::line(&Row::Claim { record: rec.clone() }));
                self.index.insert(slot_id, rec.clone());
                ClaimOutcome::Claimed(rec)
            }
            Some(held) if held.is_stale(&now, &held.heartbeat_at_utc) => ClaimOutcome::Claimed(
                preempt_stale(self, &slot_id, lane, resource, &owner, held, &now, question_hash),
            ),
            Some(held) => ClaimOutcome::Busy(BusyError { holder: held }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_slot_id_bare_and_prefixed_matrix() {
        assert_eq!(ResourceType::from_slot_id("gpu-0"), ResourceType::BeeSlot); // H-1 legacy
        assert_eq!(ResourceType::from_slot_id("bee:gpu-0"), ResourceType::BeeSlot);
        assert_eq!(ResourceType::from_slot_id("council:panel"), ResourceType::Council);
        assert_eq!(ResourceType::from_slot_id("quorum:main"), ResourceType::Quorum);
        assert_eq!(ResourceType::from_slot_id("voice:main"), ResourceType::Voice);
    }

    #[test]
    fn ttl_policy_per_type() {
        assert_eq!(ResourceType::BeeSlot.ttl_s(), DEFAULT_TTL_S);
        assert_eq!(ResourceType::Council.ttl_s(), COUNCIL_TTL_S);
        assert_eq!(ResourceType::Quorum.ttl_s(), QUORUM_TTL_S);
        assert_eq!(ResourceType::Voice.ttl_s(), VOICE_TTL_S);
    }

    #[test]
    fn namespaced_ids_bees_bare_others_prefixed() {
        assert_eq!(namespaced_id(ResourceType::BeeSlot, "gpu-0"), "gpu-0");
        assert_eq!(namespaced_id(ResourceType::Council, "panel"), "council:panel");
        assert_eq!(namespaced_id(ResourceType::Quorum, "main"), "quorum:main");
        assert_eq!(namespaced_id(ResourceType::Voice, "main"), "voice:main");
    }
}
