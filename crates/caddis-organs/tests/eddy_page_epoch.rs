//! eddy_page_epoch.rs — CARD-0242 RED-first. Page rollover is a
//! ledgered epoch: hashes do not compare across compactions. The
//! omp-6896-i5 drive compacted MID-RUN — across a rollover the prefix
//! is replaced, so divergence is not progress and equality is not
//! convergence evidence.

use caddis_organs::eddy::{StatusClass, Tick, Verdict};
use caddis_organs::eddy_arm::{ArmSpec, Armed, Bound, LoopClass};

fn t(seq: u64, outcome: u64, page: u64) -> Tick {
    Tick {
        run_id: "line-a".into(),
        seq,
        payload_hash: 5,
        status_class: StatusClass::Ok,
        outcome_hash: outcome,
        artifact_hash: 0,
        cache_read: 0,
        cache_write: 0,
        latency_ms: 0,
        ts_ms: 10_000 + seq * 1_000,
        resume_after: None,
        page,
    }
}

/// THE RED: three identical outcomes, but the third sits on a NEW
/// page. Today's stagnant_window calls this Stagnant; hashes across a
/// rollover are not comparable — only ONE tick is on page 1.
#[test]
fn stagnation_does_not_cross_a_page_boundary() {
    let spec = ArmSpec {
        bound: Some(Bound::Iterations(1_000)),
        class: None,
        lease_ms: None,
    };
    let armed = Armed::arm("watch", spec).unwrap();
    let ticks = vec![t(1, 0xAA, 0), t(2, 0xAA, 0), t(3, 0xAA, 1)];
    assert!(
        matches!(armed.judge(&ticks), Verdict::Continue),
        "pre-boundary hashes belong to a different context"
    );
}

/// Same-page identical outcomes still stagnate (the law unchanged
/// inside one page).
#[test]
fn same_page_stagnation_still_works() {
    let spec = ArmSpec {
        bound: Some(Bound::Iterations(1_000)),
        class: None,
        lease_ms: None,
    };
    let armed = Armed::arm("watch", spec).unwrap();
    let ticks = vec![t(1, 0xAA, 0), t(2, 0xAA, 0), t(3, 0xAA, 0)];
    assert!(matches!(armed.judge(&ticks), Verdict::Stagnant));
}

/// The artifact window obeys the same comparability: stable artifacts
/// across a rollover are NOT convergence evidence.
#[test]
fn artifact_fixpoint_does_not_cross_a_page_boundary() {
    let spec = ArmSpec {
        bound: Some(Bound::Iterations(1_000)),
        class: Some(LoopClass::UntilChange),
        lease_ms: None,
    };
    let armed = Armed::arm("uc", spec).unwrap();
    let ticks = vec![
        Tick {
            artifact_hash: 0xC0DE,
            ..t(1, 1, 0)
        },
        Tick {
            artifact_hash: 0xC0DE,
            ..t(2, 2, 0)
        },
        Tick {
            artifact_hash: 0xC0DE,
            ..t(3, 3, 1)
        },
    ];
    assert!(
        matches!(armed.judge(&ticks), Verdict::Continue),
        "artifacts across a rollover are not comparable"
    );
}

/// Legacy runs (all page 0, the absent-field default) keep today's
/// behavior exactly.
#[test]
fn legacy_all_zero_page_is_one_page() {
    let spec = ArmSpec {
        bound: Some(Bound::Iterations(1_000)),
        class: None,
        lease_ms: None,
    };
    let armed = Armed::arm("watch", spec).unwrap();
    let ticks = vec![t(1, 0xAA, 0), t(2, 0xAA, 0), t(3, 0xAA, 0)];
    assert!(matches!(armed.judge(&ticks), Verdict::Stagnant));
}

/// The waiting lease clocks stagnation only within the current page's
/// comparable run: a rollover restarts the clock.
#[test]
fn lease_clock_restarts_at_a_page_boundary() {
    let spec = ArmSpec {
        bound: Some(Bound::Iterations(1_000)),
        class: None,
        lease_ms: Some(5_000),
    };
    let armed = Armed::arm("watch", spec).unwrap();
    // Stagnant on page 0 spanning 0->6s (would exceed a 5s lease),
    // but the LAST tick moved to page 1: the comparable run on page 1
    // is one tick — no measurable span, no expiry.
    let ticks = vec![t(1, 0xFEED, 0), t(2, 0xFEED, 0), t(3, 0xFEED, 1)];
    assert!(matches!(armed.judge(&ticks), Verdict::Continue));
}
