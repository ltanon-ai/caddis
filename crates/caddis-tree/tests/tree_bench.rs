//! tree_bench.rs — BC5 RED (CARD-0095): the acceptance bench. The leaf
//! gate is the TARGET repo's REAL checker (a spawned process), the
//! driver applies the failure map end-to-end, and the honest columns are
//! counted: goals attempted, first-attempt green, bubble-ups, strong
//! closures. The bench MUST exercise a bubble-up and a dead-end closure.

mod common;

use caddis_tree::bench::{walk_goal, BenchCols, Checker, CheckerExecutor};
use caddis_tree::walker::LeafExecutor;
use common::{plan, scratch, seed_repo, walking};

/// A real process that always exits 0 / 1 — proof the gate is REAL.
fn real_checker(cwd: &std::path::Path, ok: bool) -> Checker {
    let code = if ok { "0" } else { "1" };
    Checker::new(
        if cfg!(windows) { "cmd" } else { "sh" },
        if cfg!(windows) {
            vec!["/C".into(), format!("exit {code}")]
        } else {
            vec!["-c".into(), format!("exit {code}")]
        },
        cwd.to_path_buf(),
    )
}

#[test]
fn checker_is_a_real_process_gate() {
    let root = scratch("chk");
    assert!(real_checker(&root, true).run(), "exit 0 passes");
    assert!(!real_checker(&root, false).run(), "exit 1 fails");
    let exec = CheckerExecutor::new(real_checker(&root, true), 3);
    use caddis_tree::walker::Outcome;
    assert_eq!(
        exec.dispatch("X", 1),
        Outcome {
            pass: true,
            cost: 3
        }
    );
}

#[test]
fn green_goal_counts_first_attempt_green() {
    let root = scratch("bgreen");
    seed_repo(&root);
    let mut w = walking(&root);
    w.accept_plan("PLAN-T", &plan()).unwrap();
    let mut cols = BenchCols::default();
    walk_goal(
        &mut w,
        &["CARD-A", "CARD-B"],
        &CheckerExecutor::new(real_checker(&root, true), 3),
        &mut cols,
    )
    .unwrap();
    assert_eq!(
        cols.first_attempt_green, 1,
        "whole goal green on first attempts"
    );
    assert_eq!(cols.bubble_ups, 0);
    assert_eq!(cols.strong_closures, 0);
}

/// CARD-A never passes: 3 attempts, bubble-up, one replan, then the
/// sibling still fails — the dead end closes STRONG.
struct PoisonExecutor {
    ok_cards: Vec<&'static str>,
}

impl LeafExecutor for PoisonExecutor {
    fn dispatch(&self, card: &str, _attempt: u32) -> caddis_tree::walker::Outcome {
        let pass = self.ok_cards.contains(&card);
        caddis_tree::walker::Outcome { pass, cost: 1 }
    }
}

#[test]
fn bench_exercises_bubble_up_and_dead_end_closure() {
    let root = scratch("bdead");
    seed_repo(&root);
    let mut w = walking(&root);
    w.accept_plan("PLAN-T", &plan()).unwrap();
    let mut cols = BenchCols::default();
    // goal 1: CARD-A poisoned -> 3 fails -> bubble-up ends the attempt
    walk_goal(
        &mut w,
        &["CARD-A"],
        &PoisonExecutor { ok_cards: vec![] },
        &mut cols,
    )
    .unwrap();
    assert_eq!(cols.bubble_ups, 1, "the bubble-up was exercised");
    assert_eq!(cols.strong_closures, 0);

    // goal 2 (fresh tree, parent already replanned): every leaf that
    // exhausts retries now closes STRONG — the dead end
    let root2 = scratch("bdead2");
    seed_repo(&root2);
    let mut w2 = walking(&root2);
    w2.accept_plan("PLAN-T", &plan()).unwrap();
    w2.record_bubble_up("CARD-A", "PLAN-T").unwrap(); // the one replan is spent
    let mut cols2 = BenchCols::default();
    walk_goal(
        &mut w2,
        &["CARD-A"],
        &PoisonExecutor { ok_cards: vec![] },
        &mut cols2,
    )
    .unwrap();
    assert_eq!(
        cols2.strong_closures, 1,
        "the dead-end closure was exercised"
    );
    assert_eq!(cols2.first_attempt_green, 0);
}
