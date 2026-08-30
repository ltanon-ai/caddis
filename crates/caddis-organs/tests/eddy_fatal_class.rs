//! eddy_fatal_class.rs — CARD-0232 RED-first.
//!
//! A 403 quota is fatal-until-reset, not "retry three times". Live
//! counter-evidence from the quorum's own run: the K3 seat returned a
//! BYTE-IDENTICAL `403 ... 5-hour usage limit ... access_terminated_error`
//! on both dispatches — so error text does not always vary either.
//!
//! RED first, per the card: replay the byte-identical 403 above and
//! prove the loop halts on observation ONE, not 3.

use caddis_organs::eddy::{verdict, FatalClass, HaltReason, StatusClass, Tick, Verdict};

/// The K3 replay: byte-identical 403 bodies on both dispatches.
/// Everything except seq is IDENTICAL — same payload, same outcome.
fn k3_403(seq: u64, resume_after: Option<u64>) -> Tick {
    Tick {
        run_id: "run-k3".into(),
        seq,
        payload_hash: 0x5eed,
        status_class: StatusClass::Fatal(FatalClass::Auth),
        outcome_hash: 0xdead_beef, // identical on every observation
        cache_read: 0,
        cache_write: 0,
        latency_ms: 12,
        ts_ms: 1_700_000_000_000 + seq,
        resume_after,
        artifact_hash: 0,
        page: 0,
    }
}

#[test]
fn byte_identical_403_halts_on_observation_one_not_three() {
    // ONE observation must halt — this is the RED the card demands:
    // the fail-streak law alone would need three.
    let first = vec![k3_403(1, None)];
    assert!(
        matches!(verdict(&first), Verdict::Halt(HaltReason::Fatal { .. })),
        "a fatal class halts at ONE observation"
    );
    // And the full identical replay halts with the same law:
    let both = vec![k3_403(1, None), k3_403(2, None)];
    assert!(matches!(
        verdict(&both),
        Verdict::Halt(HaltReason::Fatal { .. })
    ));
}

#[test]
fn fatal_reason_labels_the_class_not_the_streak() {
    // Three fatal ticks: the reason must be Fatal, never FailStreak —
    // "retry three times" is exactly the wrong doctrine for a quota.
    let ticks = vec![k3_403(1, None), k3_403(2, None), k3_403(3, None)];
    match verdict(&ticks) {
        Verdict::Halt(HaltReason::Fatal { class, .. }) => {
            assert_eq!(class, FatalClass::Auth);
        }
        other => panic!("expected Fatal halt, got {other:?}"),
    }
}

#[test]
fn resume_after_is_recorded_when_the_provider_supplies_it() {
    let ticks = vec![k3_403(1, Some(1_700_000_018_000))]; // reset in 3 minutes
    match verdict(&ticks) {
        Verdict::Halt(HaltReason::Fatal { resume_after, .. }) => {
            assert_eq!(resume_after, Some(1_700_000_018_000));
        }
        other => panic!("expected Fatal halt, got {other:?}"),
    }
}

/// Classification is on the TYPED class, never on error text: the wire
/// carries ok | fail | fatal.quota | fatal.auth | fatal.terminated, and
/// an unknown class string is REFUSED by the parser (fail-closed codec),
/// never silently read as Ok.
#[test]
fn wire_classes_parse_typed_and_refuse_unknown() {
    assert_eq!(StatusClass::parse_wire("ok"), Some(StatusClass::Ok));
    assert_eq!(StatusClass::parse_wire("fail"), Some(StatusClass::Fail));
    assert_eq!(
        StatusClass::parse_wire("fatal.quota"),
        Some(StatusClass::Fatal(FatalClass::Quota))
    );
    assert_eq!(
        StatusClass::parse_wire("fatal.auth"),
        Some(StatusClass::Fatal(FatalClass::Auth))
    );
    assert_eq!(
        StatusClass::parse_wire("fatal.terminated"),
        Some(StatusClass::Fatal(FatalClass::Terminated))
    );
    assert_eq!(StatusClass::parse_wire("403 Forbidden"), None);
    assert_eq!(StatusClass::parse_wire("Access denied"), None);
    assert_eq!(StatusClass::parse_wire(""), None);
}

#[test]
fn fatal_outranks_stagnant_and_bound_labels() {
    // Identical outcomes AND a fatal class: the reason is Fatal — the
    // operator must see WHICH death, not just that it stopped.
    let ticks = vec![k3_403(1, None), k3_403(2, None), k3_403(3, None)];
    assert!(matches!(
        verdict(&ticks),
        Verdict::Halt(HaltReason::Fatal { .. })
    ));
}
