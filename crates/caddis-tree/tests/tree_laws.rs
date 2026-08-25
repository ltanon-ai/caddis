//! tree_laws.rs — BC3 (CARD-0093), part 2: intake law, strong-lane law,
//! the failure map, and the repo-reality plan gates. BC4 (CARD-0094):
//! strategy stamping and preset hysteresis.

mod common;

use caddis_tree::plan_gates::{self, Finding};
use caddis_tree::presets::PresetGate;
use caddis_tree::state::{EventKind, Lane, StateErr, TreeState};
use caddis_tree::walker::{Action, Walker};
use common::{caps, fail_exec, pass_exec, plan, scratch, seed_repo, walking};
use std::fs;

#[test]
fn intake_laws_strong_root_red_once() {
    let root = scratch("intake");
    fs::create_dir_all(&root).unwrap();
    let st = TreeState::new(root.join("goal.jsonl"), "w1", caps()).unwrap();
    let mut w = Walker::new(st, root.clone());
    assert!(
        matches!(w.intake("root_red.md"), Err(StateErr::NoRootRed)),
        "missing file refused"
    );
    fs::write(root.join("root_red.md"), "assert!(tree_works());\n").unwrap();
    w.intake("root_red.md").unwrap();
    assert!(
        matches!(w.intake("root_red.md"), Err(StateErr::AlreadyIntaked)),
        "one intake per goal"
    );
    let over = w.dispatch_as(
        "CARD-A",
        &pass_exec(),
        &Lane::Weak("sim".into()),
        "weak-first",
        0,
    );
    assert!(
        matches!(over, Err(StateErr::OrphanCard)),
        "no plan accepted yet"
    );
}

#[test]
fn strong_lane_never_writes_under_a_live_subtree() {
    let root = scratch("strong");
    seed_repo(&root);
    let mut w = walking(&root);
    w.accept_plan("PLAN-T", &plan()).unwrap();
    let strong = w.dispatch_as("CARD-A", &pass_exec(), &Lane::Strong, "weak-first", 0);
    assert!(
        matches!(strong, Err(StateErr::StrongUnderLive)),
        "the strong lane closes, never writes under live"
    );
    w.record_strong_close("CARD-A").unwrap();
    let after = w.dispatch_as("CARD-A", &pass_exec(), &Lane::Strong, "weak-first", 0);
    assert!(
        matches!(after, Err(StateErr::AlreadyDone)),
        "closed is closed for every lane"
    );
}

#[test]
fn failure_map_retries_then_bubbles_then_strong_closes() {
    let root = scratch("fmap");
    seed_repo(&root);
    let mut w = walking(&root);
    w.accept_plan("PLAN-T", &plan()).unwrap();
    for attempt in 1..=2 {
        assert!(!w
            .dispatch_as(
                "CARD-A",
                &fail_exec(),
                &Lane::Weak("sim".into()),
                "weak-first",
                0
            )
            .unwrap());
        match w.on_fail("CARD-A") {
            Action::Retry { attempt: n } => assert_eq!(n, attempt + 1),
            other => panic!("attempt {attempt}: expected Retry, got {other:?}"),
        }
    }
    assert!(!w
        .dispatch_as(
            "CARD-A",
            &fail_exec(),
            &Lane::Weak("sim".into()),
            "weak-first",
            0
        )
        .unwrap());
    assert_eq!(
        w.on_fail("CARD-A"),
        Action::BubbleUp {
            from: "CARD-A".into(),
            to: "PLAN-T".into()
        },
        "retry-leaf is <=3 attempts: the 3rd failure bubbles"
    );
    w.record_bubble_up("CARD-A", "PLAN-T").unwrap();
    for _ in 1..=3 {
        w.dispatch_as(
            "CARD-B",
            &fail_exec(),
            &Lane::Weak("sim".into()),
            "weak-first",
            0,
        )
        .unwrap();
    }
    assert_eq!(
        w.on_fail("CARD-B"),
        Action::StrongClose {
            card: "CARD-B".into()
        },
        "parent already replanned once"
    );
}

#[test]
fn plan_gates_check_repo_reality() {
    let root = scratch("gates");
    seed_repo(&root);
    let good = plan();
    assert!(
        plan_gates::check(&good, &root).is_empty(),
        "existing paths, greppable symbols"
    );
    fs::remove_file(root.join("b.py")).unwrap();
    let findings = plan_gates::check(&good, &root);
    assert!(findings
        .iter()
        .any(|f| f.child == "CARD-B" && f.what.contains("missing")));
    fs::write(root.join("b.py"), "def nothing():\n    pass\n").unwrap();
    let findings: Vec<Finding> = plan_gates::check(&good, &root);
    assert!(
        findings
            .iter()
            .any(|f| f.child == "CARD-B" && f.what.contains("bar")),
        "symbol must be greppable in its own paths"
    );
}

#[test]
fn dispatch_stamps_strategy_into_events() {
    let root = scratch("strategy");
    seed_repo(&root);
    let mut w = walking(&root);
    w.accept_plan("PLAN-T", &plan()).unwrap();
    assert!(w
        .dispatch_as(
            "CARD-A",
            &pass_exec(),
            &Lane::Weak("sim".into()),
            "weak-first",
            0
        )
        .unwrap());
    let st = TreeState::load(root.join("goal.jsonl"), caps()).unwrap();
    assert!(
        st.events().iter().any(|e| matches!(
            &e.kind,
            EventKind::LeafDispatch { strategy, .. } if strategy == "weak-first"
        )),
        "strategy is stamped per dispatch"
    );
}

#[test]
fn dispatch_records_context_bytes() {
    // CTXROT-1 RED: the LeafDispatch event carries the measured context
    // byte sum (card + anchors + annex) beside cost — telemetry the
    // replay can count on, not an estimate.
    let root = scratch("ctxbytes");
    seed_repo(&root);
    let mut w = walking(&root);
    w.accept_plan("PLAN-T", &plan()).unwrap();
    assert!(w
        .dispatch_as(
            "CARD-A",
            &pass_exec(),
            &Lane::Weak("sim".into()),
            "weak-first",
            4321
        )
        .unwrap());
    let st = TreeState::load(root.join("goal.jsonl"), caps()).unwrap();
    assert!(
        st.events().iter().any(|e| matches!(
            &e.kind,
            EventKind::LeafDispatch {
                context_bytes: 4321,
                ..
            }
        )),
        "the measured byte sum rides the dispatch event"
    );
    let log = fs::read_to_string(root.join("goal.jsonl")).unwrap();
    assert!(
        log.contains("\"context_bytes\":\"4321\""),
        "and survives the log round-trip: {log}"
    );
}

#[test]
fn old_log_without_context_bytes_still_parses() {
    // CTXROT-1 tolerant direction: a v2-era log line (no context_bytes
    // field) replays with the field at 0 — the tree refuses only on seq
    // mismatch, and the codec extends without breaking old logs.
    let root = scratch("ctxold");
    fs::write(root.join("root_red.md"), "assert!(tree_works());\n").unwrap();
    let log = root.join("goal.jsonl");
    fs::write(
        &log,
        concat!(
            "{\"seq\":\"1\",\"writer\":\"w1\",\"kind\":\"goal_intake\",\"root_red\":\"r.md\"}\n",
            "{\"seq\":\"2\",\"writer\":\"w1\",\"kind\":\"plan_accepted\",\"plan\":\"P\",\"children\":\"CARD-A\"}\n",
            "{\"seq\":\"3\",\"writer\":\"w1\",\"kind\":\"subtree_live\",\"parent\":\"P\"}\n",
            "{\"seq\":\"4\",\"writer\":\"w1\",\"kind\":\"leaf_dispatch\",\"card\":\"CARD-A\",",
            "\"attempt\":\"1\",\"cost\":\"5\",\"lane\":\"weak:sim\",\"strategy\":\"weak-first\"}\n",
        ),
    )
    .unwrap();
    let st = TreeState::load(&log, caps()).unwrap();
    assert!(
        st.events().iter().any(|e| matches!(
            &e.kind,
            EventKind::LeafDispatch { context_bytes: 0, card, .. } if card == "CARD-A"
        )),
        "old line parses, missing field reads as 0"
    );
}

#[test]
fn preset_gate_switches_only_after_four() {
    let mut g = PresetGate::new();
    for _ in 0..3 {
        assert_eq!(g.tick(false), "weak-first", "three failures are noise");
    }
    assert_eq!(g.tick(false), "strong-first", "four is a trend");
    assert_eq!(
        g.tick(true),
        "strong-first",
        "accept resets; return is operator-only"
    );
}
