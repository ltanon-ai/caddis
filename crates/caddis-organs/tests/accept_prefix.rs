//! accept_prefix.rs — CARD-0248 RED-first.
//!
//! MTP's deepest law: on verification failure, keep every step up to
//! the first divergence and rewind ONLY the tail. Today the estate
//! discards everything — a card whose Done-When fails at check 7 of
//! 10 re-runs the whole card. `longest_prefix` is the pure organ that
//! converts every failed verification from a full restart into a
//! tail-rewind: given ordered checkpoint results, the accepted prefix
//! is everything up to (not including) the first Fail.
//!
//! Two laws pinned here from CARD-0248 §EXECUTION:
//!
//! 1. The prefix is CONTIGUOUS from the start — everything up to (not
//!    including) the first Fail. Steps after a Fail are never accepted,
//!    even if later results are Pass. Relaxed acceptance (MTP's top-k
//!    tolerance) is REJECTED for work — work verifies exactly or
//!    rewinds (our 0% false-pass law outranks throughput).
//! 2. Empty on first-step fail — if the very first result is Fail,
//!    the prefix is empty (step 0, hash 0). No partial credit.
//!
//! Today the test cannot even compile — the module, the enum, and the
//! function do not exist. That is the RED.

use caddis_organs::accept_prefix::{longest_prefix, CheckResult, VerifiedPrefix};

/// Helper: a Pass at the given step with the given checkpoint hash.
fn pass(step: usize, hash: u64) -> CheckResult {
    CheckResult::Pass { step, hash }
}

/// Helper: a Fail at the given step with a reason string.
fn fail(step: usize, why: &str) -> CheckResult {
    CheckResult::Fail {
        step,
        why: why.to_string(),
    }
}

/// RED: all Pass — the prefix covers every step. The checkpoint_hash
/// is the hash of the LAST accepted checkpoint (the resume point).
#[test]
fn all_pass_prefix_covers_everything() {
    let results = vec![pass(1, 0xAA), pass(2, 0xBB), pass(3, 0xCC)];
    let prefix = longest_prefix(&results);
    assert_eq!(prefix.step, 3, "all 3 steps accepted");
    assert_eq!(prefix.checkpoint_hash, 0xCC, "hash of last accepted step");
}

/// RED: first step is Fail — the prefix is EMPTY. No partial credit,
/// no skipping. Step 0, hash 0.
#[test]
fn first_step_fail_yields_empty_prefix() {
    let results = vec![fail(1, "build broke"), pass(2, 0xBB)];
    let prefix = longest_prefix(&results);
    assert_eq!(prefix.step, 0, "empty prefix on first-step fail");
    assert_eq!(prefix.checkpoint_hash, 0, "no checkpoint to resume from");
}

/// RED: Fail at step 7 of 10 — the prefix is 6/10. This is the card's
/// headline scenario: "accepted 6/10, rewound from step 7". The
/// checkpoint_hash is the hash of step 6 (the last accepted Pass),
/// which is the resume point for the tail-rewind.
#[test]
fn fail_at_step_7_of_10_prefix_is_six() {
    let results = vec![
        pass(1, 0x01),
        pass(2, 0x02),
        pass(3, 0x03),
        pass(4, 0x04),
        pass(5, 0x05),
        pass(6, 0x06),
        fail(7, "check 7 failed"),
        pass(8, 0x08),
        pass(9, 0x09),
        pass(10, 0x0A),
    ];
    let prefix = longest_prefix(&results);
    assert_eq!(prefix.step, 6, "accepted 6/10");
    assert_eq!(
        prefix.checkpoint_hash, 0x06,
        "resume from step 6 checkpoint"
    );
}

/// RED: the prefix is CONTIGUOUS — a Pass AFTER a Fail is never
/// accepted. Relaxed acceptance (top-k tolerance) is REJECTED for
/// work. The organ stops at the first divergence, full stop.
#[test]
fn relaxed_acceptance_is_rejected_prefix_is_contiguous() {
    let results = vec![
        pass(1, 0x11),
        pass(2, 0x22),
        fail(3, "divergence"),
        pass(4, 0x44),
        pass(5, 0x55),
    ];
    let prefix = longest_prefix(&results);
    assert_eq!(prefix.step, 2, "only 2 accepted — steps after Fail ignored");
    assert_eq!(prefix.checkpoint_hash, 0x22, "last accepted is step 2");
}

/// RED: empty input — empty prefix. Total on any input.
#[test]
fn empty_input_yields_empty_prefix() {
    let prefix = longest_prefix(&[]);
    assert_eq!(prefix.step, 0);
    assert_eq!(prefix.checkpoint_hash, 0);
}

/// RED: single Pass — prefix is that one step.
#[test]
fn single_pass_prefix_is_one() {
    let prefix = longest_prefix(&[pass(1, 0x42)]);
    assert_eq!(prefix.step, 1);
    assert_eq!(prefix.checkpoint_hash, 0x42);
}

/// RED: single Fail — empty prefix.
#[test]
fn single_fail_yields_empty_prefix() {
    let prefix = longest_prefix(&[fail(1, "only step failed")]);
    assert_eq!(prefix.step, 0);
    assert_eq!(prefix.checkpoint_hash, 0);
}

/// RED: the dispatch counter proves a full re-run, not a tail-rewind.
/// This pins the card's RED-TEST clause: "the worker's repair test
/// proves a Done-When failure re-runs from step 1 (pinned: dispatch
/// counter shows full re-run, not tail-rewind)". The pure organ's
/// contract is that `longest_prefix` returns the accepted length —
/// the HOST then re-arms from `step + 1`, NOT from 1. If the host
/// re-armed from 1, the dispatch counter would show N re-runs of the
/// full sequence; with tail-rewind it shows one run that reached
/// `prefix.step` and then re-armed from `prefix.step + 1`.
#[test]
fn tail_rewind_re_arms_from_prefix_step_plus_one_not_one() {
    // 10 checks, fail at 7 → prefix is 6.
    let results = vec![
        pass(1, 0x01),
        pass(2, 0x02),
        pass(3, 0x03),
        pass(4, 0x04),
        pass(5, 0x05),
        pass(6, 0x06),
        fail(7, "check 7"),
        pass(8, 0x08),
        pass(9, 0x09),
        pass(10, 0x0A),
    ];
    let prefix: VerifiedPrefix = longest_prefix(&results);

    // The resume point is prefix.step + 1 — the step AFTER the last
    // accepted Pass. A full re-run would resume from 1.
    let resume_step = prefix.step + 1;
    assert_eq!(resume_step, 7, "tail-rewind resumes from step 7, not 1");
    assert_ne!(resume_step, 1, "NOT a full re-run from step 1");
}
