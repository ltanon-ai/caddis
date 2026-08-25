//! canary.rs — the golden-path self-test organ (wave 1).
//! Port of the qpi-cli Golden-Path Canary (D29 keystone), re-hosted on the
//! CADDIS substrate: every hop exercises a REAL organ — envelope, policy,
//! idempotency, ledger, checkpoint, watchdog — end to end.
//!
//! Law carried over from the source (run.ts/types.ts):
//! - GREEN = the chain provably works right now;
//! - any RED means the host HALTS the loop (the host decides — this organ
//!   reports, it does not kill);
//! - DEGRADED (unreachable external, absent host lane) NEVER halts.
//!
//! Harness-agnostic by construction: the only external lane (a model probe)
//! is a host-provided closure. Absent closure -> DEGRADED, exactly like the
//! source's "LINEAR_API_KEY not set — not wired" hop.
//!
//! This file is now the CHAIN and nothing else: what the hops are, in what
//! order, and which failures end the run. The hop bodies live in
//! `hops_core.rs` (2–6, the caddis-core substrate) and `hops_organs.rs`
//! (7–11, the organs), the data model in `hop.rs`, the verdict and its
//! `state.json` in `canary_state.rs`. Split under the 280-line law — and the
//! same inlining that made the file long had also pushed `run_canary` to
//! cyclomatic 15, five over the cap.

use std::path::Path;

use crate::canary_state::finalize;
use crate::hop::ok;
use crate::hops_core::{
    envelope_hop, idempotency_hop, ledger_append_hop, ledger_readback_hop, open_ledger, policy_hop,
};
use crate::hops_organs::{
    checkpoint_roundtrip, cleanup_hop, dashboard_hop, host_lane, watchdog_selfprobe,
};
use crate::util::iso8601_now;

// Re-exported so every existing path — `canary::Hop`, `canary::HopStatus`,
// `canary::aggregate`, `canary::HostHooks` — still resolves after the split.
//
// `Hop` is a `pub use` rather than a private `use` because this module needs
// the name in scope AND must keep publishing it. The first draft wrote
// `Hop as CanaryHop` to dodge that collision and left this comment claiming
// the path was preserved — renaming a public symbol to resolve a conflict the
// split itself introduced, which is precisely the forbidden move. Nothing in
// the suite caught it: in-crate tests import via `use super::*` and picked up
// the private alias. `tests/public_paths.rs` now pins these from outside.
pub use crate::hop::{aggregate, CanaryResult, Hop, HopStatus, HostHooks, ModelLane};

/// Run the 11-hop golden path in `workdir` (scratch; state.json lands there).
///
/// Reads as the chain it is: each hop appends its own verdict, and the two
/// hops that can end the run say so with an early return. Nothing here knows
/// HOW a hop is proved — only that it is, and in what order.
pub fn run_canary(workdir: &Path, hooks: &mut HostHooks) -> CanaryResult {
    let mut hops: Vec<Hop> = Vec::with_capacity(11);
    let ts = iso8601_now();
    std::fs::create_dir_all(workdir).ok();

    // Hop 1 — heartbeat: this run IS the heartbeat firing the canary.
    hops.push(ok(1, "heartbeat", Some(0), "canary work-item dispatched"));

    // Hop 2 — envelope. A validator that cannot be trusted ends the run:
    // every later hop is judged against frames it produced.
    let env = match envelope_hop(&ts) {
        Ok((hop, env)) => {
            hops.push(hop);
            env
        }
        Err(hop) => {
            hops.push(hop);
            return finalize(workdir, &ts, hops);
        }
    };

    hops.push(policy_hop(&env));
    hops.push(idempotency_hop(&env));

    // Hop 5 — ledger. Hops 5, 6 and 7 all read this file, so an unopenable
    // ledger leaves nothing further to prove.
    let ledger_path = workdir.join("canary-ledger.jsonl");
    let mut ledger = match open_ledger(&ledger_path) {
        Ok(l) => l,
        Err(hop) => {
            hops.push(hop);
            return finalize(workdir, &ts, hops);
        }
    };
    hops.push(ledger_append_hop(&mut ledger, &env));
    hops.push(ledger_readback_hop(&ledger_path));

    hops.push(checkpoint_roundtrip(workdir, &ledger_path));
    hops.push(watchdog_selfprobe(workdir));
    hops.push(host_lane(hooks, &ts));
    hops.push(dashboard_hop(&workdir.join("state.json")));
    hops.push(cleanup_hop(&ledger_path));

    finalize(workdir, &ts, hops)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("caddis-canary-{name}-{}", std::process::id()));
        // A stale temp dir from a previous run may simply not exist; the
        // test creates it next either way.
        let _ = fs::remove_dir_all(&d); // swallow: best-effort-cleanup
        d
    }

    #[test]
    fn aggregate_red_wins_degraded_never_escalates() {
        let mut hs = vec![
            Hop {
                hop: 1,
                name: "a",
                status: HopStatus::Ok,
                ms: None,
                detail: String::new(),
            },
            Hop {
                hop: 2,
                name: "b",
                status: HopStatus::Degraded,
                ms: None,
                detail: String::new(),
            },
        ];
        assert_eq!(aggregate(&hs), (HopStatus::Ok, 0, 1));
        let hs = vec![
            hs.pop().unwrap(),
            Hop {
                hop: 3,
                name: "c",
                status: HopStatus::Red,
                ms: None,
                detail: String::new(),
            },
        ];
        assert_eq!(aggregate(&hs), (HopStatus::Red, 1, 1));
    }

    #[test]
    fn golden_path_green_without_host_lane() {
        let dir = tmp("green");
        let mut hooks = HostHooks::none();
        let r = run_canary(&dir, &mut hooks);
        assert_eq!(r.status, HopStatus::Ok, "hops: {r:#?}");
        assert_eq!(r.red_count, 0);
        assert!(
            r.degraded_count >= 1,
            "absent host lane is DEGRADED, not silent"
        );
        assert_eq!(r.hops.len(), 11, "the chain is 11 hops");
        // state.json landed and reports OK.
        let state = fs::read_to_string(dir.join("state.json")).unwrap();
        assert!(state.contains("\"status\":\"OK\""), "{state}");
    }

    #[test]
    fn hostile_host_lane_token_mismatch_is_red() {
        let dir = tmp("mismatch");
        let mut hooks = HostHooks {
            model_call: Some(Box::new(|_| Ok("I am a chatty model".to_string()))),
        };
        let r = run_canary(&dir, &mut hooks);
        assert_eq!(r.status, HopStatus::Red);
        assert_eq!(r.red_count, 1);
        assert!(r
            .hops
            .iter()
            .any(|h| h.name == "host-lane" && h.status == HopStatus::Red));
    }

    #[test]
    fn cooperative_host_lane_is_ok() {
        let dir = tmp("coop");
        let mut hooks = HostHooks {
            model_call: Some(Box::new(|p| {
                let token = p.rsplit(' ').next().unwrap_or("");
                Ok(token.to_string())
            })),
        };
        let r = run_canary(&dir, &mut hooks);
        assert_eq!(r.status, HopStatus::Ok, "hops: {r:#?}");
        assert_eq!(r.degraded_count, 0);
    }
}
