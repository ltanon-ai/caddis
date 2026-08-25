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

pub mod canary;
pub mod checkpoint;
pub mod util;
pub mod watchdog;

pub const VERSION: &str = "0.1.0";
