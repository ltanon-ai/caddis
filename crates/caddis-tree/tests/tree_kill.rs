//! tree_kill.rs — BC3 RED (CARD-0093), part 1: the event log's crash and
//! ownership laws. Kill-mid-tree is the load-bearing test: everything in
//! memory dies, state is rebuilt from the jsonl ALONE, and the walk
//! resumes with no duplicate dispatch. Authored strong-lane at goal
//! intake, before any implementation existed.

mod common;

use caddis_tree::state::{Caps, EventKind, Lane, StateErr, TreeState};
use caddis_tree::walker::Walker;
use common::{caps, pass_exec, plan, scratch, seed_repo, walking};
use std::fs;
use std::io::Write;

#[test]
fn kill_mid_tree_resume_completes_without_duplicate_dispatch() {
    let root = scratch("kill");
    seed_repo(&root);
    let log = root.join("goal.jsonl");
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
    drop(w); // KILL: all memory gone; the jsonl is the only survivor

    let st2 = TreeState::load(&log, caps()).unwrap();
    assert_eq!(st2.writer(), "w1");
    assert_eq!(st2.seq(), 5, "intake+plan+live+dispatch+gated");
    let mut w2 = Walker::new(st2, root.clone());
    let again = w2.dispatch_as(
        "CARD-A",
        &pass_exec(),
        &Lane::Weak("sim".into()),
        "weak-first",
        0,
    );
    assert!(
        matches!(again, Err(StateErr::AlreadyDone)),
        "resume never re-dispatches a gated leaf"
    );
    assert!(w2
        .dispatch_as(
            "CARD-B",
            &pass_exec(),
            &Lane::Weak("sim".into()),
            "weak-first",
            0
        )
        .unwrap());
    let st3 = TreeState::load(&log, caps()).unwrap();
    let a: usize = st3
        .events()
        .iter()
        .filter(|e| matches!(&e.kind, EventKind::LeafDispatch { card, .. } if card == "CARD-A"))
        .count();
    assert_eq!(a, 1, "exactly one dispatch of CARD-A across the kill");
}

#[test]
fn seq_corruption_is_refused_at_load() {
    let root = scratch("seq");
    seed_repo(&root);
    let log = root.join("goal.jsonl");
    let mut w = walking(&root);
    w.accept_plan("PLAN-T", &plan()).unwrap();
    drop(w);
    // hand-corrupt: append an event whose seq jumps past last+1
    let line =
        "{\"seq\":\"99\",\"writer\":\"w1\",\"kind\":\"strong_close\",\"card\":\"X\"}\n".to_string();
    fs::OpenOptions::new()
        .append(true)
        .open(&log)
        .unwrap()
        .write_all(line.as_bytes())
        .unwrap();
    assert!(matches!(
        TreeState::load(&log, caps()),
        Err(StateErr::SeqMismatch)
    ));
}

#[test]
fn second_writer_is_refused() {
    let root = scratch("writer");
    seed_repo(&root);
    let log = root.join("goal.jsonl");
    let w = walking(&root);
    drop(w);
    assert!(matches!(
        TreeState::load_as(&log, "w2", caps()),
        Err(StateErr::WriterConflict)
    ));
}

#[test]
fn global_goal_caps_refuse_further_dispatch() {
    let root = scratch("caps");
    seed_repo(&root);
    let log = root.join("goal.jsonl");
    let tight = Caps {
        max_attempts: 1,
        max_cost: 1000,
    };
    let st = TreeState::new(&log, "w1", tight).unwrap();
    let mut w = Walker::new(st, root.clone());
    w.intake("root_red.md").unwrap();
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
    let over = w.dispatch_as(
        "CARD-B",
        &pass_exec(),
        &Lane::Weak("sim".into()),
        "weak-first",
        0,
    );
    assert!(
        matches!(over, Err(StateErr::CapAttempts)),
        "attempts are GLOBAL per goal, not per leaf"
    );

    let root2 = scratch("cost");
    seed_repo(&root2);
    let st2 = TreeState::new(
        root2.join("goal.jsonl"),
        "w1",
        Caps {
            max_attempts: 10,
            max_cost: 7,
        },
    )
    .unwrap();
    let mut w2 = Walker::new(st2, root2.clone());
    w2.intake("root_red.md").unwrap();
    w2.accept_plan("PLAN-T", &plan()).unwrap();
    assert!(w2
        .dispatch_as(
            "CARD-A",
            &pass_exec(),
            &Lane::Weak("sim".into()),
            "weak-first",
            0
        )
        .unwrap());
    let costly = w2.dispatch_as(
        "CARD-B",
        &pass_exec(),
        &Lane::Weak("sim".into()),
        "weak-first",
        0,
    );
    assert!(
        matches!(costly, Err(StateErr::CapCost)),
        "5+5 > 7: cost is cumulative per goal"
    );
}
