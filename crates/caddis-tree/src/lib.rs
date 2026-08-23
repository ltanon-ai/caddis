//! caddis-tree — BC3 (card-tree quorum 2026-08-23): the goal-tree organ.
//! tree-state is an append-only event jsonl with atomic temp-rename
//! writes, monotonic seq (refuse-on-mismatch), a single writer (the
//! orchestrating session), and GLOBAL attempt/cost caps per goal; the
//! walker resumes any tree from its file alone. The crate is deliberately
//! substrate-agnostic: the dispatch substrate is NAMED as a trait
//! (walker::LeafExecutor) — today's live substrate is the orchestrating
//! session doing one-shot executor-lane calls with mechanical gates;
//! ladder.py stays profiles-only telemetry.

mod codec;
pub mod event;
pub mod plan_gates;
pub mod state;
pub mod walker;

pub use event::{Caps, EventKind, Lane, StateErr, TreeEvent};
pub use state::TreeState;
pub use walker::{Action, LeafExecutor, Outcome, SimExecutor, Walker};
