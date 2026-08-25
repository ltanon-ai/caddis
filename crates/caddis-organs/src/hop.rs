//! hop.rs — what a canary HOP is, and how a run of them is judged.
//! Split out of canary.rs under the 280-line law.
//!
//! The seam is data versus procedure: this module knows the shape of a hop
//! and the rule for combining hops into a verdict, and it knows nothing about
//! which organs get exercised or in what order. That is why `aggregate` can
//! be tested on hand-built hops with no filesystem in sight — the rule it
//! encodes (RED wins, DEGRADED never escalates) is the canary's whole law,
//! and it deserves to sit where nothing can drag IO into it.

/// The three verdicts a hop can carry. DEGRADED is deliberately not a
/// failure: an unreachable external lane is not a broken chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HopStatus {
    Ok,
    Degraded,
    Red,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Hop {
    /// 1-based hop number (the chain order is part of the contract).
    pub hop: u8,
    pub name: &'static str,
    pub status: HopStatus,
    pub ms: Option<u64>,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CanaryResult {
    pub ts: String,
    pub status: HopStatus,
    pub red_count: u32,
    pub degraded_count: u32,
    pub hops: Vec<Hop>,
}

/// Exact port of types.ts `aggregate`: RED wins, DEGRADED never escalates.
pub fn aggregate(hops: &[Hop]) -> (HopStatus, u32, u32) {
    let red = hops.iter().filter(|h| h.status == HopStatus::Red).count() as u32;
    let degraded = hops
        .iter()
        .filter(|h| h.status == HopStatus::Degraded)
        .count() as u32;
    (
        if red > 0 {
            HopStatus::Red
        } else {
            HopStatus::Ok
        },
        red,
        degraded,
    )
}

/// The host model lane: prompt in, reply text out (Err = lane down).
pub type ModelLane = Box<dyn FnMut(&str) -> Result<String, String>>;

/// Host-provided external lanes. None = not wired (DEGRADED, never RED).
pub struct HostHooks {
    /// Probe the model lane: given a token prompt, return the reply text.
    pub model_call: Option<ModelLane>,
}

impl HostHooks {
    pub fn none() -> Self {
        HostHooks { model_call: None }
    }
}

pub(crate) fn ok(hop: u8, name: &'static str, ms: Option<u64>, detail: &str) -> Hop {
    Hop {
        hop,
        name,
        status: HopStatus::Ok,
        ms,
        detail: detail.to_string(),
    }
}
pub(crate) fn degraded(hop: u8, name: &'static str, detail: &str) -> Hop {
    Hop {
        hop,
        name,
        status: HopStatus::Degraded,
        ms: None,
        detail: detail.to_string(),
    }
}
pub(crate) fn red(hop: u8, name: &'static str, detail: &str, ms: Option<u64>) -> Hop {
    Hop {
        hop,
        name,
        status: HopStatus::Red,
        ms,
        detail: detail.to_string(),
    }
}
