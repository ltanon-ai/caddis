//! bench.rs — BC5, the acceptance bench. The leaf gate is the TARGET
//! repo's REAL checker — a spawned process whose exit status decides
//! pass/fail, never a simulated verdict. The driver applies the failure
//! map end-to-end (retry <=3 -> bubble-up -> one replan -> strong close)
//! and counts ONLY the honest columns: goals attempted, first-attempt
//! green, bubble-ups, strong closures. Strong-lane-review-only is the
//! SHIPPED DEFAULT until an E2E bench is green — structurally: there is
//! no weak plan-review code path anywhere in the walker, so no silent
//! fallback can exist.

use crate::event::{Lane, StateErr};
use crate::presets::WEAK_FIRST;
use crate::walker::{Action, LeafExecutor, Outcome, Walker};
use std::path::PathBuf;
use std::process::Command;

/// The target repo's real checker: program + args run in cwd; exit
/// status success = pass. This is the contract mid-gate.
pub struct Checker {
    program: String,
    args: Vec<String>,
    cwd: PathBuf,
}

impl Checker {
    pub fn new(program: impl Into<String>, args: Vec<String>, cwd: PathBuf) -> Self {
        Self {
            program: program.into(),
            args,
            cwd,
        }
    }

    pub fn run(&self) -> bool {
        Command::new(&self.program)
            .args(&self.args)
            .current_dir(&self.cwd)
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

/// The bench's leaf executor: the substrate applies the edit, then the
/// REAL checker decides. Cost is charged per dispatch.
pub struct CheckerExecutor {
    checker: Checker,
    cost: u64,
}

impl CheckerExecutor {
    pub fn new(checker: Checker, cost: u64) -> Self {
        Self { checker, cost }
    }
}

impl LeafExecutor for CheckerExecutor {
    fn dispatch(&self, _card: &str, _attempt: u32) -> Outcome {
        Outcome {
            pass: self.checker.run(),
            cost: self.cost,
        }
    }
}

/// The honest-number columns (BC5): nothing else is reported.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct BenchCols {
    pub goals_attempted: u32,
    pub first_attempt_green: u32,
    pub bubble_ups: u32,
    pub strong_closures: u32,
}

enum LeafEnd {
    Green,
    Bubbled,
    Closed,
}

/// Walk one goal's leaves through the failure map. Returns true when the
/// whole goal went green with no retry (the p^N number). A bubble-up or a
/// strong close ends the goal attempt — the replan is a new plan, and a
/// closed subtree never gets silent siblings.
pub fn walk_goal(
    walker: &mut Walker,
    children: &[&str],
    exec: &dyn LeafExecutor,
    cols: &mut BenchCols,
) -> Result<bool, StateErr> {
    cols.goals_attempted += 1;
    let mut first_green = true;
    for card in children {
        match walk_leaf(walker, card, exec, cols)? {
            LeafEnd::Green => {
                if walker.dispatched(card) > 1 {
                    first_green = false;
                }
            }
            _ => return Ok(false),
        }
    }
    if first_green {
        cols.first_attempt_green += 1;
    }
    Ok(first_green)
}

fn walk_leaf(
    walker: &mut Walker,
    card: &str,
    exec: &dyn LeafExecutor,
    cols: &mut BenchCols,
) -> Result<LeafEnd, StateErr> {
    loop {
        let pass = walker.dispatch_as(card, exec, &Lane::Weak("bench".into()), WEAK_FIRST)?;
        if pass {
            return Ok(LeafEnd::Green);
        }
        match walker.on_fail(card) {
            Action::Retry { .. } => continue,
            Action::BubbleUp { from, to } => {
                walker.record_bubble_up(&from, &to)?;
                cols.bubble_ups += 1;
                return Ok(LeafEnd::Bubbled);
            }
            Action::StrongClose { card } => {
                walker.record_strong_close(&card)?;
                cols.strong_closures += 1;
                return Ok(LeafEnd::Closed);
            }
        }
    }
}

/// Per-card command executor: each leaf's outcome comes from ITS OWN
/// command — the live substrate invocation plus the real checker — and
/// the exit status decides. This is how a live bench drives the shipped
/// walker without the crate ever owning the substrate.
pub struct MultiChecker {
    cwd: PathBuf,
    per_card: std::collections::HashMap<String, (String, Vec<String>)>,
}

impl MultiChecker {
    pub fn new(cwd: PathBuf) -> Self {
        Self {
            cwd,
            per_card: std::collections::HashMap::new(),
        }
    }

    pub fn add(&mut self, card: &str, program: &str, args: Vec<String>) {
        self.per_card
            .insert(card.to_string(), (program.to_string(), args));
    }
}

impl LeafExecutor for MultiChecker {
    fn dispatch(&self, card: &str, _attempt: u32) -> Outcome {
        let pass = self
            .per_card
            .get(card)
            .map(|(p, a)| {
                Command::new(p)
                    .args(a)
                    .current_dir(&self.cwd)
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false)
            })
            .unwrap_or(false);
        Outcome { pass, cost: 1 }
    }
}
