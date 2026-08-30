//! eddy_health.rs — CARD-0241. Cache-ratio as a blocker CANDIDATE:
//! health signal, never a halt. Fires when a run was cache-warm and
//! the trailing window went cold — `_card_0173` measured that
//! `cacheWrite=0 after warmup` IS the monotone-prefix signal, and a
//! loop that eats its own cache burns ~408k input/turn silently.
//!
//! All-cold runs (the measured ollama-cloud shape) report NOTHING:
//! zeros are the provider's answer, not a regression. The window
//! constant is REUSED (STAGNANT_WINDOW) — diagnostics take no new law.

use crate::blocker::{file_blocker, list_open_blockers, Blocker};
use crate::eddy::{Tick, STAGNANT_WINDOW};
use crate::util::iso8601_now;

/// The health report: what collapsed, and where it was last healthy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthReport {
    pub last_warm_seq: u64,
    pub why: String,
}

/// Pure health read over a run's ticks: Some when the run was
/// cache-warm (some tick with cache_read > 0) and the trailing
/// STAGNANT_WINDOW ticks are all cold (cache_read == 0).
pub fn cache_health(ticks: &[Tick]) -> Option<HealthReport> {
    let last_warm = ticks.iter().rposition(|t| t.cache_read > 0)?;
    let cold_window: usize = STAGNANT_WINDOW as usize;
    if ticks.len() < cold_window || last_warm + cold_window > ticks.len() {
        return None; // cold streak shorter than the window: not a phase
    }
    if ticks[ticks.len() - cold_window..]
        .iter()
        .any(|t| t.cache_read > 0)
    {
        return None; // still warm inside the trailing window
    }
    Some(HealthReport {
        last_warm_seq: ticks[last_warm].seq,
        why: format!(
            "cache collapsed: cache_read fell to 0 for {cold_window} ticks after a warm run (last warm seq {})",
            ticks[last_warm].seq
        ),
    })
}

/// File AT MOST ONE eddy-health blocker per run (idempotent on an
/// already-open report for the same run). Health NEVER gates the
/// verdict and NEVER changes an exit code.
pub fn enforce_health(
    run_id: &str,
    report: &HealthReport,
    blocker_path: &std::path::Path,
) -> std::io::Result<()> {
    let source = format!("eddy-health:{run_id}");
    if list_open_blockers(blocker_path)
        .iter()
        .any(|b| b.source == source)
    {
        return Ok(()); // already flagged: one blocker per run
    }
    file_blocker(
        blocker_path,
        &Blocker {
            source,
            reason: report.why.clone(),
            ts: iso8601_now(),
        },
    )
}
