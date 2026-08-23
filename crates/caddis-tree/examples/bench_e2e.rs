//! bench_e2e.rs — the LIVE acceptance bench driver (BC5, CARD-0095).
//! Usage: `cargo run -p caddis-tree --example bench_e2e -- <scenario-dir>`.
//!
//! Scenario layout (file-based, no config parsing):
//!   <dir>/root_red.md      the goal's integration RED (intake demands it)
//!   <dir>/plan.md          the PLAN card (validate_plan + plan_gates)
//!   <dir>/cards/<CARD>.py  per-leaf substrate invocation: dispatch the
//!                          executor lane, apply the edit, run the TARGET
//!                          repo's REAL checker; exit status = gate verdict
//!   <dir>/goal.jsonl       the tree log (created; reused on re-runs —
//!                          resume is the point)
//!
//! Prints the honest columns; exit 0 = goal first-attempt green.

use caddis_card::Card;
use caddis_tree::bench::{walk_goal, BenchCols, MultiChecker};
use caddis_tree::state::{Caps, TreeState};
use caddis_tree::walker::Walker;

fn python_bin() -> &'static str {
    if cfg!(windows) {
        "python"
    } else {
        "python3"
    }
}
fn main() {
    let dir = match std::env::args().nth(1) {
        Some(d) => std::path::PathBuf::from(d),
        None => {
            eprintln!("usage: bench_e2e <scenario-dir>");
            std::process::exit(2);
        }
    };
    let log = dir.join("goal.jsonl");
    let caps = Caps {
        max_attempts: 16,
        max_cost: 64,
    };
    let state = if log.is_file() {
        TreeState::load_as(&log, "bench-e2e", caps).expect("resume from goal.jsonl")
    } else {
        TreeState::new(&log, "bench-e2e", caps).expect("fresh log")
    };
    let mut walker = Walker::new(state, dir.clone());
    // On resume the intake already happened (AlreadyIntaked) — that is
    // the kill-mid-tree law, not an error here.
    let _ = walker.intake("root_red.md");
    let text = std::fs::read_to_string(dir.join("plan.md")).expect("plan.md");
    let card = Card::parse(&text).expect("plan parses");
    let plan = card.validate_plan().expect("plan validates");
    let plan_id = card.frontmatter.get("id").cloned().unwrap_or_default();
    let _ = walker.accept_plan(&plan_id, &plan);

    let mut exec = MultiChecker::new(dir.clone());
    let cards_dir = dir.join("cards");
    for entry in std::fs::read_dir(&cards_dir).expect("cards/") {
        let p = entry.expect("entry").path();
        if p.extension().map(|e| e == "py").unwrap_or(false) {
            let card_id = p.file_stem().unwrap().to_string_lossy().into_owned();
            exec.add(
                &card_id,
                python_bin(),
                vec![p.to_string_lossy().into_owned()],
            );
        }
    }
    let children: Vec<String> = plan.children.iter().map(|c| c.id.clone()).collect();
    let refs: Vec<&str> = children.iter().map(|s| s.as_str()).collect();
    let mut cols = BenchCols::default();
    match walk_goal(&mut walker, &refs, &exec, &mut cols) {
        Ok(green) => {
            println!(
                "cols goals_attempted={} first_attempt_green={} bubble_ups={} strong_closures={}",
                cols.goals_attempted,
                cols.first_attempt_green,
                cols.bubble_ups,
                cols.strong_closures
            );
            std::process::exit(if green { 0 } else { 1 });
        }
        Err(e) => {
            eprintln!("bench error: {e:?}");
            std::process::exit(2);
        }
    }
}
