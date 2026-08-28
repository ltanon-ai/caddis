//! caddis-router — ROUTING ORGAN (BUILD-QUEUE caddis-router-organ, 2026-08-26).
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
//!   runs for the class; P2 adds the cold-start family median (ADVISORY —
//!   [`stats::CapsReport::p1_caps`] feeds route() OWN measurements only).
//! - **O3** among suitable lanes (quality >= floor) pick the CHEAPEST;
//!   ties break Local > Free > Mid > Premium (free/local first).
//! - **O2** NO droid lanes — unenforceable by decree alone, so
//!   [`LaneTier::parse`] rejects `"droid"` and there is no variant for it.
//!
//! P2 (2026-08-28): [`ledger`] decision+outcome stream (R6 append-only,
//! O_EXCL lock + fsync, fail-closed), [`stats`] EWMA/floors/cold-start,
//! [`verify`] honest findings, [`collect`] the retroactive collector over
//! the council-consult archive (slice 3a: bee-card source landed — the
//! TinyAGI-history source follows).
//!
//! P3 (2026-08-28): [`escalation`] state machine (O2/F4: RED-TEST fail ->
//! smallest measured rung strictly above, REDO never resume, MAX_HOPS=3),
//! R1 static per-class budget ceilings in [`policy`] (no ceiling = fail
//! closed), QQ2/R9 decay+hysteresis wired through
//! [`stats::HYSTERESIS_FAILS`] into [`route`] selection and escalation
//! rungs (one pass heals; floor guards re-entry).
//!
//! P4 (2026-08-28, slice 1): [`alerts`] — the alert organ (`alerts.jsonl`,
//! same append law as the ledger) + the R2/R4 transition scan: persistent
//! decay becomes a `promotion` ledger row and an operator alert, idempotent
//! by prefix; [`Alert::from_escalation_stop`] is the surface the dispatch
//! adapters will call on every P3 fail-safe halt.
//!
//! P4 slice 3 (2026-08-28): [`gate`] — the DISPATCH-PATH GATE. The one
//! surface the real dispatch paths (TinyAGI consult / omp task / bee card
//! consume) call: `route_gated` persists the decision row (F3) before the
//! caller may dispatch and announces degraded runs first (R4);
//! `escalate_gated` persists the stop alert on every escalation refusal.
//! Both stop alerts + the success-row law make the routing event REAL in
//! the organ's streams before any work runs. Still ahead: the lane
//! registry + CLI consumption surface, R5 identity (P4 remainder),
//! in-world room (P5).
//!
//! P4 slice 4 (2026-08-28, council mistral+cartographer): [`registry`] —
//! the OPERATOR-AUTHORED lane universe (`lanes.jsonl`, JSONL flat objects,
//! static-until-ruled, Q1/Q2 unanimous; entry = id|family|tier|cost only —
//! alive is the caller's probe, caps are ledger-derived, Q4) + the
//! `route-gated` CLI: the SUBPROCESS consumption surface (versioned
//! stdout JSON; exit 0 routed / 1 refused / 2 usage-or-defect; liveness
//! via `--alive` or the NAMED `--assume-alive` assumption, Q3 — silence
//! is never consent). R5 identity next; in-world room is P5.

pub mod alerts;
pub mod collect;
pub mod escalation;
pub mod gate;
pub mod lane;
pub mod ledger;
pub mod lock;
pub mod policy;
pub mod policy_file;
pub mod profile;
pub mod registry;
pub mod route;
pub mod stats;
pub mod verify;

pub use gate::{Gate, GateErr};

pub use alerts::{
    run_scan, transitions, Alert, AlertErr, AlertKind, AlertRow, Alerts, LoadedAlerts, ScanErr,
    ScanPlan, ScanReport, Transition,
};
pub use collect::{
    collect_bees, collect_councils, collect_tinyagi, BeeLane, BeeReport, CollectErr, CollectReport,
    SeatDispatch, TinyagiReport, TASK_CLASS_BEE, TASK_CLASS_CONSULT, TASK_CLASS_TINYAGI,
};
pub use escalation::{escalate, Escalation, EscalationCtx, EscalationErr, MAX_HOPS};
pub use lane::{Capability, DataClass, Lane, LaneTier};
pub use ledger::{DecisionRow, Ledger, LedgerErr, Loaded, Outcome, OutcomeRow, ParsedRow, Row};
pub use policy::RoutePolicy;
pub use policy_file::{encode_policy, load_policy, parse_policy, PolicyFileErr};
pub use profile::{profile_from_card, ProfileErr, TaskProfile};
pub use registry::{load_registry, parse_registry, LaneEntry, LaneRegistry, RegistryErr};
pub use route::{route, RouteDecision, RouteErr};
pub use stats::{CapsReport, LaneCap, EWMA_ALPHA, HYSTERESIS_FAILS, MIN_SAMPLES};
pub use verify::{verify_path, Finding, VerifyReport};

pub const VERSION: &str = "0.8.0";
