//! eddy_host_runner.rs — CARD-0234 RED-first.
//!
//! The runner is a HOST of eddy::verdict, not a second organ. H3
//! falsifier context: the kill-runner+respawn rows on the bee2 lane
//! (2026-08-27) are the shape of "I killed it and it kept going" —
//! the mechanism is real even though today's burn was NOT the runner.
//!
//! RED choreography, per the card: the test first asserts the runner
//! host EXISTS as a delegating host — it does not, so the suite is red
//! — and after the card, exactly ONE N=3 threshold constant remains in
//! the crate (the watchdog's, which the verdict now reads directly);
//! the runner host defines NO threshold of its own, pins the boundary
//! behaviorally, and structurally re-exports the one verdict.

use caddis_organs::eddy::{FatalClass, StatusClass, Tick, Verdict};
use caddis_organs::eddy_arm::{ArmSpec, Armed, Bound, LoopClass};
use caddis_organs::eddy_runner::{self, RunnerAction};

fn t(seq: u64, status: StatusClass, outcome: u64) -> Tick {
    Tick {
        run_id: "bee2".into(),
        seq,
        payload_hash: 9,
        status_class: status,
        outcome_hash: outcome,
        cache_read: 0,
        cache_write: 0,
        latency_ms: 800,
        ts_ms: seq * 1_000,
        resume_after: None,
        artifact_hash: 0,
        page: 0,
    }
}

fn armed() -> Armed {
    Armed::arm(
        "run the lane",
        ArmSpec {
            bound: Some(Bound::Iterations(1_000)),
            class: None,
            lease_ms: None,
        },
    )
    .unwrap()
}

#[test]
fn runner_host_reexports_the_one_verdict() {
    // Structural pin: the runner's judgement IS the nerve's judgement —
    // same function, re-exported, never reimplemented.
    let v: fn(&[Tick]) -> Verdict = eddy_runner::verdict;
    let _ = v;
}

#[test]
fn one_threshold_law_remains_and_it_is_the_watchdogs() {
    // The duplication this card deletes: eddy once carried its own
    // MAX_CONSECUTIVE_FAILURES next to watchdog::DEFAULT_MAX_FAILURES.
    // After the deletion the behavioral boundary is pinned here, and
    // the single constant the law reads is the watchdog's.
    let n = caddis_organs::watchdog::DEFAULT_MAX_FAILURES;
    let two: Vec<Tick> = (1..n)
        .map(|i| t(i as u64, StatusClass::Fail, i as u64))
        .collect();
    assert!(matches!(eddy_runner::verdict(&two), Verdict::Continue));
    let three: Vec<Tick> = (1..=n)
        .map(|i| t(i as u64, StatusClass::Fail, i as u64))
        .collect();
    assert!(matches!(eddy_runner::verdict(&three), Verdict::Halt(_)));
    assert_eq!(n, 3);
}

/// The equivalence matrix: for every tick shape, the runner host's
/// action matches the declared arm contract's judgement — the runner
/// may never stop later (or earlier) than the nerve would.
#[test]
fn runner_action_equals_verdict_across_the_matrix() {
    let cases: Vec<(Vec<Tick>, RunnerAction)> = vec![
        (vec![], RunnerAction::Fire),
        (vec![t(1, StatusClass::Ok, 1)], RunnerAction::Fire),
        // streak 2: fire
        (
            vec![t(1, StatusClass::Fail, 1), t(2, StatusClass::Fail, 2)],
            RunnerAction::Fire,
        ),
        // streak 3: STOP — the 2026-08-28 burn
        (
            vec![
                t(1, StatusClass::Fail, 1),
                t(2, StatusClass::Fail, 2),
                t(3, StatusClass::Fail, 3),
            ],
            RunnerAction::Stop("fail streak"),
        ),
        // fatal one: STOP at the first observation
        (
            vec![t(1, StatusClass::Fatal(FatalClass::Quota), 1)],
            RunnerAction::Stop("fatal"),
        ),
        // identical outcomes under until-external: WAIT, do not stop
        (
            vec![
                t(1, StatusClass::Ok, 7),
                t(2, StatusClass::Ok, 7),
                t(3, StatusClass::Ok, 7),
            ],
            RunnerAction::Wait,
        ),
    ];
    let arm = armed();
    for (ticks, expected) in cases {
        let got = eddy_runner::action(&arm, &ticks);
        assert_eq!(got, expected, "ticks: {} long", ticks.len());
    }
}

#[test]
fn runner_respects_the_arm_bound_like_the_nerve() {
    let arm = Armed::arm(
        "bounded",
        ArmSpec {
            bound: Some(Bound::Iterations(2)),
            class: None,
            lease_ms: None,
        },
    )
    .unwrap();
    let one = vec![t(1, StatusClass::Ok, 1)];
    assert_eq!(eddy_runner::action(&arm, &one), RunnerAction::Fire);
    let two = vec![t(1, StatusClass::Ok, 1), t(2, StatusClass::Ok, 2)];
    assert_eq!(eddy_runner::action(&arm, &two), RunnerAction::Stop("bound"));
}

#[test]
fn until_change_runner_stops_on_convergence_as_candidate() {
    let arm = Armed::arm(
        "uc",
        ArmSpec {
            bound: Some(Bound::Iterations(1_000)),
            class: Some(LoopClass::UntilChange),
            lease_ms: None,
        },
    )
    .unwrap();
    let stuck = vec![
        t(1, StatusClass::Ok, 5),
        t(2, StatusClass::Ok, 5),
        t(3, StatusClass::Ok, 5),
    ];
    assert_eq!(
        eddy_runner::action(&arm, &stuck),
        RunnerAction::Stop("converged")
    );
}
