//! attention.rs — CARD-0244. The attention dividend ledger.
//!
//! Between the attention layer, the memory layers and RAM sits
//! unexploited territory. Every resident span burns money each tick
//! (stored_tokens × turns). This organ CONNECTS the telemetry caddis
//! already holds — pager cold-store spans, observe context events,
//! eddy cache_read/cache_write per tick, loop.epoch rollovers — into
//! economics: which spans earn their keep and which are dead weight.
//!
//! The organ ORDERS; the pager STAYS the executor. `ledger()` is PURE
//! (no I/O): it reads the tick stream (page epochs, cost) and the
//! span-event stream (residency + dividend attribution) and emits
//! [`EvictionOrder`]s for spans that accrued zero dividend across N
//! page epochs. The host (pager nerve) executes an order through the
//! EXISTING eviction path — no second writer, no new lock.
//!
//! Dividend accrues when output citation/usage attributes to the span
//! (a [`SpanEvent::Dividend`]). A [`SpanEvent::Present`] marks a span
//! resident in a page epoch. PINNED spans are never ordered — the
//! pager's own law outranks the organ.
//!
//! The window reuses [`STAGNANT_WINDOW`](crate::eddy_law::STAGNANT_WINDOW):
//! one estate constant, never a second. Cost-per-span per epoch (the
//! "rent") is derivable from the tick stream the host already records.

use std::collections::BTreeMap;

use crate::eddy::Tick;
use crate::eddy_law::STAGNANT_WINDOW;

/// Attribution + residency events for resident spans. The host emits
/// these from the existing observe/message event stream — the organ
/// never reads files itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpanEvent {
    /// A span is resident in `page` (present in the cold store for that
    /// epoch). `pinned` reflects the pager's pin flag at that epoch.
    Present { seq: u64, pinned: bool, page: u64 },
    /// A span earned dividend — its content was cited or used in output
    /// attributable to `page`.
    Dividend { seq: u64, page: u64 },
}

/// One span's economic account: residency cost vs. dividend earned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpanAccount {
    pub seq: u64,
    pub pinned: bool,
    /// Distinct page epochs the span was resident in.
    pub resident_pages: u64,
    /// Total dividend events attributed to this span.
    pub dividend: u64,
}

impl SpanAccount {
    /// Build accounts from the event stream, keyed by span seq.
    /// BTreeMap for deterministic ledger diffs (same house style as
    /// [`crate::deja_vu::AttentionMap`]). `resident_pages` counts
    /// distinct page epochs the span was seen in — Present OR Dividend
    /// (a cited span was present).
    pub fn from_events(events: &[SpanEvent]) -> BTreeMap<u64, SpanAccount> {
        let mut accounts: BTreeMap<u64, SpanAccount> = BTreeMap::new();
        let mut pages: BTreeMap<u64, Vec<u64>> = BTreeMap::new();
        for ev in events {
            let acc = accounts.entry(ev.seq()).or_insert_with(|| SpanAccount {
                seq: ev.seq(),
                pinned: false,
                resident_pages: 0,
                dividend: 0,
            });
            let seen = pages.entry(ev.seq()).or_default();
            match ev {
                SpanEvent::Present { pinned, page, .. } => {
                    if *pinned {
                        acc.pinned = true;
                    }
                    if !seen.contains(page) {
                        seen.push(*page);
                    }
                }
                SpanEvent::Dividend { page, .. } => {
                    acc.dividend += 1;
                    if !seen.contains(page) {
                        seen.push(*page);
                    }
                }
            }
        }
        for (seq, acc) in accounts.iter_mut() {
            acc.resident_pages = pages.get(seq).map_or(0, |v| v.len() as u64);
        }
        accounts
    }
}

/// An order to evict a span, with evidence the host logs on the dash
/// EVENTS feed. The pager nerve executes it via the existing eviction
/// path — the order is advice, not a write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvictionOrder {
    pub seq: u64,
    pub evidence: String,
}

trait SpanSeq {
    fn seq(&self) -> u64;
}

impl SpanSeq for SpanEvent {
    fn seq(&self) -> u64 {
        match self {
            SpanEvent::Present { seq, .. } | SpanEvent::Dividend { seq, .. } => *seq,
        }
    }
}

/// Count distinct page epochs in the tick stream. Page 0 is
/// legacy/unknown (CARD-0242); a single-epoch run counts as 1.
fn distinct_page_epochs(ticks: &[Tick]) -> u64 {
    let mut seen: Vec<u64> = Vec::new();
    for t in ticks {
        if !seen.contains(&t.page) {
            seen.push(t.page);
        }
    }
    seen.len().max(1) as u64
}

/// PURE: order eviction for spans with zero dividend across N page
/// epochs. PINNED spans are never ordered. The window is
/// [`STAGNANT_WINDOW`] — one estate constant, never a second.
pub fn ledger(ticks: &[Tick], events: &[SpanEvent]) -> Vec<EvictionOrder> {
    let epochs = distinct_page_epochs(ticks);
    let accounts = SpanAccount::from_events(events);
    let window = STAGNANT_WINDOW as u64;
    accounts
        .into_values()
        .filter(|a| !a.pinned && a.dividend == 0 && a.resident_pages >= window)
        .map(|a| EvictionOrder {
            seq: a.seq,
            evidence: format!(
                "zero_dividend resident_pages={} epochs={}",
                a.resident_pages, epochs
            ),
        })
        .collect()
}
