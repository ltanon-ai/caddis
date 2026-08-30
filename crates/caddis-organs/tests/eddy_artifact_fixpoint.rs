//! eddy_artifact_fixpoint.rs — CARD-0239 RED-first. Prose alone no
//! longer converges a run: the fixpoint basis is the ARTIFACTS when
//! the host supplies their hashes, prose never alone.
//!
//! RED: under today's CARD-0231 law, identical outcome_hash halts an
//! until-change run EVEN WHILE artifacts move — the loop is WORKING,
//! not converged. The RED test pins that wrong halt.

use caddis_organs::eddy::{StatusClass, Tick, Verdict};
use caddis_organs::eddy_arm::{ArmSpec, Armed, Bound, LoopClass};

fn t(seq: u64, outcome: u64, artifact: u64) -> Tick {
    Tick {
        run_id: "line-a".into(),
        seq,
        payload_hash: 5,
        status_class: StatusClass::Ok,
        outcome_hash: outcome,
        artifact_hash: artifact,
        cache_read: 0,
        cache_write: 0,
        latency_ms: 0,
        ts_ms: seq * 1_000,
        resume_after: None,
        page: 0,
    }
}

fn until_change() -> Armed {
    Armed::arm(
        "work",
        ArmSpec {
            bound: Some(Bound::Iterations(1_000)),
            class: Some(LoopClass::UntilChange),
            lease_ms: None,
        },
    )
    .unwrap()
}

fn until_external() -> Armed {
    Armed::arm(
        "watch",
        ArmSpec {
            bound: Some(Bound::Iterations(1_000)),
            class: None,
            lease_ms: None,
        },
    )
    .unwrap()
}

/// THE RED: prose stagnated but artifacts MOVE → the run is working.
/// Today's law halts it; this card makes it Continue.
#[test]
fn moving_artifacts_are_progress_not_convergence() {
    let arm = until_change();
    let ticks = vec![
        t(1, 0xAA, 100),
        t(2, 0xAA, 101), // same prose, DIFFERENT artifact
        t(3, 0xAA, 102), // same prose, DIFFERENT artifact
    ];
    assert!(
        matches!(arm.judge(&ticks), Verdict::Continue),
        "prose stagnation with moving artifacts is WORK, not convergence"
    );
}

/// until-change + STAGNANT_WINDOW identical NONZERO artifacts →
/// Halt(Converged) — still a CANDIDATE, never success.
#[test]
fn stable_artifacts_converge_until_change() {
    let arm = until_change();
    let ticks = vec![t(1, 1, 0xC0DE), t(2, 2, 0xC0DE), t(3, 3, 0xC0DE)];
    match arm.judge(&ticks) {
        Verdict::Halt(r) => assert!(matches!(
            r,
            caddis_organs::eddy::HaltReason::Converged { .. }
        )),
        other => panic!("expected converged halt, got {other:?}"),
    }
}

/// until-external + identical artifacts → Stagnant (WAITING), no halt:
/// a watcher whose artifacts stopped changing is still waiting.
#[test]
fn stable_artifacts_under_until_external_are_waiting() {
    let arm = until_external();
    let ticks = vec![t(1, 1, 0xC0DE), t(2, 2, 0xC0DE), t(3, 3, 0xC0DE)];
    assert!(matches!(arm.judge(&ticks), Verdict::Stagnant));
}

/// No artifact signal at all (all zero): the prose basis still governs
/// (CARD-0231 behavior preserved for hosts that ship no hashes).
#[test]
fn absent_artifacts_fall_back_to_prose() {
    let arm = until_change();
    let ticks = vec![t(1, 0xAA, 0), t(2, 0xAA, 0), t(3, 0xAA, 0)];
    assert!(matches!(arm.judge(&ticks), Verdict::Halt(_)));
    let arm = until_external();
    assert!(matches!(arm.judge(&ticks), Verdict::Stagnant));
}

/// Artifact hashes must be NONZERO to count: zero means "nothing
/// observed", and three nothings are not convergence evidence.
#[test]
fn zero_artifacts_are_not_convergence_evidence() {
    let arm = until_change();
    // prose MOVES, artifacts all zero: nothing converges.
    let ticks = vec![t(1, 1, 0), t(2, 2, 0), t(3, 3, 0)];
    assert!(matches!(arm.judge(&ticks), Verdict::Continue));
}
