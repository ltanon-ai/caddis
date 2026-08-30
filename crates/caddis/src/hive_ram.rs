//! hive_ram.rs — CARD-0277. RAM-aware dispatch: the router consults
//! kv_bridge so dispatch and memory are ONE decision.
//!
//! A bee working the same domain keeps its context prefix (archetype +
//! soul + terrain map) KV-resident across cards — no re-reading, no
//! re-computation. [`plan_residency`] wraps [`kv_bridge::co_schedule`]
//! with the lane's current pinned spans (from the attention pager) and
//! the card's new pins; the returned [`KvLease`]s are ADVICE rows in the
//! comb. The host POSTs them to the model valve — the bridge NEVER
//! talks to the engine directly (one writer per resource; kv_bridge
//! law 3). The panel/valve POST is a host concern; organs advise.
//!
//! Continuity rule: a card whose terrain map overlaps the lane's
//! resident prefix > [`RESIDENCY_HIT_THRESHOLD`] is a residency hit —
//! the router PREFERS that lane over a marginally higher dance score.
//! The threshold is a named const, test-pinned.
//!
//! Card wall (QUALITY-5): [`release_doc_pins`] releases the card's
//! doc-pin leases but NOT the soul prefix — identity survives the card.

use std::collections::BTreeSet;

use caddis_organs::attention::SpanEvent;
use caddis_organs::kv_bridge::{co_schedule, KvLease, ValveStatus};

/// A residency hit when the card's terrain map covers more than this
/// fraction of the lane's resident terrain prefix. Named, test-pinned
/// (CARD-0277 §EXECUTION): the router prefers the resident lane over a
/// marginally higher dance score past this boundary.
pub const RESIDENCY_HIT_THRESHOLD: f64 = 0.6;

/// The kind of a resident context span. The lane's resident prefix is
/// archetype + soul + terrain map of the domain the last 3 cards
/// touched; doc-pins are per-card and released at the wall.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpanKind {
    Archetype,
    Soul,
    Terrain,
    DocPin,
}

/// One resident span: its identity (seq), its kind, and the page epochs
/// it is pinned in (from the attention pager). A span pinned across N
/// epochs contributes N prefix segments to the KV-cache lease.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResidentSpan {
    pub seq: u64,
    pub kind: SpanKind,
    pub pages: Vec<u64>,
}

/// A bee lane's RAM state: the domain the last 3 cards touched and the
/// lane's resident prefix — the pinned spans from the attention pager
/// (archetype + soul + terrain map of that domain).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneResidency {
    pub domain: String,
    pub prefix: Vec<ResidentSpan>,
}

/// A card's RAM demand: its domain, its terrain-map span seqs (for the
/// continuity overlap), its new pins (terrain + doc-pins the card
/// brings), and the doc-pin seqs the card wall releases.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardPins {
    pub domain: String,
    pub terrain: Vec<u64>,
    pub pins: Vec<ResidentSpan>,
    pub doc_pins: Vec<u64>,
}

/// Expand resident spans into the pager's Present event stream: one
/// pinned Present event per (span, page-epoch). Non-pinned spans do not
/// appear — the pager's pin flag is the sole authority (kv_bridge law).
fn to_events(spans: &[ResidentSpan]) -> Vec<SpanEvent> {
    let mut events = Vec::new();
    for s in spans {
        for &page in &s.pages {
            events.push(SpanEvent::Present {
                seq: s.seq,
                pinned: true,
                page,
            });
        }
    }
    events
}

/// The lane's resident terrain seqs (the terrain-kind spans of its
/// prefix) — the domain-relevant part the continuity rule measures
/// against. Archetype + soul are common across domains and excluded so
/// they cannot inflate a cross-domain overlap.
fn lane_terrain(lane: &LaneResidency) -> BTreeSet<u64> {
    lane.prefix
        .iter()
        .filter(|s| s.kind == SpanKind::Terrain)
        .map(|s| s.seq)
        .collect()
}

/// Fraction of the lane's resident terrain prefix covered by the card's
/// terrain map: |card.terrain ∩ lane.terrain| / |lane.terrain|. Returns
/// 0.0 when the lane has no resident terrain (no prefix to reuse).
/// Bounded at 1.0 since the intersection is a subset of the lane's
/// terrain.
fn terrain_overlap(lane: &LaneResidency, card: &CardPins) -> f64 {
    let lane_terr = lane_terrain(lane);
    if lane_terr.is_empty() {
        return 0.0;
    }
    let card_terr: BTreeSet<u64> = card.terrain.iter().copied().collect();
    let shared = lane_terr.intersection(&card_terr).count() as f64;
    shared / lane_terr.len() as f64
}

/// Continuity rule: a card whose terrain map overlaps the lane's
/// resident prefix > [`RESIDENCY_HIT_THRESHOLD`] is a residency hit.
/// The router PREFERS the resident lane over a marginally higher dance
/// score. Strictly greater than the threshold — a 60% tie is not a hit.
pub fn residency_hit(lane: &LaneResidency, card: &CardPins) -> bool {
    terrain_overlap(lane, card) > RESIDENCY_HIT_THRESHOLD
}

/// PURE: plan the KV-cache residency for dispatching `card` to `lane`.
/// On a residency hit the leases cover the card's new pins AND the
/// lane's resident prefix (the bee keeps its prefix resident — no
/// re-reading). On a miss only the card's own pins are leased (fresh
/// leases). The leases are advice rows; the host POSTs them to the
/// valve. When the valve is unloaded, no leases are produced — the
/// bridge defers to the valve's state (kv_bridge law).
pub fn plan_residency(lane: &LaneResidency, card: &CardPins, valve: &ValveStatus) -> Vec<KvLease> {
    let mut spans: Vec<ResidentSpan> = Vec::new();
    if residency_hit(lane, card) {
        spans.extend(lane.prefix.iter().cloned());
    }
    spans.extend(card.pins.iter().cloned());
    co_schedule(&to_events(&spans), valve)
}

/// Card wall (QUALITY-5): release the card's doc-pin leases. The
/// doc-pin was the card's own scratch and is released when the card
/// closes; the soul prefix (and archetype + terrain) survive — identity
/// survives the card. Returns the leases minus those whose prefix_hash
/// is in `doc_pins`.
pub fn release_doc_pins(leases: &[KvLease], doc_pins: &[u64]) -> Vec<KvLease> {
    let drop: BTreeSet<u64> = doc_pins.iter().copied().collect();
    leases
        .iter()
        .filter(|l| !drop.contains(&l.prefix_hash))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(seq: u64, kind: SpanKind, pages: &[u64]) -> ResidentSpan {
        ResidentSpan {
            seq,
            kind,
            pages: pages.to_vec(),
        }
    }

    #[test]
    fn to_events_emits_one_present_per_page_epoch() {
        let spans = vec![span(7, SpanKind::Terrain, &[0, 1, 2])];
        let events = to_events(&spans);
        assert_eq!(events.len(), 3, "one event per pinned page epoch");
        assert!(
            events.iter().all(|e| matches!(
                e,
                SpanEvent::Present {
                    pinned: true,
                    seq: 7,
                    ..
                }
            )),
            "all events are pinned Present for seq 7"
        );
    }

    #[test]
    fn terrain_overlap_counts_lane_coverage() {
        let lane = LaneResidency {
            domain: "X".into(),
            prefix: vec![
                span(1, SpanKind::Archetype, &[0]),
                span(3, SpanKind::Terrain, &[0]),
                span(4, SpanKind::Terrain, &[0]),
                span(5, SpanKind::Terrain, &[0]),
            ],
        };
        // card covers 2 of 3 terrain spans -> 2/3
        let card = CardPins {
            domain: "X".into(),
            terrain: vec![3, 4],
            pins: vec![],
            doc_pins: vec![],
        };
        let got = terrain_overlap(&lane, &card);
        assert!((got - (2.0 / 3.0)).abs() < 1e-9, "2/3 coverage: {got}");
    }

    #[test]
    fn terrain_overlap_excludes_archetype_and_soul() {
        let lane = LaneResidency {
            domain: "X".into(),
            prefix: vec![
                span(1, SpanKind::Archetype, &[0]),
                span(2, SpanKind::Soul, &[0]),
                span(3, SpanKind::Terrain, &[0]),
            ],
        };
        // A card claiming the archetype+soul seqs but no terrain -> 0.
        let card = CardPins {
            domain: "Y".into(),
            terrain: vec![1, 2],
            pins: vec![],
            doc_pins: vec![],
        };
        assert_eq!(terrain_overlap(&lane, &card), 0.0, "only terrain counts");
    }

    #[test]
    fn empty_lane_terrain_is_zero_overlap() {
        let lane = LaneResidency {
            domain: "X".into(),
            prefix: vec![span(1, SpanKind::Soul, &[0])],
        };
        let card = CardPins {
            domain: "X".into(),
            terrain: vec![1],
            pins: vec![],
            doc_pins: vec![],
        };
        assert_eq!(terrain_overlap(&lane, &card), 0.0, "no terrain prefix -> 0");
    }

    #[test]
    fn release_doc_pins_drops_only_listed_seqs() {
        let leases = vec![
            KvLease {
                prefix_hash: 1,
                bytes_est: 512,
                expires: 300,
            },
            KvLease {
                prefix_hash: 9,
                bytes_est: 512,
                expires: 300,
            },
        ];
        let kept = release_doc_pins(&leases, &[9]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].prefix_hash, 1, "soul prefix survives");
    }
}
