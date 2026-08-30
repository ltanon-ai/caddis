//! kv_bridge.rs — CARD-0246. The RAM↔attention bridge.
//!
//! RAM sits BETWEEN the memory layers and the attention layer, and
//! nothing schedules it. The model valve already owns VRAM. The pager
//! already knows which spans are PINNED. The bridge: co-schedule them —
//! the active session's pinned prefix stays resident in the local
//! engine's KV cache, so attention layers never recompute it.
//!
//! The organ is PURE: [`co_schedule`] reads the span-event stream and
//! the valve status, and emits [`KvLease`]s for pinned spans. The HOST
//! side POSTs leases to the model valve (`model_on` with ttl) — the
//! bridge NEVER talks to the engine directly (one writer per resource).
//!
//! Stage-0 finding (CARD-0246 §EXECUTION): the estate's local Ollama
//! does NOT reuse KV cache across separate `/api/generate` calls
//! (`prompt_eval_count` stays constant — see `tools/kv_probe.py`). The
//! bridge's host-side integration retargets to a vLLM/TabbyAPI lane.
//! The PURE organ is delivered regardless: it maps pinned spans to
//! leases sized by token count × bytes_per_token (measured by the host,
//! never guessed). When a KV-reusing engine arrives, the leases are
//! already correct.
//!
//! Laws:
//! - PINNED spans (pager law) map to leases; non-pinned do NOT.
//! - `bytes_est = token_count × bytes_per_token`, where token_count is
//!   the number of DISTINCT page epochs the span was pinned in.
//! - `expires` mirrors the valve's `keep_alive_secs` — the bridge never
//!   invents a ttl.
//! - When `model_loaded == false`, NO leases are produced — the bridge
//!   defers to the valve's state.
//!
//! The window reuses [`STAGNANT_WINDOW`](crate::eddy_law::STAGNANT_WINDOW):
//! one estate constant, never a second.

use std::collections::BTreeMap;

use crate::attention::SpanEvent;

/// The model valve's status — what the host reads from the valve before
/// calling `co_schedule`. The bridge NEVER queries the valve itself
/// (one writer per resource); the host injects this snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValveStatus {
    /// Whether the model is currently loaded in VRAM. When false, no
    /// leases are produced — there is nothing to pin into.
    pub model_loaded: bool,
    /// Engine-specific bytes per KV-cache token, MEASURED by the host
    /// in stage 0 (never guessed). The probe (`tools/kv_probe.py`)
    /// determines whether the engine reuses KV at all; this field
    /// carries the measured bytes/token for engines that do.
    pub bytes_per_token: u64,
    /// The valve's keep_alive window in seconds. Leases mirror this —
    /// the bridge never invents a ttl.
    pub keep_alive_secs: u64,
}

/// One KV-cache lease: a pinned span's prefix stays resident in the
/// engine's KV cache for `expires` seconds. The host POSTs this to the
/// model valve; the bridge never talks to the engine directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KvLease {
    /// The pinned span's identity (its seq from the event stream).
    pub prefix_hash: u64,
    /// Estimated KV-cache bytes: token_count × bytes_per_token.
    pub bytes_est: u64,
    /// When the lease expires (seconds), mirroring the valve's
    /// keep_alive_secs.
    pub expires: u64,
}

/// Count distinct page epochs a span was PINNED in. Each distinct epoch
/// is one prefix segment in the KV cache.
fn pinned_epochs(events: &[SpanEvent], target_seq: u64) -> u64 {
    let mut seen: Vec<u64> = Vec::new();
    for ev in events {
        if let SpanEvent::Present {
            seq,
            pinned: true,
            page,
        } = ev
        {
            if *seq == target_seq && !seen.contains(page) {
                seen.push(*page);
            }
        }
    }
    seen.len() as u64
}

/// Collect the set of distinct span seqs that are PINNED in at least one
/// Present event. BTreeMap for deterministic ordering (house style,
/// same as [`crate::attention::SpanAccount::from_events`]).
fn pinned_spans(events: &[SpanEvent]) -> BTreeMap<u64, ()> {
    let mut spans: BTreeMap<u64, ()> = BTreeMap::new();
    for ev in events {
        if let SpanEvent::Present {
            seq, pinned: true, ..
        } = ev
        {
            spans.entry(*seq).or_insert(());
        }
    }
    spans
}

/// PURE: map pinned spans to KV-cache leases. PINNED spans (pager law)
/// become leases sized by token count (distinct pinned epochs) ×
/// `bytes_per_token` (measured by the host, never guessed). Non-pinned
/// spans produce no leases. When the valve reports `model_loaded ==
/// false`, no leases are produced — the bridge defers to the valve.
///
/// The function reads only the slices it is given; it has no I/O and
/// no hidden state. Calling it twice with the same input yields the
/// same output (deterministic, BTreeMap-ordered).
pub fn co_schedule(events: &[SpanEvent], valve: &ValveStatus) -> Vec<KvLease> {
    if !valve.model_loaded {
        return Vec::new();
    }
    pinned_spans(events)
        .into_keys()
        .map(|seq| {
            let token_count = pinned_epochs(events, seq);
            KvLease {
                prefix_hash: seq,
                bytes_est: token_count * valve.bytes_per_token,
                expires: valve.keep_alive_secs,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn present(seq: u64, pinned: bool, page: u64) -> SpanEvent {
        SpanEvent::Present { seq, pinned, page }
    }

    #[test]
    fn pinned_epochs_counts_distinct_pages() {
        let events = vec![
            present(7, true, 0),
            present(7, true, 1),
            present(7, true, 0), // duplicate page 0
            present(7, true, 2),
        ];
        assert_eq!(pinned_epochs(&events, 7), 3, "pages 0,1,2");
    }

    #[test]
    fn pinned_epochs_ignores_non_pinned() {
        let events = vec![present(7, false, 0), present(7, true, 1)];
        assert_eq!(pinned_epochs(&events, 7), 1, "only page 1 is pinned");
    }

    #[test]
    fn pinned_spans_collects_distinct_seqs() {
        let events = vec![
            present(1, true, 0),
            present(2, false, 0),
            present(3, true, 0),
        ];
        let spans = pinned_spans(&events);
        let keys: Vec<u64> = spans.into_keys().collect();
        assert_eq!(keys, vec![1, 3], "only 1 and 3 are pinned");
    }
}
