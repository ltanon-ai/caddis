//! worker_withheld_halt — CARD-0235 RED-first: withheld-forever must
//! HALT the line, not re-fire it every tick. Fixture: harness.rs.

mod harness;

use harness::World;
use std::fs;

/// No card file -> the line is refused at FIRST SIGHT: no spawn at all
/// (today the bee runs and the line re-fires forever — the measured
/// CARD-9007 double-run is exactly this).
#[test]
fn no_card_file_line_is_refused_without_spawn() {
    let w = World::new("nocard");
    w.arm();
    let marker = w.root.join("marker.txt");
    let bee = w.bee_script();
    w.queue(&format!(
        "CARD-9007 python {} {}\n",
        bee.display(),
        marker.display()
    ));
    let (o, e, c) = w.tick();
    assert_eq!(c, 1, "unprovable line is a refusal: {o}{e}");
    assert!(e.contains("no card file"), "reason names it: {e}");
    assert!(!marker.exists(), "no bee may spawn for an unprovable line");
    // And the line is NOT consumed — the operator sees it in the queue.
    let q = fs::read_to_string(w.line_dir().join("queue")).unwrap();
    assert!(
        q.contains("CARD-9007"),
        "line stays visible for the operator"
    );
}

/// Withheld after REAL dispatches: counts, halts at the ONE eddy
/// threshold, takes the line out of rotation, files a blocker.
#[test]
fn withheld_dispatches_halt_the_line_at_three() {
    let w = World::new("withheld3");
    w.arm();
    let marker = w.root.join("marker.txt");
    let bee = w.bee_script();
    // Card file exists; its Done-When check FAILS on purpose (work not
    // provable done). argv is idempotent, so re-running can never fix it.
    fs::write(
        w.root.join("_card_9101.md"),
        "# Withheld probe\n\n# Done-When\n\n- $ python -c \"import sys;sys.exit(1)\"\n",
    )
    .unwrap();
    w.queue(&format!(
        "CARD-9101 python {} {}\n",
        bee.display(),
        marker.display()
    ));

    let threshold = caddis_organs::watchdog::DEFAULT_MAX_FAILURES; // the ONE constant
    for i in 1..threshold {
        let (o, e, c) = w.tick();
        assert_eq!(c, 0, "tick {i}: the bee itself succeeded: {o}{e}");
        assert!(o.contains("DW-FAIL"), "tick {i} withheld: {o}");
        let q = fs::read_to_string(w.line_dir().join("queue")).unwrap();
        assert!(
            q.trim_start().starts_with("CARD-9101"),
            "tick {i}: line still in rotation, q={q}"
        );
    }
    let (o, e, c) = w.tick();
    assert_eq!(
        c, 0,
        "the halt is a line halt, not a process failure: {o}{e}"
    );
    assert!(
        o.contains("WITHHELD-HALT CARD-9101"),
        "the halt is VISIBLE: {o}{e}"
    );
    let q = fs::read_to_string(w.line_dir().join("queue")).unwrap();
    assert!(
        q.trim_start().starts_with("withheld CARD-9101"),
        "line out of rotation: {q}"
    );
    let blockers = fs::read_to_string(w.line_dir().join("blockers.jsonl")).unwrap();
    assert!(
        blockers.contains("CARD-9101"),
        "blocker names the card: {blockers}"
    );
    assert!(
        blockers.contains("worker:"),
        "blocker source is the worker host: {blockers}"
    );
    // The burn is bounded: exactly threshold dispatches ran.
    let runs = fs::read_to_string(&marker).unwrap().lines().count();
    assert_eq!(
        runs as u32, threshold,
        "dispatch count == the eddy threshold"
    );
}

/// A card that EARNS done resets the withheld count (repair cycles
/// stay possible across queue rotations).
#[test]
fn earned_done_resets_the_withheld_count() {
    let w = World::new("reset");
    w.arm();
    let marker = w.root.join("marker.txt");
    let bee = w.bee_script();
    // 9201's check FAILS on purpose (withheld); 9202's passes (earned).
    fs::write(
        w.root.join("_card_9201.md"),
        "# A\n\n# Done-When\n\n- $ python -c \"import sys;sys.exit(1)\"\n",
    )
    .unwrap();
    fs::write(
        w.root.join("_card_9202.md"),
        "# B\n\n# Done-When\n\n- $ python -c \"print('ok')\"\n",
    )
    .unwrap();
    // 2 withheld on 9201 (below threshold)...
    w.queue(&format!(
        "CARD-9201 python {} {}\n",
        bee.display(),
        marker.display()
    ));
    for _ in 0..2 {
        let (o, _, c) = w.tick();
        assert_eq!(c, 0);
        assert!(o.contains("DW-FAIL"), "{o}");
    }
    // ...a DIFFERENT card earns done (the operator rotated the queue)...
    w.queue(&format!(
        "CARD-9202 python {} {}\n",
        bee.display(),
        marker.display()
    ));
    let (o, _, c) = w.tick();
    assert_eq!(c, 0, "{o}");
    assert!(o.contains("DW-OK"), "9202 earned done: {o}");
    // 9201 is head again: count must have RESET (2 fresh dispatches
    // do NOT halt; the third does).
    w.queue(&format!(
        "CARD-9201 python {} {}\n",
        bee.display(),
        marker.display()
    ));
    for _ in 0..2 {
        let (o, _, c) = w.tick();
        assert_eq!(c, 0, "{o}");
        assert!(o.contains("DW-FAIL"), "{o}");
        let q = fs::read_to_string(w.line_dir().join("queue")).unwrap();
        assert!(
            q.trim_start().starts_with("CARD-9201"),
            "not yet halted: {q}"
        );
    }
    let (o, _, _) = w.tick();
    assert!(
        o.contains("WITHHELD-HALT CARD-9201"),
        "reset then 3 more halts: {o}"
    );
}
