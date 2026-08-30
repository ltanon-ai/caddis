//! attention_dividend.rs — CARD-0244 RED-first. Attention dividend ledger.
//!
//! Spans pay rent (stored_tokens × turns) or get evicted, BY MEASUREMENT.
//! The organ ORDERS; the pager STAYS the executor. `ledger()` is PURE:
//! it reads the tick stream (page epochs, cost) and the span-event stream
//! (residency + dividend attribution) and emits EvictionOrders for spans
//! that accrued zero dividend across N page epochs.
//!
//! THE RED: today no eviction anywhere is dividend-driven. The test pins
//! a zero-dividend resident span surviving 3 epochs untouched — `ledger()`
//! does not exist yet, so this cannot compile.

use caddis_organs::attention::{ledger, SpanAccount, SpanEvent};
use caddis_organs::eddy::{StatusClass, Tick};

fn tick(seq: u64, page: u64) -> Tick {
    Tick {
        run_id: "run-a".into(),
        seq,
        payload_hash: 5,
        status_class: StatusClass::Ok,
        outcome_hash: 0,
        artifact_hash: 0,
        cache_read: 0,
        cache_write: 0,
        latency_ms: 0,
        ts_ms: 10_000 + seq * 1_000,
        resume_after: None,
        page,
    }
}

/// RED: a resident span present across 3 page epochs with zero dividend
/// must receive an EvictionOrder. Pinned spans are never ordered.
#[test]
fn zero_dividend_span_across_three_epochs_is_ordered() {
    // Three ticks = three page epochs (0, 1, 2). Span 7 is present in
    // every epoch but never cited — zero dividend.
    let ticks = vec![tick(1, 0), tick(2, 1), tick(3, 2)];
    let events = vec![
        SpanEvent::Present {
            seq: 7,
            pinned: false,
            page: 0,
        },
        SpanEvent::Present {
            seq: 7,
            pinned: false,
            page: 1,
        },
        SpanEvent::Present {
            seq: 7,
            pinned: false,
            page: 2,
        },
    ];

    let orders = ledger(&ticks, &events);

    assert_eq!(orders.len(), 1, "one zero-dividend span -> one order");
    assert_eq!(orders[0].seq, 7, "order names the dead span");
    assert!(
        orders[0].evidence.contains("zero_dividend"),
        "evidence cites the cause: {:?}",
        orders[0].evidence
    );
}

/// RED: a pinned span present across 3 epochs with zero dividend is NEVER
/// ordered — the pager's own law outranks the organ.
#[test]
fn pinned_span_is_never_ordered() {
    let ticks = vec![tick(1, 0), tick(2, 1), tick(3, 2)];
    let events = vec![
        SpanEvent::Present {
            seq: 9,
            pinned: true,
            page: 0,
        },
        SpanEvent::Present {
            seq: 9,
            pinned: true,
            page: 1,
        },
        SpanEvent::Present {
            seq: 9,
            pinned: true,
            page: 2,
        },
    ];

    let orders = ledger(&ticks, &events);
    assert!(orders.is_empty(), "pinned spans survive: {orders:?}");
}

/// RED: a span that earns dividend (cited) across the same 3 epochs is
/// NOT ordered — it pays its rent.
#[test]
fn cited_span_is_not_ordered() {
    let ticks = vec![tick(1, 0), tick(2, 1), tick(3, 2)];
    let events = vec![
        SpanEvent::Present {
            seq: 5,
            pinned: false,
            page: 0,
        },
        SpanEvent::Dividend { seq: 5, page: 0 },
        SpanEvent::Present {
            seq: 5,
            pinned: false,
            page: 1,
        },
        SpanEvent::Present {
            seq: 5,
            pinned: false,
            page: 2,
        },
    ];

    let orders = ledger(&ticks, &events);
    assert!(orders.is_empty(), "cited span earns its keep: {orders:?}");
}

/// RED: a span present in only 2 epochs (below STAGNANT_WINDOW) is not
/// ordered yet — it has not exhausted the grace window.
#[test]
fn span_below_window_is_not_ordered() {
    let ticks = vec![tick(1, 0), tick(2, 1), tick(3, 2)];
    let events = vec![
        SpanEvent::Present {
            seq: 3,
            pinned: false,
            page: 0,
        },
        SpanEvent::Present {
            seq: 3,
            pinned: false,
            page: 1,
        },
    ];

    let orders = ledger(&ticks, &events);
    assert!(orders.is_empty(), "below window = grace: {orders:?}");
}

/// RED: ledger is PURE — it does not read files, only the slices it is
/// given. The SpanAccount it builds must report the dividend tally.
#[test]
fn account_reports_dividend_and_resident_pages() {
    let events = vec![
        SpanEvent::Present {
            seq: 1,
            pinned: false,
            page: 0,
        },
        SpanEvent::Dividend { seq: 1, page: 0 },
        SpanEvent::Dividend { seq: 1, page: 1 },
        SpanEvent::Present {
            seq: 1,
            pinned: false,
            page: 2,
        },
    ];
    let accounts = SpanAccount::from_events(&events);
    let a = &accounts[&1];
    assert_eq!(a.seq, 1);
    assert!(!a.pinned);
    assert_eq!(a.dividend, 2, "two dividend events");
    assert!(
        a.resident_pages >= 3,
        "present in 3 epochs: {}",
        a.resident_pages
    );
}
