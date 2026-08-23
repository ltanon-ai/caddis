//! walker.rs — the goal-tree walker (BC3). Goal intake REQUIRES a root
//! integration RED authored by the strong lane AT intake; plans pass the
//! crate oracle (validate_plan) plus the repo-reality gates before their
//! subtree goes LIVE; leaves dispatch through the NAMED substrate; the
//! failure map is retry-leaf (<=3) / replan-parent (once) / strong-close;
//! the strong lane never writes under a LIVE subtree — it closes.

use crate::event::{EventKind, Lane, StateErr};
use crate::plan_gates;
use crate::state::TreeState;
use caddis_card::Plan;
use std::collections::HashSet;
use std::path::PathBuf;

/// THE DISPATCH SUBSTRATE (named, BC3): whoever invokes the executor per
/// leaf. Live substrate today = the orchestrating session itself: one-shot
/// call to the executor lane (e.g. Ollama /api/chat) per card with a FRESH
/// context, mechanical gates applied to the output, ladder.py recording
/// capability telemetry (ladder.py is profiles-only by ruling; it never
/// dispatches). In-crate, SimExecutor stands in for tests.
pub trait LeafExecutor {
    fn dispatch(&self, card: &str, attempt: u32) -> Outcome;
}

#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub pass: bool,
    pub cost: u64,
}

/// Deterministic stand-in: repeats the LAST scripted outcome forever.
pub struct SimExecutor {
    script: Vec<Outcome>,
}

impl SimExecutor {
    pub fn new(script: Vec<Outcome>) -> Self {
        Self { script }
    }
}

impl LeafExecutor for SimExecutor {
    fn dispatch(&self, _card: &str, _attempt: u32) -> Outcome {
        self.script.last().cloned().unwrap_or(Outcome {
            pass: false,
            cost: 0,
        })
    }
}

/// The failure-map decision for a gated-failed leaf.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Retry { attempt: u32 },
    BubbleUp { from: String, to: String },
    StrongClose { card: String },
}

pub struct Walker {
    state: TreeState,
    root: PathBuf,
    replanned: HashSet<String>,
}

impl Walker {
    /// Build (or, after a kill, RESUME) a walker: the world-view comes
    /// from the tree-state alone.
    pub fn new(state: TreeState, root: PathBuf) -> Self {
        let mut replanned = HashSet::new();
        for ev in state.events() {
            if let EventKind::BubbleUp { to, .. } = &ev.kind {
                replanned.insert(to.clone());
            }
        }
        Self {
            state,
            root,
            replanned,
        }
    }

    /// Goal intake: the root integration RED must exist on disk — the
    /// strong lane authors it HERE, before any leaf moves.
    pub fn intake(&mut self, root_red: &str) -> Result<(), StateErr> {
        if !self.root.join(root_red).is_file() {
            return Err(StateErr::NoRootRed);
        }
        self.state.append(EventKind::GoalIntake {
            root_red: root_red.to_string(),
        })
    }

    /// Accept a plan: repo-reality gates first (structure was validated
    /// crate-side by the caller), then the subtree goes LIVE.
    pub fn accept_plan(&mut self, plan_id: &str, plan: &Plan) -> Result<(), StateErr> {
        let findings = plan_gates::check(plan, &self.root);
        if !findings.is_empty() {
            let what = findings
                .iter()
                .map(|f| format!("{}: {}", f.child, f.what))
                .collect::<Vec<_>>()
                .join("; ");
            return Err(StateErr::PlanGates(what));
        }
        let children: Vec<String> = plan.children.iter().map(|c| c.id.clone()).collect();
        self.state.append(EventKind::PlanAccepted {
            plan: plan_id.to_string(),
            children,
        })?;
        self.state.append(EventKind::SubtreeLive {
            parent: plan_id.to_string(),
        })
    }

    /// Dispatch one leaf as `lane` through the substrate; returns the gate
    /// verdict. Refuses orphans, duplicates, caps and strong-under-live.
    pub fn dispatch_as(
        &mut self,
        card: &str,
        exec: &dyn LeafExecutor,
        lane: &Lane,
        strategy: &str,
    ) -> Result<bool, StateErr> {
        self.state.can_dispatch()?;
        let attempt = self.state.dispatched(card) + 1;
        let out = exec.dispatch(card, attempt);
        self.state.append(EventKind::LeafDispatch {
            card: card.to_string(),
            attempt,
            cost: out.cost,
            lane: lane.clone(),
            strategy: strategy.to_string(),
        })?;
        self.state.append(EventKind::LeafGated {
            card: card.to_string(),
            pass: out.pass,
        })?;
        Ok(out.pass)
    }

    /// Dispatch count for a leaf (the bench's first-attempt check).
    pub fn dispatched(&self, card: &str) -> u32 {
        self.state.dispatched(card)
    }

    /// The failure map: retry the leaf up to 3 attempts, bubble up to the
    /// parent for ONE replan, then the strong lane closes.
    pub fn on_fail(&self, card: &str) -> Action {
        let attempts = self.state.dispatched(card);
        if attempts < 3 {
            return Action::Retry {
                attempt: attempts + 1,
            };
        }
        if let Some(parent) = self.state.parent_of(card) {
            if !self.replanned.contains(parent) {
                return Action::BubbleUp {
                    from: card.to_string(),
                    to: parent.clone(),
                };
            }
        }
        Action::StrongClose {
            card: card.to_string(),
        }
    }

    /// Record a bubble-up: the child failed past retry, the parent owes a
    /// replan (once).
    pub fn record_bubble_up(&mut self, from: &str, to: &str) -> Result<(), StateErr> {
        self.state.append(EventKind::BubbleUp {
            from: from.to_string(),
            to: to.to_string(),
        })?;
        self.state.append(EventKind::ReplanParent {
            parent: to.to_string(),
            reason: format!("child {from} exhausted 3 attempts"),
        })?;
        self.replanned.insert(to.to_string());
        Ok(())
    }

    /// The strong lane closes a leaf (its ONLY write under a live
    /// subtree): the parent subtree closes with it.
    pub fn record_strong_close(&mut self, card: &str) -> Result<(), StateErr> {
        self.state.append(EventKind::StrongClose {
            card: card.to_string(),
        })?;
        if let Some(parent) = self.state.parent_of(card).cloned() {
            self.state.append(EventKind::SubtreeClosed { parent })?;
        }
        Ok(())
    }
}
