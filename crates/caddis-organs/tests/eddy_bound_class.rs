//! eddy_bound_class.rs — CARD-0231 RED-first.
//!
//! `/loop` with no limit is unbounded today (loopLimit undefined =>
//! re-fire forever). And converged-vs-waiting is a CONTRACT, not an
//! inference — no hash tells you whether identical output means done
//! or means still polling. So:
//!   - arm REQUIRES a bound (iterations or duration); unbounded is
//!     refused with a reason;
//!   - arm DECLARES a class: until-change (repetition means done) or
//!     until-external (repetition means WAITING — only bound,
//!     fail-streak and fatal class may halt);
//!   - interactive /loop defaults to until-external;
//!   - the organ may NEVER emit SUCCESS from a hash — success needs an
//!     external completion witness; repetition without that proof is
//!     STAGNANT. There is no Success variant to emit.

use caddis_organs::eddy::{StatusClass, Tick, Verdict};
use caddis_organs::eddy_arm::{ArmError, ArmSpec, Armed, Bound, LoopClass};

fn ok_tick(seq: u64, outcome: u64, ts_ms: u64) -> Tick {
    Tick {
        run_id: "run-31".into(),
        seq,
        payload_hash: 5,
        status_class: StatusClass::Ok,
        outcome_hash: outcome,
        cache_read: 0,
        cache_write: 0,
        latency_ms: 800,
        ts_ms,
        resume_after: None,
        artifact_hash: 0,
        page: 0,
    }
}

fn armed(bound: Bound, class: Option<LoopClass>) -> Armed {
    Armed::arm(
        "poll the pipeline",
        ArmSpec {
            bound: Some(bound),
            class,
            lease_ms: None,
        },
    )
    .expect("bounded arm must succeed")
}

#[test]
fn unbounded_arm_is_refused_with_a_reason() {
    let err = Armed::arm(
        "redo it",
        ArmSpec {
            bound: None,
            class: None,
            lease_ms: None,
        },
    )
    .unwrap_err();
    match err {
        ArmError::Unbounded { reason } => {
            assert!(!reason.is_empty(), "the refusal must say why");
            assert!(reason.contains("bound"), "reason names the missing bound");
        }
    }
}

#[test]
fn interactive_default_class_is_until_external() {
    let a = armed(Bound::Iterations(50), None);
    assert_eq!(a.class(), LoopClass::UntilExternal);
}

#[test]
fn iterations_bound_halts_at_n() {
    let a = armed(Bound::Iterations(3), None);
    let two = vec![ok_tick(1, 11, 1_000), ok_tick(2, 12, 2_000)];
    assert!(matches!(a.judge(&two), Verdict::Continue));
    let three = vec![
        ok_tick(1, 11, 1_000),
        ok_tick(2, 12, 2_000),
        ok_tick(3, 13, 3_000),
    ];
    assert!(matches!(a.judge(&three), Verdict::Halt(_)));
}

#[test]
fn duration_bound_uses_the_tick_clock() {
    let a = armed(Bound::Millis(5_000), None);
    let short = vec![ok_tick(1, 11, 10_000), ok_tick(2, 12, 14_999)];
    assert!(matches!(a.judge(&short), Verdict::Continue));
    let long = vec![ok_tick(1, 11, 10_000), ok_tick(2, 12, 15_000)];
    assert!(matches!(a.judge(&long), Verdict::Halt(_)));
    // Legacy ticks with no clock (ts_ms 0) must not spuriously halt:
    let legacy = vec![ok_tick(1, 11, 0), ok_tick(2, 12, 0)];
    assert!(matches!(a.judge(&legacy), Verdict::Continue));
}

/// Repetition under until-external is WAITING: reported as Stagnant,
/// never halted, never success. Only bound / fail-streak / fatal halt.
#[test]
fn repetition_under_until_external_is_stagnant_waiting() {
    let a = armed(Bound::Iterations(1_000), None);
    let stuck: Vec<Tick> = (1..=4)
        .map(|i| ok_tick(i, 0xFEED, i * 1_000)) // identical outcome, four times
        .collect();
    match a.judge(&stuck) {
        Verdict::Stagnant => {} // the honest report: nothing is progressing
        Verdict::Continue => panic!("4 identical outputs must at least be Stagnant"),
        Verdict::Halt(_) => panic!("until-external may not halt on repetition"),
        Verdict::UnprovableDone { .. } => panic!("ok ticks are not unprovable"),
    }
}

/// Same ticks, class until-change: repetition means DONE, so the loop
/// STOPS — but the halt reason is a CONVERGED CANDIDATE, not a success:
/// success needs an external completion witness (quorum §4).
#[test]
fn repetition_under_until_change_halts_as_converged_candidate() {
    let a = armed(Bound::Iterations(1_000), Some(LoopClass::UntilChange));
    let stuck: Vec<Tick> = (1..=4).map(|i| ok_tick(i, 0xFEED, i * 1_000)).collect();
    assert!(matches!(a.judge(&stuck), Verdict::Halt(_)));
}

#[test]
fn changing_outcomes_are_progress_not_stagnation() {
    let a = armed(Bound::Iterations(1_000), None);
    let moving: Vec<Tick> = (1..=4).map(|i| ok_tick(i, i * 100, i * 1_000)).collect();
    assert!(matches!(a.judge(&moving), Verdict::Continue));
}
