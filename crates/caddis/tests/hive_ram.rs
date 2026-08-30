//! hive_ram.rs — CARD-0277 RED-first. RAM-aware dispatch: the router
//! consults kv_bridge so dispatch and memory are ONE decision.
//!
//! THE RED: today no `caddis::hive_ram` exists — the router dispatches
//! with ZERO knowledge of which context prefix is already KV-resident
//! on a lane. A bee working the same domain re-reads and recomputes its
//! prefix (archetype + soul + terrain map) on every card.
//!
//! Laws pinned here from CARD-0277 §EXECUTION:
//!
//! 1. `plan_residency` wraps `kv_bridge::co_schedule` with the lane's
//!    current pinned spans (archetype + soul + terrain map of the
//!    domain the last 3 cards touched — spans from the attention
//!    pager) and the card's new pins. The leases are ADVICE rows in the
//!    comb; the host POSTs them to the valve (organs advise — kv_bridge
//!    law 3).
//! 2. Continuity rule: a card whose terrain map overlaps the lane's
//!    resident prefix > `RESIDENCY_HIT_THRESHOLD` makes the router
//!    PREFER that lane (residency hit) over a marginally higher dance
//!    score. The threshold is a named const, test-pinned.
//! 3. The card wall (QUALITY-5) releases the card's doc-pin leases but
//!    NOT the soul prefix — identity survives the card.

use caddis::hive_ram::{
    plan_residency, release_doc_pins, residency_hit, CardPins, LaneResidency, ResidentSpan,
    SpanKind, RESIDENCY_HIT_THRESHOLD,
};
use caddis_organs::kv_bridge::ValveStatus;
use std::collections::BTreeSet;

/// A valve with the model loaded (the only world where leases exist).
fn valve() -> ValveStatus {
    ValveStatus {
        model_loaded: true,
        bytes_per_token: 512,
        keep_alive_secs: 300,
    }
}

/// The lane's resident prefix after 3 cards in domain X touched it:
/// archetype (1) + soul (2) + terrain map (3..=7), each pinned across
/// the 3 page epochs (0,1,2) those cards spanned — straight from the
/// attention pager.
fn lane_x() -> LaneResidency {
    LaneResidency {
        domain: "X".into(),
        prefix: vec![
            span(1, SpanKind::Archetype, &[0, 1, 2]),
            span(2, SpanKind::Soul, &[0, 1, 2]),
            span(3, SpanKind::Terrain, &[0, 1, 2]),
            span(4, SpanKind::Terrain, &[0, 1, 2]),
            span(5, SpanKind::Terrain, &[0, 1, 2]),
            span(6, SpanKind::Terrain, &[0, 1, 2]),
            span(7, SpanKind::Terrain, &[0, 1, 2]),
        ],
    }
}

/// A new domain-X card: its terrain map reuses 4 of the lane's 5
/// terrain spans (3,4,5,6) — a residency hit — and brings one new
/// terrain pin (8) plus one doc-pin (9) at the card's own page (3).
fn card_x() -> CardPins {
    CardPins {
        domain: "X".into(),
        terrain: vec![3, 4, 5, 6],
        pins: vec![
            span(8, SpanKind::Terrain, &[3]),
            span(9, SpanKind::DocPin, &[3]),
        ],
        doc_pins: vec![9],
    }
}

/// A domain-Y card on the domain-X lane: its terrain map (10,11) does
/// NOT overlap the lane's resident terrain — fresh leases only, no
/// residency hit.
fn card_y() -> CardPins {
    CardPins {
        domain: "Y".into(),
        terrain: vec![10, 11],
        pins: vec![
            span(10, SpanKind::Terrain, &[3]),
            span(11, SpanKind::DocPin, &[3]),
        ],
        doc_pins: vec![11],
    }
}

fn span(seq: u64, kind: SpanKind, pages: &[u64]) -> ResidentSpan {
    ResidentSpan {
        seq,
        kind,
        pages: pages.to_vec(),
    }
}

fn lease_seqs(leases: &[caddis_organs::kv_bridge::KvLease]) -> BTreeSet<u64> {
    leases.iter().map(|l| l.prefix_hash).collect()
}

/// RED: the continuity threshold is a named, test-pinned const.
#[test]
fn residency_hit_threshold_is_named_const() {
    assert_eq!(RESIDENCY_HIT_THRESHOLD, 0.6);
}

/// RED: a domain-X card on a domain-X lane is a residency hit — the
/// router PREFERS the resident lane over a marginally higher dance
/// score. 4 of 5 terrain spans overlap (0.8 > 0.6).
#[test]
fn domain_x_card_is_residency_hit() {
    assert!(residency_hit(&lane_x(), &card_x()));
}

/// RED: a domain-Y card on a domain-X lane is NOT a residency hit —
/// zero terrain overlap. The router does not prefer the resident lane.
#[test]
fn domain_y_card_is_not_residency_hit() {
    assert!(!residency_hit(&lane_x(), &card_y()));
}

/// RED: the threshold is a real boundary. 2 of 5 terrain spans (0.4)
/// is below it; 3 of 5 (0.6) is NOT strictly greater, so still a miss.
#[test]
fn residency_hit_threshold_is_a_real_boundary() {
    let mut below = card_x();
    below.terrain = vec![3, 4]; // 2/5 = 0.4
    assert!(!residency_hit(&lane_x(), &below));
    let mut edge = card_x();
    edge.terrain = vec![3, 4, 5]; // 3/5 = 0.6, not > 0.6
    assert!(!residency_hit(&lane_x(), &edge));
}

/// RED: a residency-hit card gets leases covering its pins AND the
/// lane's resident prefix (archetype + soul + terrain). The bee keeps
/// its prefix resident — no re-reading, no re-computation.
#[test]
fn residency_hit_covers_pins_and_resident_prefix() {
    let leases = plan_residency(&lane_x(), &card_x(), &valve());
    let seqs = lease_seqs(&leases);
    // Lane resident prefix: 1,2,3,4,5,6,7. Card pins: 8,9. Union = 1..=9.
    assert_eq!(seqs, (1..=9).collect::<BTreeSet<u64>>());
}

/// RED: a non-hit (domain-Y) card gets FRESH leases only — its own
/// pins — and NOT the lane's resident prefix. The prefix stays
/// resident for the domain-X bee; the domain-Y bee pays its own way.
#[test]
fn non_hit_card_gets_fresh_leases_only() {
    let leases = plan_residency(&lane_x(), &card_y(), &valve());
    let seqs = lease_seqs(&leases);
    assert_eq!(seqs, [10, 11].into_iter().collect::<BTreeSet<u64>>());
    // The lane's resident prefix is NOT leased for the domain-Y card.
    assert!(!seqs.contains(&1), "archetype not leased for domain-Y");
    assert!(!seqs.contains(&2), "soul not leased for domain-Y");
}

/// RED: the card wall (QUALITY-5) releases the card's doc-pin leases
/// but NOT the soul prefix. Identity survives the card; the doc-pin
/// was the card's own scratch and is released when the card closes.
#[test]
fn card_wall_releases_doc_pins_not_soul_prefix() {
    let leases = plan_residency(&lane_x(), &card_x(), &valve());
    let released = release_doc_pins(&leases, &card_x().doc_pins);
    let seqs = lease_seqs(&released);
    // The doc-pin (9) is released.
    assert!(
        !seqs.contains(&9),
        "doc-pin lease must be released at the wall"
    );
    // The soul prefix (2) survives the card wall.
    assert!(seqs.contains(&2), "soul prefix lease must survive the wall");
    // The archetype (1) and terrain (3..=7) also survive — only the
    // card's own doc-pin is released.
    assert!(seqs.contains(&1), "archetype survives the wall");
    assert!(seqs.contains(&7), "terrain prefix survives the wall");
}

/// RED: every lease the bridge emits mirrors the valve's keep_alive
/// (the bridge never invents a ttl) and carries a measured bytes_est
/// (never 0 when the model is loaded). kv_bridge law, re-pinned here
/// through the hive_ram wrapper.
#[test]
fn leases_mirror_valve_ttl_and_carry_measured_bytes() {
    let leases = plan_residency(&lane_x(), &card_x(), &valve());
    assert!(leases.iter().all(|l| l.expires == 300), "ttl from valve");
    assert!(
        leases.iter().all(|l| l.bytes_est > 0),
        "bytes_est measured, never 0"
    );
}

/// RED: when the valve is unloaded, no leases are produced — the
/// bridge defers to the valve's state. A residency hit does not
/// override a cold valve.
#[test]
fn unloaded_valve_produces_no_leases_even_on_hit() {
    let v = ValveStatus {
        model_loaded: false,
        bytes_per_token: 512,
        keep_alive_secs: 300,
    };
    let leases = plan_residency(&lane_x(), &card_x(), &v);
    assert!(leases.is_empty(), "cold valve -> no leases, even on a hit");
}
