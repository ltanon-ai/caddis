//! kv_bridge.rs — CARD-0246 RED-first. The RAM↔attention bridge.
//!
//! RAM sits BETWEEN the memory layers and the attention layer, and
//! nothing schedules it. The model valve already owns VRAM. The pager
//! already knows which spans are PINNED. The bridge: co-schedule them —
//! the active session's pinned prefix stays resident in the local
//! engine's KV cache, so attention layers never recompute it.
//!
//! THE RED: today `co_schedule` does not compile into existence, and
//! stage-0's probe script (`tools/kv_probe.py`) returns "unmeasured".
//! The test pins a pinned-span world with an EMPTY lease set — the
//! valve pins models with ZERO knowledge of what the session pinned.
//!
//! Laws pinned here from CARD-0246 §EXECUTION:
//!
//! 1. `co_schedule` is PURE: pinned spans (pager law) map to leases
//!    sized by token count × engine-specific bytes/token (measured in
//!    stage 0, not guessed).
//! 2. PINNED spans become leases; non-pinned spans do NOT — the pager's
//!    own pin flag is the sole authority.
//! 3. The bridge NEVER talks to the engine directly (one writer per
//!    resource) — leases are advice for the host to POST to the valve.

use caddis_organs::attention::SpanEvent;
use caddis_organs::kv_bridge::{co_schedule, KvLease, ValveStatus};

fn present(seq: u64, pinned: bool, page: u64) -> SpanEvent {
    SpanEvent::Present { seq, pinned, page }
}

fn dividend(seq: u64, page: u64) -> SpanEvent {
    SpanEvent::Dividend { seq, page }
}

/// RED: a pinned span present across 3 epochs maps to exactly ONE
/// KvLease. The lease's prefix_hash is stable (same span → same hash),
/// and bytes_est is token_count × bytes_per_token (measured, never 0).
#[test]
fn pinned_span_maps_to_one_lease() {
    let events = vec![
        present(7, true, 0),
        present(7, true, 1),
        present(7, true, 2),
    ];
    let valve = ValveStatus {
        model_loaded: true,
        bytes_per_token: 512,
        keep_alive_secs: 300,
    };
    let leases = co_schedule(&events, &valve);
    assert_eq!(leases.len(), 1, "one pinned span -> one lease: {leases:?}");
    let l: &KvLease = &leases[0];
    assert_eq!(l.prefix_hash, 7, "lease keyed by span seq");
    assert!(l.bytes_est > 0, "bytes_est is measured, never 0: {l:?}");
    assert_eq!(l.expires, 300, "lease ttl from valve keep_alive");
}

/// RED: non-pinned spans produce NO leases — the pager's pin flag is
/// the sole authority. A resident-but-not-pinned span does not get a
/// KV lease; it may be evicted by the attention ledger (CARD-0244).
#[test]
fn non_pinned_span_produces_no_lease() {
    let events = vec![
        present(3, false, 0),
        present(3, false, 1),
        present(3, false, 2),
    ];
    let valve = ValveStatus {
        model_loaded: true,
        bytes_per_token: 512,
        keep_alive_secs: 300,
    };
    let leases = co_schedule(&events, &valve);
    assert!(leases.is_empty(), "non-pinned -> no lease: {leases:?}");
}

/// RED: multiple pinned spans map to multiple leases — one per distinct
/// pinned span seq. Each lease carries its own bytes_est and the same
/// ttl from the valve.
#[test]
fn multiple_pinned_spans_map_to_multiple_leases() {
    let events = vec![
        present(1, true, 0),
        present(2, true, 0),
        present(1, true, 1),
        present(2, true, 1),
    ];
    let valve = ValveStatus {
        model_loaded: true,
        bytes_per_token: 256,
        keep_alive_secs: 600,
    };
    let leases = co_schedule(&events, &valve);
    assert_eq!(
        leases.len(),
        2,
        "two pinned spans -> two leases: {leases:?}"
    );
    // Leases are ordered by seq (BTreeMap determinism, house style).
    assert_eq!(leases[0].prefix_hash, 1);
    assert_eq!(leases[1].prefix_hash, 2);
    assert_eq!(leases[0].expires, 600);
    assert_eq!(leases[1].expires, 600);
}

/// RED: when the valve reports model_loaded=false, NO leases are
/// produced — there is nothing to pin into. The bridge defers to the
/// valve's state, never overriding it.
#[test]
fn valve_unloaded_produces_no_leases() {
    let events = vec![present(7, true, 0), present(7, true, 1)];
    let valve = ValveStatus {
        model_loaded: false,
        bytes_per_token: 512,
        keep_alive_secs: 300,
    };
    let leases = co_schedule(&events, &valve);
    assert!(leases.is_empty(), "valve off -> no leases: {leases:?}");
}

/// RED: dividend events are IGNORED by the bridge — the bridge keys on
/// PINNED residency only. A cited-but-not-pinned span gets no lease;
/// a pinned span that also earns dividend still gets its lease.
#[test]
fn dividend_events_do_not_create_leases() {
    let events = vec![
        dividend(5, 0),
        dividend(5, 1),
        present(9, true, 0),
        present(9, true, 1),
    ];
    let valve = ValveStatus {
        model_loaded: true,
        bytes_per_token: 512,
        keep_alive_secs: 300,
    };
    let leases = co_schedule(&events, &valve);
    assert_eq!(
        leases.len(),
        1,
        "only pinned span 9 -> one lease: {leases:?}"
    );
    assert_eq!(leases[0].prefix_hash, 9);
}

/// RED: bytes_est is token_count × bytes_per_token. The token count is
/// the number of DISTINCT page epochs the span was PINNED in — each
/// epoch contributes one prefix segment. This is the measured basis from
/// stage 0, never a guess.
#[test]
fn bytes_est_is_token_count_times_bytes_per_token() {
    let events = vec![
        present(7, true, 0),
        present(7, true, 1),
        present(7, true, 2),
    ];
    let valve = ValveStatus {
        model_loaded: true,
        bytes_per_token: 1024,
        keep_alive_secs: 300,
    };
    let leases = co_schedule(&events, &valve);
    assert_eq!(leases.len(), 1);
    // 3 distinct epochs × 1024 bytes/token = 3072 bytes_est.
    assert_eq!(
        leases[0].bytes_est,
        3 * 1024,
        "3 epochs × 1024 = 3072: {:?}",
        leases[0]
    );
}

/// RED: a span pinned in only ONE epoch gets a lease with bytes_est =
/// 1 × bytes_per_token. One prefix segment, one lease, one measurement.
#[test]
fn single_epoch_pinned_span_gets_minimal_lease() {
    let events = vec![present(7, true, 0)];
    let valve = ValveStatus {
        model_loaded: true,
        bytes_per_token: 512,
        keep_alive_secs: 300,
    };
    let leases = co_schedule(&events, &valve);
    assert_eq!(leases.len(), 1);
    assert_eq!(leases[0].bytes_est, 512, "1 epoch × 512 = 512");
}

/// RED: co_schedule is PURE — it reads only the slices it is given and
/// the valve status. Calling it twice with the same input yields the
/// same output (deterministic, no hidden state).
#[test]
fn co_schedule_is_pure_and_deterministic() {
    let events = vec![present(7, true, 0), present(7, true, 1)];
    let valve = ValveStatus {
        model_loaded: true,
        bytes_per_token: 512,
        keep_alive_secs: 300,
    };
    let a = co_schedule(&events, &valve);
    let b = co_schedule(&events, &valve);
    assert_eq!(a, b, "pure: same input -> same output");
}

/// RED: empty events produce empty leases. Total on empty input.
#[test]
fn empty_events_produce_no_leases() {
    let valve = ValveStatus {
        model_loaded: true,
        bytes_per_token: 512,
        keep_alive_secs: 300,
    };
    let leases = co_schedule(&[], &valve);
    assert!(leases.is_empty(), "empty input -> empty leases: {leases:?}");
}
