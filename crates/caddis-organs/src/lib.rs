//! caddis-organs — ORGANS WAVE 1 (sergeant unit caddis-organs-wave1, 2026-08-25).
//! The self-watching organs, ported harness-agnostic from proven modules:
//!
//! - [`watchdog`] — self-HEAL: probe -> restart -> backoff -> blocker
//!   (TinyAGI watchdog, adoption A3),
//! - [`canary`] — self-TEST: an 11-hop golden path over the CADDIS substrate;
//!   RED halts the host loop, DEGRADED never does (qpi-cli canary, D29),
//! - [`checkpoint`] — self-UNDO: pre-mutation snapshots any host takes
//!   before a write-class action (the unwired QPI checkpoint purpose).
//!
//! The split of authority mirrors the warden law: the organs REPORT and
//! PROVE; the host (harness adapter, heartbeat runner) decides and halts.
//! Zero runtime dependencies beyond caddis-core; sync, std only.

pub mod accept_prefix;
pub mod attention;
pub mod blocker;
pub mod canary;
pub mod canary_state;
pub mod checkpoint;
pub mod cosine_draft;
pub mod deja_vu;
pub mod eddy;
pub mod eddy_arm;
pub mod eddy_health;
pub mod eddy_law;
pub mod eddy_runner;
pub mod hop;
pub mod hops_core;
pub mod hops_organs;
pub mod kv_bridge;
pub mod python_arsenal;
pub mod shell;
pub mod soul;
pub mod util;
pub mod util_time;
pub mod valence;
pub mod valence_mood;
pub mod valence_senses;
pub mod watchdog;

pub const VERSION: &str = "0.1.0";
