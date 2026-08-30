//! caddis-bitynas — the BITYNAS lease organ core (CARD-BITYNAS-1): GPU-pool
//! slot leases over an append-only `pool/leases.jsonl` journal, with TTL
//! preemption that is never silent, and the O2 lane guard (no droid lanes).
//! CARD-BITYNAS-6 adds TYPED resources — Council, Quorum (same question
//! JOINs one consultation) and Voice — on the same atom and journal via
//! [`ResourceType`]/[`LeaseStore::claim_typed`], bees staying in their
//! bare H-1 namespace.
//!
//! Time math comes from `caddis-organs::util_time` and journal rows from
//! `serde_json` (orchestrator steering 2026-08-30: workspace deps allowed).
//! The lane vocabulary mirrors `caddis-router/src/lane.rs LaneTier::parse`
//! by card order — do NOT depend on the router crate; copies law: if the
//! vocabulary ever moves, the fix lands in BOTH copies.
//!
//! # The acceptance example
//!
//! The second claim of a live slot is refused and the error carries the
//! holder's identity — a squatter is named, never just "busy":
//!
//! ```
//! use caddis_bitynas::{LeaseOwner, LeaseStore};
//!
//! let nanos = std::time::SystemTime::now()
//!     .duration_since(std::time::UNIX_EPOCH)
//!     .unwrap()
//!     .subsec_nanos();
//! let path = std::env::temp_dir().join(format!("bitynas-doc-{}-{nanos}.jsonl", std::process::id()));
//! let mut store = LeaseStore::open(&path).unwrap();
//!
//! let a = LeaseOwner { session_id: "ses-A".into(), host: "host-a".into(), pid: 111 };
//! let b = LeaseOwner { session_id: "ses-B".into(), host: "host-b".into(), pid: 222 };
//! assert!(store.claim("gpu-0", "premium", a).is_ok());
//!
//! // gpu-0 is live — the second claim MUST fail, naming the holder:
//! let err = store.claim("gpu-0", "premium", b).unwrap_err();
//! assert_eq!(err.holder.session_id, "ses-A");
//! assert_eq!(err.holder.host, "host-a");
//! assert_eq!(err.holder.pid, 111);
//!
//! let _ = std::fs::remove_file(&path);
//! ```

pub mod lease;
pub mod resource;
pub mod store;

mod journal;
mod lane;

pub use lane::lane_allowed;
pub use lease::{BusyError, LeaseOwner, LeaseRecord, PeremptionEvent, DEFAULT_TTL_S};
pub use resource::{ClaimOutcome, ResourceType, COUNCIL_TTL_S, QUORUM_TTL_S, VOICE_TTL_S};
pub use store::LeaseStore;
