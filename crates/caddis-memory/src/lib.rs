//! caddis-memory — the MEMORY ORGAN: P1 read-only Recall API + P2 refresh watchdog.
//!
//! Origin: OMP sergeant BUILD-QUEUE `caddis-memory-organ` (2026-08-26). Spec =
//! state/briefs/caddis-memory-system-council/CONVENING.md — council 2/3 +
//! live fact-check, medium weight, quorum not required for the read path.
//!
//! Laws carried in code:
//! - **Read-only v1 (Q2, ratified 3/3):** recall risk and write risk never
//!   ship together. The write path (`caddis-remember`, P3) is quorum-gated
//!   as its own brief and does not exist here.
//! - **CLI wrap v1 (Q1):** qmd is driven through its CLI behind this crate's
//!   seam; swapping to a persistent interface later is internal.
//! - **Env sanitization (hard spec, fact-check row 3):** the inherited
//!   environment carries `CI=true` under this harness, which makes qmd refuse
//!   LLM operations. Every spawn scrubs it ([`exec::should_strip`]).
//! - **Lane budgets (Q1 latency spec):** fast lane 5 s (`search`, `get`),
//!   deep lane 60 s (`query`, measured 15–21 s live).
//! - **Fail-closed:** timeout, spawn failure, nonzero exit, or unparseable
//!   output is an error — never an empty-result success.
//! - **Ungated reads (Q7, ratified 3/3):** v1 reads are local and
//!   side-effect-free; the warden gates writes, not recall.
//! - **Telemetry ships with v1 (Q1):** every call returns its [`Report`].
//!
//! P2 (this version): the refresh watchdog ([`refresh`]) and the organ-owned
//! collection registry ([`registry`]) — both live-proven against the real
//! machine index (`tests/live_probe.rs`, `--ignored`).

pub mod canary;
pub mod exec;
pub mod json;
pub mod parse;
pub mod recall;
pub mod refresh;
pub mod registry;
pub mod remember;
pub mod sha256;
pub use canary::{Golden, Verdict};
pub use exec::{Job, Outcome, Runner};
pub use parse::{GetDoc, Hit};
pub use refresh::{
    CollectionStatus, LockState, RefreshConfig, RefreshError, RefreshVerdict, StatusSnapshot,
    StepTrace,
};
