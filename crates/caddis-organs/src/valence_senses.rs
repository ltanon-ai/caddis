//! valence_senses.rs — CARD-0254. The seven-sense structs, the
//! telemetry input snapshots, and the pure sense composers, split
//! out of valence.rs under the 280-line law.
//!
//! The seven senses map to existing caddis telemetry:
//! - Proprioception: stored_pct from pager observe.
//! - TimeSense: delta-nerve elapsed + event count.
//! - Interoception: last-3-turn error/warning rate from gates.
//! - TouchSense: file size, CCN, test count from gate census.
//! - SmellSense: cache_health + eviction frequency trends.
//! - EmpathySense: bee statuses from the board.
//! - Conation: goal urgency + pace verdict from tree.
//!
//! Zero deps. The telemetry snapshots are OURS — the host fills them
//! from the existing nerves. No cross-crate types: caddis-organs must
//! not depend on caddis (circular) or caddis-tree.

use crate::eddy::{StatusClass, Tick};

// ── the seven senses ────────────────────────────────────────────────

/// Proprioception: the pager's stored_pct (how full the page is).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Proprioception {
    pub stored_pct: u64,
    pub stored_tokens: u64,
}

/// TimeSense: delta-nerve elapsed + event count from the eddy ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TimeSense {
    pub elapsed_ms: u64,
    pub event_count: u64,
}

/// Interoception: last-3-turn error/warning rate from gates (0..100).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Interoception {
    pub error_rate: u64,
}

/// TouchSense: file size, max CCN, test count from the gate census.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TouchSense {
    pub file_size: u64,
    pub max_ccn: u32,
    pub test_count: u32,
}

/// SmellSense: cache_health grade (0=collapsed .. 100=warm) + eviction
/// frequency trend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SmellSense {
    pub cache_health: u8,
    pub eviction_trend: u64,
}

/// EmpathySense: bee statuses from the board.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EmpathySense {
    pub bee_count: u32,
    pub fail_count: u32,
}

/// Conation: goal urgency + pace verdict from the tree.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Conation {
    pub goal_urgency: u32,
    pub pace_verdict: String,
}

// ── telemetry input snapshots (host-filled) ────────────────────────

/// The pager observe snapshot: stored_pct, stored_tokens, evicted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PagerObserve {
    pub stored_pct: u64,
    pub stored_tokens: u64,
    pub evicted: u64,
}

/// The gate census snapshot: file size, max CCN, test count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GateCensus {
    pub file_size: u64,
    pub max_ccn: u32,
    pub test_count: u32,
}

/// One bee board tick: the card id and its exit code.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BoardTick {
    pub card: String,
    pub exit: i64,
}

/// The tree pulse: goal urgency (0..10) + pace verdict string.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TreePulse {
    pub goal_urgency: u32,
    pub pace_verdict: String,
}

// ── sense composers (pure) ──────────────────────────────────────────

/// TimeSense: elapsed (ms) from the first to last tick with a clock,
/// plus the event count (tick count).
pub fn time_sense(ticks: &[Tick]) -> TimeSense {
    TimeSense {
        elapsed_ms: elapsed_ms(ticks),
        event_count: ticks.len() as u64,
    }
}

/// Wall-clock span of the tick stream (last ts - first ts). Ticks
/// without a clock (ts_ms 0) contribute nothing.
fn elapsed_ms(ticks: &[Tick]) -> u64 {
    let clocked: Vec<&Tick> = ticks.iter().filter(|t| t.ts_ms > 0).collect();
    if clocked.is_empty() {
        return 0;
    }
    let first = clocked.iter().map(|t| t.ts_ms).min().unwrap_or(0);
    let last = clocked.iter().map(|t| t.ts_ms).max().unwrap_or(0);
    last.saturating_sub(first)
}

/// Interoception: error/warning rate from the trailing ticks + gate
/// census. Fail/Fatal/Unprovable ticks feed the error rate; a CCN at
/// the cap is a warning. Scaled to 0..100.
pub fn interoception(ticks: &[Tick], gates: &GateCensus) -> Interoception {
    let window: usize = 3;
    let trailing = ticks.len().saturating_sub(window);
    let recent = &ticks[trailing..];
    let errs = recent
        .iter()
        .filter(|t| !matches!(t.status_class, StatusClass::Ok))
        .count() as u64;
    let rate = if recent.is_empty() {
        0
    } else {
        (errs * 100) / recent.len() as u64
    };
    let ccn_warn = if gates.max_ccn >= 11 { 100 } else { 0 };
    let rate = rate.max(ccn_warn).min(100);
    Interoception { error_rate: rate }
}

/// SmellSense: cache_health grade (0..100) from the trailing
/// cache_read trend, plus the eviction count as a trend proxy.
pub fn smell_sense(ticks: &[Tick], pager: &PagerObserve) -> SmellSense {
    let window: usize = 3;
    let trailing = ticks.len().saturating_sub(window);
    let recent = &ticks[trailing..];
    let warm = recent.iter().filter(|t| t.cache_read > 0).count() as u64;
    let grade = if recent.is_empty() {
        100
    } else {
        ((warm * 100) / recent.len() as u64).min(100) as u8
    };
    SmellSense {
        cache_health: grade,
        eviction_trend: pager.evicted,
    }
}

/// EmpathySense: count bees and failed bees (exit != 0).
pub fn empathy_sense(board: &[BoardTick]) -> EmpathySense {
    EmpathySense {
        bee_count: board.len() as u32,
        fail_count: board.iter().filter(|b| b.exit != 0).count() as u32,
    }
}
