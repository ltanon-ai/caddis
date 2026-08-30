//! eddy_halt_failstreak.rs — CARD-0229 RED-first.
//!
//! Fail-streak N=3 is the PRIMARY halt verdict. Measured: fixpoint-on-
//! prose does NOT subsume failure counting — the four phantom replies
//! of 2026-08-28 differ in text, so no hash converges; 429/403 bodies
//! carry varying request-ids. `watchdog::DEFAULT_MAX_FAILURES` already
//! says 3.
//!
//! RED first, per the card: before this commit's organ code, the 3rd
//! Fail filed no blocker (verdict/enforce did not exist).

use std::fs;
use std::path::PathBuf;

use caddis_organs::blocker::list_open_blockers;
use caddis_organs::eddy::{enforce, verdict, StatusClass, Tick, Verdict};

fn tmp(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "caddis-eddy-failstreak-{tag}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

fn t(seq: u64, status: StatusClass) -> Tick {
    Tick {
        run_id: "run-9".into(),
        seq,
        payload_hash: 7,
        status_class: status,
        outcome_hash: seq * 31, // every outcome differs: hash-convergence must be irrelevant
        cache_read: 0,
        cache_write: 0,
        latency_ms: 800,
        ts_ms: 0, // legacy lines carry no clock
        resume_after: None,
        artifact_hash: 0,
        page: 0,
    }
}

#[test]
fn threshold_is_three_and_belongs_to_the_watchdog() {
    // CARD-0234 deleted eddy's duplicate constant: the ONE law reads
    // watchdog::DEFAULT_MAX_FAILURES, and the boundary is behavioral.
    let n = caddis_organs::watchdog::DEFAULT_MAX_FAILURES;
    assert_eq!(n, 3);
    let two: Vec<Tick> = (1..n as u64).map(|i| t(i, StatusClass::Fail)).collect();
    assert!(matches!(verdict(&two), Verdict::Continue));
    let three: Vec<Tick> = (1..=n as u64).map(|i| t(i, StatusClass::Fail)).collect();
    assert!(matches!(verdict(&three), Verdict::Halt(_)));
}

#[test]
fn third_consecutive_fail_halts_and_files_blocker() {
    // RED was here: with no verdict organ, three Fails filed nothing.
    let ticks = vec![
        t(1, StatusClass::Ok),
        t(2, StatusClass::Fail),
        t(3, StatusClass::Fail),
        t(4, StatusClass::Fail),
    ];
    assert!(matches!(verdict(&ticks), Verdict::Halt(_)));

    let dir = tmp("halt3");
    let blockers = dir.join("blockers.jsonl");
    let v = enforce("run-9", &ticks, &blockers).unwrap();
    assert!(matches!(v, Verdict::Halt(_)));
    let open = list_open_blockers(&blockers);
    assert_eq!(open.len(), 1, "the 3rd Fail must file exactly one blocker");
    assert_eq!(open[0].source, "eddy:run-9");
    assert!(open[0].reason.contains("3"), "reason states the streak");
}

#[test]
fn two_fails_continue_no_blocker() {
    let ticks = vec![t(1, StatusClass::Fail), t(2, StatusClass::Fail)];
    assert!(matches!(verdict(&ticks), Verdict::Continue));
    let dir = tmp("streak2");
    let blockers = dir.join("blockers.jsonl");
    let v = enforce("run-9", &ticks, &blockers).unwrap();
    assert!(matches!(v, Verdict::Continue));
    assert!(list_open_blockers(&blockers).is_empty());
}

/// The streak RESETS on any non-Fail. KNOWN GAP (ledgered, not papered
/// over — docs/defects/DEFECT-eddy-copyloop-gap.md): the measured
/// copy-loop phase (stopReason=stop, nothing progressing) produces Ok
/// ticks, so fail-streak alone leaves it UNCOVERED until the loop-CLASS
/// work in CARD-0231.
#[test]
fn non_fail_resets_streak_known_gap_stays_open() {
    let ticks = vec![
        t(1, StatusClass::Fail),
        t(2, StatusClass::Fail),
        t(3, StatusClass::Ok), // reset
        t(4, StatusClass::Fail),
        t(5, StatusClass::Fail),
    ];
    assert!(matches!(verdict(&ticks), Verdict::Continue));
}

#[test]
fn outcome_drift_never_prevents_the_halt() {
    // 2026-08-28 shape: 589 failures, every body different. The streak
    // counts STATUS, not hashes.
    let ticks: Vec<Tick> = (0..589).map(|i| t(i, StatusClass::Fail)).collect();
    assert!(matches!(verdict(&ticks), Verdict::Halt(_)));
}
