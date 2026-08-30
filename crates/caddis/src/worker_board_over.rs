//! worker_board_over.rs — CARD-0247. Over-capacity context rendering,
//! split from worker_board_sections.rs at the 280-line cap. Same organ,
//! same law: read-only, never spawns, never writes. When the observed
//! omp session's stored tokens exceed its window, the bar MUST show the
//! true ratio (never a clamped serene 100%), and the section MUST name
//! the remedy — a clamp is a lie and a red badge without a next step
//! is noise (operator ruling 2026-08-28).

use crate::worker_board_frame::{self as fr, Frame};
use crate::worker_board_sections::opt;
use crate::worker_board_state::Page;

/// Round(stored * 100 / window) — the unclamped percent the operator
/// sees when over-capacity. None when either side is missing.
pub(crate) fn true_percent(stored: Option<u64>, window: Option<u64>) -> Option<u64> {
    let s = stored?;
    let w = window?;
    if w == 0 {
        return None;
    }
    Some((s.saturating_mul(100) + w / 2) / w)
}

/// Render the OVER-context rows: the true value row + the remedy action.
/// The fold() caller decides WHEN; this is the WHAT.
pub(crate) fn render_over(f: &mut Frame, p: &Page, fallback_pct: u64) {
    let true_pct = true_percent(p.stored, p.window).unwrap_or(fallback_pct);
    f.row(
        "ctx",
        &format!("{true_pct}% OVER ({}/{})", opt(p.stored), opt(p.window)),
        fr::RED,
    );
    f.row(
        "action",
        "-> compact or switch to a larger-window model",
        fr::RED,
    );
}
