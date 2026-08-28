//! ttl_tests.rs — P1 slice 2 gates: the F10 TTL state machine and the
//! sweep (Done-When: expired TTL removes a seat from panel selection).

use super::*;
use crate::registry::{Card, Registry, SeatCard};
use crate::{CostClass, SeatState};

const NOW: u64 = 1_800_000_000; // a fixed epoch — the machine is pure.

fn seat(id: &str, provider: &str, state: SeatState, cost: CostClass, since: u64) -> SeatCard {
    SeatCard {
        id: id.into(),
        provider: provider.into(),
        family: provider.into(),
        model: id.rsplit('/').next().unwrap().into(),
        lane_type: crate::LaneType::Http,
        cost_class: cost,
        state,
        since_epoch_s: since,
        caps: 1,
        cost_in_usd_per_mtok: if cost == CostClass::Free { 0.0 } else { 1.0 },
        cost_out_usd_per_mtok: if cost == CostClass::Free { 0.0 } else { 2.0 },
        context_window: 128_000,
        max_tokens: 16_384,
        source: "models.json#deadbeef".into(),
    }
}

fn card(s: SeatCard) -> Card {
    Card::Seat(s)
}

// --- cadence priors are DATA (pinned: re-rule via the edit path, not edits here).

#[test]
fn cadence_priors_are_pinned() {
    let c = Cadence::default();
    assert_eq!(c.live_probe_every_s, 3600);
    assert_eq!(c.expired_ttl_s, 86_400);
    assert_eq!(c.rate_limited_cooldown_s, 900);
    assert_eq!(c.probing_timeout_s, 300);
    assert_eq!(c.failed_retry_every_s, 3600);
}

// --- step: every state's law, boundary included (>=, not >).

#[test]
fn live_stale_reprobes_fresh_keeps() {
    let c = Cadence::default();
    let fresh = seat("p/a", "p", SeatState::Live, CostClass::Free, NOW - 3599);
    let stale = seat("p/a", "p", SeatState::Live, CostClass::Free, NOW - 3600);
    assert_eq!(step(&fresh, NOW, &c, true), Step::Keep);
    // Council 2026-08-28: the 1h clock VERIFIES — never benches.
    assert_eq!(step(&stale, NOW, &c, true), Step::ReprobeDue);
}

/// Council ruling 2026-08-28: a stale Live seat is queued for re-probe
/// and the sweep writes NOTHING on the clock alone — Expired is
/// reachable only via a 402 probe result.
#[test]
fn live_clock_never_benches() {
    let c = Cadence::default();
    let stale = seat(
        "p/a",
        "p",
        SeatState::Live,
        CostClass::Free,
        NOW - 10 * 3600,
    );
    assert_eq!(step(&stale, NOW, &c, true), Step::ReprobeDue);
    assert_eq!(step(&stale, NOW, &c, false), Step::ReprobeDue);
    let reg = Registry::fold(&[card(stale)]);
    assert!(
        sweep(&reg, NOW, &c, quota_renewable).is_empty(),
        "no card on the clock alone"
    );
}

#[test]
fn expired_ttl_transitions_by_renewability() {
    let c = Cadence::default();
    let within = seat(
        "p/a",
        "p",
        SeatState::Expired,
        CostClass::Free,
        NOW - 86_399,
    );
    let lapsed = seat(
        "p/a",
        "p",
        SeatState::Expired,
        CostClass::Free,
        NOW - 86_400,
    );
    let lapsed_paid = seat("p/b", "p", SeatState::Expired, CostClass::Mid, NOW - 86_400);
    // Quota cooldown still running: nothing happens (never hammer a dead lane).
    assert_eq!(step(&within, NOW, &c, true), Step::Keep);
    // Renewable (free quota calendar): Failed — retries forever.
    assert_eq!(step(&lapsed, NOW, &c, true), Step::Fail);
    // Non-renewable (bought capacity): Retired — reviving is a ruling.
    assert_eq!(step(&lapsed_paid, NOW, &c, false), Step::Retire);
}

#[test]
fn rate_limited_cools_down_then_probe_is_due() {
    let c = Cadence::default();
    let cooling = seat(
        "p/a",
        "p",
        SeatState::RateLimited,
        CostClass::Free,
        NOW - 899,
    );
    let cooled = seat(
        "p/a",
        "p",
        SeatState::RateLimited,
        CostClass::Free,
        NOW - 900,
    );
    assert_eq!(step(&cooling, NOW, &c, true), Step::Keep);
    assert_eq!(step(&cooled, NOW, &c, true), Step::ReprobeDue);
}

#[test]
fn wedged_probe_fails_fresh_probe_keeps() {
    let c = Cadence::default();
    let in_flight = seat("p/a", "p", SeatState::Probing, CostClass::Free, NOW - 299);
    let wedged = seat("p/a", "p", SeatState::Probing, CostClass::Free, NOW - 300);
    assert_eq!(step(&in_flight, NOW, &c, true), Step::Keep);
    assert_eq!(step(&wedged, NOW, &c, true), Step::Fail);
}

#[test]
fn seed_probing_is_due_now_not_a_timeout_failure() {
    // The collector seed: state=probing, since=0 (no clock data). The
    // first probe is DUE — failing 75 never-probed seats on the first
    // sweep would be a defect, not honesty.
    let c = Cadence::default();
    let seed = seat("p/a", "p", SeatState::Probing, CostClass::Free, 0);
    assert_eq!(step(&seed, NOW, &c, true), Step::ReprobeDue);
    assert_eq!(step(&seed, 1, &c, false), Step::ReprobeDue);
}

#[test]
fn failed_retries_forever_never_retires() {
    let c = Cadence::default();
    let due = seat("p/a", "p", SeatState::Failed, CostClass::Free, NOW - 3600);
    let waiting = seat("p/a", "p", SeatState::Failed, CostClass::Free, NOW - 3599);
    // Even non-renewable: there is NO Failed->Retired arm (Rulings 5+8).
    assert_eq!(step(&due, NOW, &c, false), Step::ReprobeDue);
    assert_eq!(step(&waiting, NOW, &c, true), Step::Keep);
}

#[test]
fn retired_is_terminal() {
    let c = Cadence::default();
    let r = seat(
        "p/a",
        "p",
        SeatState::Retired,
        CostClass::Mid,
        NOW - 10_000_000,
    );
    assert_eq!(step(&r, NOW, &c, true), Step::Keep);
    assert_eq!(step(&r, NOW, &c, false), Step::Keep);
}

#[test]
fn quota_renewable_is_measured_free() {
    assert!(quota_renewable(&seat(
        "p/a",
        "p",
        SeatState::Live,
        CostClass::Free,
        0
    )));
    assert!(!quota_renewable(&seat(
        "p/b",
        "p",
        SeatState::Live,
        CostClass::Mid,
        0
    )));
    assert!(!quota_renewable(&seat(
        "p/c",
        "p",
        SeatState::Live,
        CostClass::Premium,
        0
    )));
}

// --- sweep: one step per seat, cards for transitions only.

#[test]
fn sweep_writes_transition_cards_only() {
    let reg = Registry::fold(&[
        card(seat("p/keep", "p", SeatState::Live, CostClass::Free, NOW)),
        card(seat(
            "p/stale",
            "p",
            SeatState::Live,
            CostClass::Free,
            NOW - 7200,
        )),
        card(seat(
            "p/lapsed",
            "p",
            SeatState::Expired,
            CostClass::Free,
            NOW - 90_000,
        )),
    ]);
    let out = sweep(&reg, NOW, &Cadence::default(), quota_renewable);
    // Exactly one card: the lapsed Expired seat. Keep and ReprobeDue
    // (the stale Live seat — the clock verifies, never benches) write none.
    assert_eq!(out.len(), 1);
    let Card::Seat(s) = &out[0] else {
        panic!("transition cards are seat cards")
    };
    assert_eq!(s.id, "p/lapsed");
    assert_eq!(s.state, SeatState::Failed);
    assert_eq!(
        s.since_epoch_s, NOW,
        "the Failed retry window starts at sweep time"
    );
    // Everything else is carried unchanged (append-only edit shape).
    assert_eq!(s.provider, "p");
    assert_eq!(s.caps, 1);
}

#[test]
fn sweep_is_one_step_never_a_cascade() {
    // Expired + since=0: elapsed is enormous -> ONE step (Fail on a
    // renewable lane), NOT Fail->further in the same sweep. The stale
    // Live seat (since=0) writes nothing — the clock never benches.
    let reg = Registry::fold(&[
        card(seat("p/a", "p", SeatState::Expired, CostClass::Free, 0)),
        card(seat("p/b", "p", SeatState::Live, CostClass::Free, 0)),
    ]);
    let out = sweep(&reg, NOW, &Cadence::default(), quota_renewable);
    assert_eq!(out.len(), 1);
    let Card::Seat(s) = &out[0] else { panic!() };
    assert_eq!(s.id, "p/a");
    assert_eq!(s.state, SeatState::Failed);
    assert_eq!(s.since_epoch_s, NOW);
    // The NEXT sweep re-derives from the new rows: nothing transitions.
    let cards = vec![
        card(seat("p/a", "p", SeatState::Expired, CostClass::Free, 0)),
        card(seat("p/b", "p", SeatState::Live, CostClass::Free, 0)),
        out[0].clone(),
    ];
    let reg2 = Registry::fold(&cards);
    assert!(sweep(&reg2, NOW, &Cadence::default(), quota_renewable).is_empty());
}

/// THE P1 Done-When: "expired TTL removes a seat from selection." The
/// full loop — stream cards -> sweep -> append -> refold -> substrate
/// projection -> panel construction — must exclude the lapsed seat.
#[test]
fn expired_ttl_removes_seat_from_panel_selection() {
    let mut cards = vec![
        card(seat(
            "alpha/a",
            "alpha",
            SeatState::Live,
            CostClass::Free,
            NOW,
        )),
        card(seat(
            "bravo/b",
            "bravo",
            SeatState::Live,
            CostClass::Free,
            NOW,
        )),
        card(seat(
            "zulu/z",
            "zulu",
            SeatState::Live,
            CostClass::Free,
            NOW,
        )),
        card(seat(
            "lapsed/l",
            "lapsed",
            SeatState::Expired,
            CostClass::Free,
            NOW - 90_000,
        )),
    ];
    let reg = Registry::fold(&cards);
    // All four project; only the three Live are selectable.
    assert_eq!(reg.seats().len(), 4);

    let transitions = sweep(&reg, NOW, &Cadence::default(), quota_renewable);
    assert_eq!(transitions.len(), 1, "only the lapsed seat transitions");
    cards.extend(transitions);
    let reg2 = Registry::fold(&cards);

    let lapsed = reg2.seats.get("lapsed/l").unwrap();
    assert_eq!(
        lapsed.state,
        SeatState::Failed,
        "renewable free lane: Failed, never auto-Retired"
    );
    assert!(
        !lapsed.state.selectable(),
        "F10: a non-Live seat is never selectable"
    );
    // Panel construction over the swept registry: 4 candidates, panel of 3
    // — the lapsed seat must NOT be seated even though it would win the
    // free-first order (id "lapsed/l" sorts before "zulu/z").
    let candidates = reg2.seats();
    let panel = crate::construct_panel(
        &candidates,
        &crate::Floors {
            min_families: 2,
            min_non_chinese: 1,
            panel_size: 3,
        },
    )
    .unwrap();
    let seated: Vec<&str> = panel
        .seats
        .iter()
        .map(|ps| ps.seat.lane_id.as_str())
        .collect();
    assert_eq!(seated.len(), 3);
    assert!(
        !seated.contains(&"lapsed/l"),
        "the expired-TTL seat is removed from selection"
    );
    assert!(
        seated.contains(&"alpha/a") && seated.contains(&"bravo/b") && seated.contains(&"zulu/z")
    );
}

#[test]
fn sweep_non_renewable_expires_to_retired() {
    let reg = Registry::fold(&[card(seat(
        "paid/p",
        "paid",
        SeatState::Expired,
        CostClass::Mid,
        NOW - 90_000,
    ))]);
    let out = sweep(&reg, NOW, &Cadence::default(), quota_renewable);
    let Card::Seat(s) = &out[0] else { panic!() };
    assert_eq!(s.state, SeatState::Retired);
    // A ruling can override the renewable predicate without touching the
    // machine: same seat, treated renewable -> Failed instead.
    let always: fn(&SeatCard) -> bool = |_| true;
    let out2 = sweep(&reg, NOW, &Cadence::default(), always);
    let Card::Seat(s2) = &out2[0] else { panic!() };
    assert_eq!(s2.state, SeatState::Failed);
}
