//! worker_done_owner.rs — CARD-0238 RED-first. One owner of `done`,
//! written in the spec, matching the code.
//!
//! The contradiction this card settles: FR-8/D-7 said "worker never
//! marks done / waking session owns D-7"; `worker_done.rs` (CARD-0218)
//! marks done through the mechanical Done-When gate; `_worker_loop.py`
//! claimed yet a third premise. Operator stamp 2026-08-28 ("tęsk
//! darbus, tvarkyti worker ir bees") ruled per the card's
//! recommendation: the MECHANICAL GATE owns done; the spec is amended
//! (v3); the loop marks nothing.

use std::fs;
use std::path::PathBuf;

fn repo(p: &str) -> PathBuf {
    // Tests run with CWD = crate dir (crates/caddis); repo root is up two.
    let mut d = std::env::current_dir().unwrap();
    let _ = d.pop();
    let _ = d.pop();
    d.join(p)
}

#[test]
fn spec_names_the_single_done_owner_v3() {
    let spec = fs::read_to_string(repo("docs/specs/SPEC-caddis-worker-bees-2026-08-28.md"))
        .expect("spec readable");
    assert!(
        spec.contains("FR-8 (AMENDED v3, CARD-0238"),
        "FR-8 carries the amendment"
    );
    assert!(
        spec.contains("D-7 (AMENDED v3, CARD-0238"),
        "D-7 carries the amendment"
    );
    assert!(
        spec.contains("- v3 2026-08-28 — CARD-0238 amendment"),
        "changelog stamped"
    );
    // The superseded ruling must be GONE, not merely contradicted:
    assert!(
        !spec.contains("Done-marking is D-7 (waking session), never the worker"),
        "the old D-7 edge-case line is deleted, not left to contradict v3"
    );
}

/// The code is the stronger law and the amendment RATIFIES it: done is
/// written by the worker's gate, only when every check passes.
#[test]
fn gate_marks_done_only_when_checks_pass() {
    let src = fs::read_to_string(repo("crates/caddis/src/worker_done.rs")).unwrap();
    assert!(
        src.contains("if passed + by_receipt == total"),
        "done is earned mechanically (checks pass or are covered by host prove-receipts, CARD-0317)"
    );
    assert!(
        src.contains("mark_queue_line(dir, card, \"done\")"),
        "the gate (not the waking session) writes done"
    );
}
#[test]
fn waking_loop_marks_nothing() {
    if let Ok(py) = fs::read_to_string(repo("_worker_loop.py")) {
        assert!(
            py.contains("marks nothing"),
            "the loop's premise matches the amended spec"
        );
    }
    // Absent file is also fine post-CARD-0236 (Rust host replaces it);
    // the contradiction was the docstring CLAIMING ownership.
}
