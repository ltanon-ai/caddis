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

use std::io::Write;
use std::path::Path;

use caddis_core::envelope;
use caddis_core::idempotency::Idempotency;
use caddis_core::ledger::Ledger;
use caddis_core::policy;

use crate::checkpoint::CheckpointStore;
use crate::util::{iso8601_now, json_escape};
use crate::watchdog::{list_open_blockers, ProbeAction, Watchdog};

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

fn ok(hop: u8, name: &'static str, ms: Option<u64>, detail: &str) -> Hop {
    Hop {
        hop,
        name,
        status: HopStatus::Ok,
        ms,
        detail: detail.to_string(),
    }
}
fn degraded(hop: u8, name: &'static str, detail: &str) -> Hop {
    Hop {
        hop,
        name,
        status: HopStatus::Degraded,
        ms: None,
        detail: detail.to_string(),
    }
}
fn red(hop: u8, name: &'static str, detail: &str, ms: Option<u64>) -> Hop {
    Hop {
        hop,
        name,
        status: HopStatus::Red,
        ms,
        detail: detail.to_string(),
    }
}

/// Run the 11-hop golden path in `workdir` (scratch; state.json lands there).
pub fn run_canary(workdir: &Path, hooks: &mut HostHooks) -> CanaryResult {
    let mut hops: Vec<Hop> = Vec::with_capacity(11);
    let ts = iso8601_now();
    std::fs::create_dir_all(workdir).ok();

    // Hop 1 — heartbeat: this run IS the heartbeat firing the canary.
    hops.push(ok(1, "heartbeat", Some(0), "canary work-item dispatched"));

    // Hop 2 — envelope: strict form accepts a good frame, rejects a bad one.
    let env_ok = envelope::validate(
        1,
        "canary-0001",
        "idem-canary-1",
        "signal/canary.hop",
        "canary",
        "ledger",
        "{\"hop\":2}",
        &ts,
    );
    let env_bad = envelope::validate(9, "canary-0001", "k", "t", "a", "b", "x", &ts);
    let env = match (&env_ok, env_bad) {
        (Ok(e), Err(_)) => {
            hops.push(ok(
                2,
                "envelope",
                None,
                "accepts strict form, rejects bad version",
            ));
            e.clone()
        }
        (Ok(_), Ok(_)) => {
            hops.push(red(2, "envelope", "validator accepted a bad version", None));
            return finalize(workdir, &ts, hops);
        }
        (Err(e), _) => {
            hops.push(red(2, "envelope", &format!("{}: {}", e.code, e.why), None));
            return finalize(workdir, &ts, hops);
        }
    };

    // Hop 3 — policy: the LAW is alive when it denies what v0 forbids.
    let d = policy::decide(&env);
    hops.push(if !d.allow && d.reason.starts_with("E-POLICY") {
        ok(3, "policy", None, "denies non-admitted type (E-POLICY)")
    } else {
        red(
            3,
            "policy",
            &format!("policy did not deny as v0 law requires: {}", d.reason),
            None,
        )
    });

    // Hop 4 — idempotency: first key passes, the replay is caught.
    let mut idem = Idempotency::new();
    let first = idem.check(&env.idem_key);
    let replay = idem.check(&env.idem_key);
    hops.push(match (first, replay) {
        (Ok(()), Err(e)) if e.starts_with("E-IDEM") => {
            ok(4, "idempotency", None, "replay detected (E-IDEM)")
        }
        _ => red(4, "idempotency", "replay was not detected", None),
    });

    // Hop 5 — ledger append on the real substrate.
    let ledger_path = workdir.join("canary-ledger.jsonl");
    let mut ledger = match Ledger::open(&ledger_path) {
        Ok(l) => l,
        Err(e) => {
            hops.push(red(5, "ledger-append", &e.to_string(), None));
            return finalize(workdir, &ts, hops);
        }
    };
    hops.push(match ledger.append(&env) {
        Ok(seq) if seq >= 1 => ok(5, "ledger-append", None, &format!("seq {seq}")),
        Ok(_) => red(5, "ledger-append", "seq did not advance", None),
        Err(e) => red(5, "ledger-append", &e.to_string(), None),
    });

    // Hop 6 — ledger read-back: reopen the file; every row must be intact.
    let reopened = Ledger::open(&ledger_path);
    hops.push(match reopened {
        Ok(l) if l.unreadable() == 0 && l.seq() >= 1 => {
            ok(6, "ledger-readback", None, "all rows intact")
        }
        Ok(l) => red(
            6,
            "ledger-readback",
            &format!("unreadable rows: {}", l.unreadable()),
            None,
        ),
        Err(e) => red(6, "ledger-readback", &e.to_string(), None),
    });

    // Hop 7 — checkpoint self-undo: snapshot, mutate, restore, compare.
    hops.push(checkpoint_roundtrip(workdir, &ledger_path));

    // Hop 8 — watchdog self-probe: the probe/restart/blocker state machine.
    hops.push(watchdog_selfprobe(workdir));

    // Hop 9 — host model lane (optional; absent = DEGRADED, never halts).
    hops.push(host_lane(hooks, &ts));

    // Hop 10 — dashboard: state.json write (done in finalize; assert writable).
    let state_path = workdir.join("state.json");
    hops.push(match std::fs::File::create(&state_path) {
        Ok(_) => ok(10, "dashboard", None, "state.json writable"),
        Err(e) => red(10, "dashboard", &e.to_string(), None),
    });

    // Hop 11 — cleanup: scratch ledger removed; state.json stays.
    hops.push(match std::fs::remove_file(&ledger_path) {
        Ok(()) => ok(11, "cleanup", None, "scratch ledger removed"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            ok(11, "cleanup", None, "nothing to clean")
        }
        Err(e) => red(11, "cleanup", &e.to_string(), None),
    });

    finalize(workdir, &ts, hops)
}

/// Hop 7 body: prove the self-undo organ on a REAL file (the canary ledger).
fn checkpoint_roundtrip(workdir: &Path, ledger_path: &Path) -> Hop {
    let store_dir = workdir.join("ckpt-store");
    let store = match CheckpointStore::open(&store_dir) {
        Ok(s) => s,
        Err(e) => return red(7, "checkpoint", &e.to_string(), None),
    };
    let before = match std::fs::read(ledger_path) {
        Ok(b) => b,
        Err(e) => return red(7, "checkpoint", &format!("read ledger: {e}"), None),
    };
    let id = match store.snapshot("canary-hop7", &[ledger_path.to_path_buf()]) {
        Ok(id) => id,
        Err(e) => return red(7, "checkpoint", &e.to_string(), None),
    };
    // The mutation: garbage over the ledger.
    if let Err(e) = std::fs::write(ledger_path, b"{corrupted") {
        return red(7, "checkpoint", &e.to_string(), None);
    }
    if let Err(e) = store.restore(&id) {
        return red(7, "checkpoint", &format!("restore: {e}"), None);
    }
    let after = std::fs::read(ledger_path).unwrap_or_default();
    if before == after {
        ok(
            7,
            "checkpoint",
            None,
            "snapshot->mutate->restore byte-identical",
        )
    } else {
        red(
            7,
            "checkpoint",
            "restore did not reproduce pre-mutation bytes",
            None,
        )
    }
}

/// Hop 8 body: failing service x2 (no blocker), 3rd files blocker, resolve,
/// healthy again — the whole watchdog law in one self-probe.
fn watchdog_selfprobe(workdir: &Path) -> Hop {
    let blockers = workdir.join("canary-blockers.jsonl");
    let _ = std::fs::remove_file(&blockers);
    let mut wd = Watchdog::new("canary-svc", &blockers)
        .health_cmd("exit 1")
        .restart_cmd("exit 0");
    for _ in 0..2 {
        let out = wd.run_probe();
        if !matches!(out.action, ProbeAction::RestartAttempted { .. }) || out.blocker.is_some() {
            return red(
                8,
                "watchdog",
                "failure accounting broken before max_failures",
                None,
            );
        }
    }
    let out = wd.run_probe();
    if out.blocker.is_none() {
        return red(8, "watchdog", "blocker not filed at max_failures", None);
    }
    if wd.run_probe().action != ProbeAction::SkippedOpenBlocker {
        return red(
            8,
            "watchdog",
            "hammering not suspended under open blocker",
            None,
        );
    }
    let open = list_open_blockers(&blockers);
    if open.len() != 1 || open[0].source != "watchdog:canary-svc" {
        return red(8, "watchdog", "blocker not persisted for the source", None);
    }
    if wd.resolve_blockers().unwrap_or(0) != 1 {
        return red(8, "watchdog", "resolve did not clear the blocker", None);
    }
    let wd = wd.health_cmd("exit 0");
    let mut wd = wd;
    if wd.run_probe().action != ProbeAction::Healthy {
        return red(8, "watchdog", "healthy probe failed after resolve", None);
    }
    ok(
        8,
        "watchdog",
        None,
        "probe->restart->blocker->resolve->healthy",
    )
}

/// Hop 9 body: the host's model lane round-trips a canary token.
fn host_lane(hooks: &mut HostHooks, ts: &str) -> Hop {
    let Some(call) = hooks.model_call.as_mut() else {
        return degraded(
            9,
            "host-lane",
            "model lane not wired (host hooks absent) — never halts",
        );
    };
    let token = format!("CANARY-{}", ts.replace([':', '-'], ""));
    let probe = format!("Reply with exactly this token and nothing else: {token}");
    match call(&probe) {
        Ok(reply) if reply.contains("CANARY-") => ok(
            9,
            "host-lane",
            None,
            &format!("reply ok ({} chars)", reply.len()),
        ),
        Ok(reply) => red(
            9,
            "host-lane",
            &format!(
                "reply did not contain token: {}",
                reply.chars().take(120).collect::<String>()
            ),
            None,
        ),
        Err(e) => red(9, "host-lane", &e, None),
    }
}

fn finalize(workdir: &Path, ts: &str, hops: Vec<Hop>) -> CanaryResult {
    let (status, red_count, degraded_count) = aggregate(&hops);
    let result = CanaryResult {
        ts: ts.to_string(),
        status,
        red_count,
        degraded_count,
        hops,
    };
    // Hop 10 actual write (best-effort — the dashboard hop already proved
    // writability; a failure here cannot change the verdict).
    let _ = write_state_json(&workdir.join("state.json"), &result);
    result
}

fn write_state_json(path: &Path, r: &CanaryResult) -> std::io::Result<()> {
    let hops = r
        .hops
        .iter()
        .map(|h| {
            format!(
                "{{\"hop\":{},\"name\":\"{}\",\"status\":\"{}\",\"detail\":\"{}\"}}",
                h.hop,
                json_escape(h.name),
                match h.status {
                    HopStatus::Ok => "OK",
                    HopStatus::Degraded => "DEGRADED",
                    HopStatus::Red => "RED",
                },
                json_escape(&h.detail)
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let doc = format!(
        "{{\"ts\":\"{}\",\"status\":\"{}\",\"redCount\":{},\"degradedCount\":{},\"hops\":[{}]}}",
        json_escape(&r.ts),
        match r.status {
            HopStatus::Ok => "OK",
            HopStatus::Degraded => "DEGRADED",
            HopStatus::Red => "RED",
        },
        r.red_count,
        r.degraded_count,
        hops
    );
    let mut f = std::fs::File::create(path)?;
    f.write_all(doc.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("caddis-canary-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
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
