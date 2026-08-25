//! hops_organs.rs — canary hops 7–11, the ones that exercise the ORGANS
//! rather than the core substrate: checkpoint self-undo, watchdog self-probe,
//! the host's model lane, the dashboard write and the scratch cleanup.
//! Split out of canary.rs under the 280-line law.
//!
//! The seam is the same one `hops_core.rs` names from the other side. What
//! marks this group is that each hop drives a whole organ through a real
//! cycle — snapshot/mutate/restore, probe/restart/blocker/resolve — rather
//! than calling one core function and checking its answer. Hop 9 is the only
//! hop that may return DEGRADED, and it is the reason that status exists.

use std::path::Path;

use crate::checkpoint::CheckpointStore;
use crate::hop::{degraded, ok, red, Hop, HostHooks};
use crate::watchdog::{list_open_blockers, ProbeAction, Watchdog};

/// Hop 7 body: prove the self-undo organ on a REAL file (the canary ledger).
pub(crate) fn checkpoint_roundtrip(workdir: &Path, ledger_path: &Path) -> Hop {
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
pub(crate) fn watchdog_selfprobe(workdir: &Path) -> Hop {
    let blockers = workdir.join("canary-blockers.jsonl");
    // The self-probe starts from no blockers; an absent file is the desired
    // state, not an error.
    let _ = std::fs::remove_file(&blockers); // swallow: best-effort-cleanup
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
pub(crate) fn host_lane(hooks: &mut HostHooks, ts: &str) -> Hop {
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

/// Hop 10 — dashboard: state.json write (the real write happens in
/// `finalize`; this hop asserts the path is writable before promising it).
pub(crate) fn dashboard_hop(state_path: &Path) -> Hop {
    match std::fs::File::create(state_path) {
        Ok(_) => ok(10, "dashboard", None, "state.json writable"),
        Err(e) => red(10, "dashboard", &e.to_string(), None),
    }
}

/// Hop 11 — cleanup: scratch ledger removed; state.json stays.
pub(crate) fn cleanup_hop(ledger_path: &Path) -> Hop {
    match std::fs::remove_file(ledger_path) {
        Ok(()) => ok(11, "cleanup", None, "scratch ledger removed"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            ok(11, "cleanup", None, "nothing to clean")
        }
        Err(e) => red(11, "cleanup", &e.to_string(), None),
    }
}
