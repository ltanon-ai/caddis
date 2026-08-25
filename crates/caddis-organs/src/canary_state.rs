//! canary_state.rs — turning a finished run of hops into a verdict and a
//! `state.json` on disk. Split out of canary.rs under the 280-line law.
//!
//! The seam is the boundary between JUDGING and PUBLISHING. `finalize` is the
//! only place a `CanaryResult` is born, and `write_state_json` is the only
//! place the run becomes something a dashboard can read. Keeping them
//! together — and away from the hop bodies — means the serialized shape has
//! exactly one author, which is what stops a second writer inventing a
//! slightly different `state.json` later.

use std::io::Write;
use std::path::Path;

use crate::hop::{aggregate, CanaryResult, Hop, HopStatus};
use crate::util::json_escape;

/// The status word as it appears in `state.json`. One spelling, one home —
/// it was written out twice in the original and a reader had to check that
/// both copies agreed.
fn status_word(s: HopStatus) -> &'static str {
    match s {
        HopStatus::Ok => "OK",
        HopStatus::Degraded => "DEGRADED",
        HopStatus::Red => "RED",
    }
}

pub(crate) fn finalize(workdir: &Path, ts: &str, hops: Vec<Hop>) -> CanaryResult {
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
    // state.json feeds a dashboard, and hop 10 has already judged this path
    // writable. A failure here cannot change the verdict the caller is about
    // to receive, and must not suppress it.
    let _ = write_state_json(&workdir.join("state.json"), &result); // swallow: best-effort-telemetry
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
                status_word(h.status),
                json_escape(&h.detail)
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let doc = format!(
        "{{\"ts\":\"{}\",\"status\":\"{}\",\"redCount\":{},\"degradedCount\":{},\"hops\":[{}]}}",
        json_escape(&r.ts),
        status_word(r.status),
        r.red_count,
        r.degraded_count,
        hops
    );
    let mut f = std::fs::File::create(path)?;
    f.write_all(doc.as_bytes())
}
