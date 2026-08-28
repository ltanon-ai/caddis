//! caddis-router — ROUTING ORGAN P1 (BUILD-QUEUE caddis-router-organ, 2026-08-26).
//!
//! Pure library: [`profile_from_card`] -> [`route`] -> [`RouteDecision`].
//! Every input is DATA (capability rows, policy maps); every output is a
//! value. Ruling provenance per piece:
//!
//! - **F1** pure crate, NO dispatch inside — the decision ROW is consumed by
//!   the existing validated dispatch paths (TinyAGI consult / omp task / bee);
//!   a second dispatcher is forbidden.
//! - **F3** the routing decision is a LEDGER ROW referenced by the task card
//!   (`route_id`), not a card of its own. The router read surface is the card
//!   frontmatter + the two mandatory structured sections (Done-When,
//!   RED-TEST) via caddis-card's own parser — zero free-text classification,
//!   zero LLM (no-LLM ruling 2026-07-26: a model in the control path
//!   hallucinates ids and is prompt-injectable by the task it classifies).
//! - **F5** data-class vocabulary Secret|Pii|Internal|Public; tier
//!   allowlist per class is POLICY DATA; no permitted lane alive -> FAIL
//!   CLOSED (the P4 warden integration adds the operator alert).
//! - **F6** floors are DATA with prior defaults (skeptic 0.85, chair 0.70);
//!   floor changes require operator sign-off (enforced outside P1).
//! - **F2** a lane enters the cheap-selection pool only with N >= 5 measured
//!   runs for the class (cold-start lanes are honest errors here, family
//!   median cold-start belongs to the P2 capability ledger).
//! - **O3** among suitable lanes (quality >= floor) pick the CHEAPEST;
//!   ties break Local > Free > Mid > Premium (free/local first).
//! - **O2** NO droid lanes — unenforceable by decree alone, so
//!   [`LaneTier::parse`] rejects `"droid"` and there is no variant for it.
//!
//! NOT in P1 (quorum-gated QQ1-3 or later phases): escalation state machine
//! (P3), capability ledger + EWMA (P2), warden policy wiring (P4), in-world
//! room (P5).

pub mod lane;
pub mod policy;
pub mod profile;
pub mod route;

pub use lane::{Capability, DataClass, Lane, LaneTier};
pub use policy::RoutePolicy;
pub use profile::{ProfileErr, TaskProfile};
pub use route::{route, RouteDecision, RouteErr};

pub const VERSION: &str = "0.1.0";
