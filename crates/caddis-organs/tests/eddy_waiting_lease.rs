//! eddy_waiting_lease.rs — CARD-0240 RED-first. WAITING needs a lease:
//! stagnation may not idle inside a long bound. A `/loop 8h` watching a
//! pipeline that stalls in minute one would re-fire every 800 ms for
//! the remaining 7h59m (~32k no-op re-fires measured at ~900ms/tick).

use caddis_organs::eddy::{halt_reason_text, HaltReason, StatusClass, Tick, Verdict};
use caddis_organs::eddy_arm::{ArmSpec, Armed, Bound};

fn t(seq: u64, outcome: u64, ts_ms: u64) -> Tick {
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
        ts_ms,
        resume_after: None,
        page: 0,
    }
}

/// A stagnant run that outlasts its lease must HALT. Today no lease
/// exists: judge reports Stagnant forever inside the bound. THE RED.
#[test]
fn stagnant_run_outlasting_lease_halts() {
    let spec = ArmSpec {
        bound: Some(Bound::Iterations(1_000)),
        class: None,
        lease_ms: Some(5_000),
    };
    let armed = Armed::arm("watch the pipeline", spec).unwrap();
    // Stagnant run spans 0s -> 6s (>= STAGNANT_WINDOW ticks, same outcome):
    let ticks = vec![
        t(1, 0xFEED, 10_000),
        t(2, 0xFEED, 13_000),
        t(3, 0xFEED, 16_000),
    ];
    match armed.judge(&ticks) {
        Verdict::Halt(HaltReason::WaitingLeaseExpired) => {}
        other => panic!("expected WaitingLeaseExpired, got {other:?}"),
    }
}

/// Within the lease: still WAITING (Stagnant), the contract unchanged.
#[test]
fn within_lease_stays_waiting() {
    let spec = ArmSpec {
        bound: Some(Bound::Iterations(1_000)),
        class: None,
        lease_ms: Some(60_000),
    };
    let armed = Armed::arm("watch", spec).unwrap();
    let ticks = vec![
        t(1, 0xFEED, 10_000),
        t(2, 0xFEED, 13_000),
        t(3, 0xFEED, 16_000),
    ];
    assert!(matches!(armed.judge(&ticks), Verdict::Stagnant));
}

/// NO DEFAULT LEASE: lease None = no lease law; the bound alone caps
/// the run (today's behavior, unchanged).
#[test]
fn no_lease_is_no_law() {
    let spec = ArmSpec {
        bound: Some(Bound::Iterations(1_000)),
        class: None,
        lease_ms: None,
    };
    let armed = Armed::arm("watch", spec).unwrap();
    let ticks = vec![
        t(1, 0xFEED, 10_000),
        t(2, 0xFEED, 110_000),
        t(3, 0xFEED, 210_000),
    ];
    assert!(matches!(armed.judge(&ticks), Verdict::Stagnant));
}

/// Legacy ticks without a clock can never expire a duration lease —
/// an unmeasured wait expires nothing (same totality as Bound::Millis).
#[test]
fn legacy_clock_never_expires() {
    let spec = ArmSpec {
        bound: Some(Bound::Iterations(1_000)),
        class: None,
        lease_ms: Some(5_000),
    };
    let armed = Armed::arm("watch", spec).unwrap();
    let ticks = vec![t(1, 0xFEED, 0), t(2, 0xFEED, 0), t(3, 0xFEED, 0)];
    assert!(matches!(armed.judge(&ticks), Verdict::Stagnant));
}

/// The halt text names the witness that never came — a halt, never
/// success.
#[test]
fn lease_halt_names_the_missing_witness() {
    let text = halt_reason_text(&HaltReason::WaitingLeaseExpired);
    assert!(text.to_lowercase().contains("witness"), "{text}");
}

/// A NON-stagnant run never touches the lease (only stagnation idles).
#[test]
fn busy_runs_ignore_the_lease() {
    let spec = ArmSpec {
        bound: Some(Bound::Iterations(1_000)),
        class: None,
        lease_ms: Some(1),
    };
    let armed = Armed::arm("work", spec).unwrap();
    let moving = vec![t(1, 1, 0), t(2, 2, 10_000), t(3, 3, 20_000)];
    assert!(matches!(armed.judge(&moving), Verdict::Continue));
}
