//! Direct tests for ledger-derived card state (CARD-0110).

use super::*;

fn row(typ: &str, from: &str, b: &str) -> String {
    format!(
        "{{\"seq\":1,\"v\":1,\"id\":\"x\",\"idem_key\":\"k\",\"type\":\"{typ}\",\
         \"from\":\"{from}\",\"to\":\"warden\",\"body\":\"{b}\",\"ts\":\"1\"}}\n"
    )
}

fn opened(from: &str, id: &str) -> String {
    row(OPEN_TYPE, from, &body("open", id, "_card_x.md", "deadbeef"))
}

fn closed(from: &str, id: &str) -> String {
    row(
        CLOSE_TYPE,
        from,
        &body("close", id, "_card_x.md", "deadbeef"),
    )
}

#[test]
fn an_empty_ledger_holds_no_card_and_nothing_unreadable() {
    let s = active_for("", "peleda.aaaaaaaa");
    assert_eq!(s.active, None);
    assert_eq!(s.unreadable, 0);
}

#[test]
fn an_open_row_makes_the_card_active() {
    let s = active_for(&opened("peleda.aaaaaaaa", "CARD-0110"), "peleda.aaaaaaaa");
    let card = s.active.expect("a card is open");
    assert_eq!(card.id, "CARD-0110");
    assert_eq!(card.path, "_card_x.md");
    assert_eq!(card.hash, "deadbeef");
}

#[test]
fn a_close_row_clears_it() {
    let led = opened("peleda.aaaaaaaa", "CARD-0110") + &closed("peleda.aaaaaaaa", "CARD-0110");
    assert_eq!(active_for(&led, "peleda.aaaaaaaa").active, None);
}

#[test]
fn reopening_after_a_close_is_active_again() {
    let led = opened("peleda.aaaaaaaa", "CARD-1")
        + &closed("peleda.aaaaaaaa", "CARD-1")
        + &opened("peleda.aaaaaaaa", "CARD-2");
    let card = active_for(&led, "peleda.aaaaaaaa").active.expect("open");
    assert_eq!(card.id, "CARD-2");
}

#[test]
fn one_sessions_card_is_invisible_to_another() {
    // THE TEST A SIDE STATE FILE WOULD SILENTLY FAIL, and the whole reason this
    // state lives in the ledger keyed on a session-scoped caller.
    let led = opened("peleda.aaaaaaaa", "CARD-0110");
    assert!(active_for(&led, "peleda.aaaaaaaa").active.is_some());
    assert_eq!(active_for(&led, "peleda.bbbbbbbb").active, None);
}

#[test]
fn the_caller_match_is_exact_and_never_a_lane_prefix() {
    // `--from` matches a lane on a dot boundary because a report about a lane
    // wants all of its sessions. A CARD belongs to one session: a prefix match
    // here would hand session A's card to session B.
    let led = opened("peleda.aaaaaaaa", "CARD-0110");
    assert_eq!(active_for(&led, "peleda").active, None);
}

#[test]
fn a_torn_row_is_counted_and_does_not_hide_the_card() {
    // A real interleaved row from the live ledger, not an invented one.
    let torn = "{\"seq\":{\"seq\":538,\"v\":5381,\"v\":1,\"id\":\",\"id\":\"wardnf7acdbc1\"}\n";
    let led = opened("peleda.aaaaaaaa", "CARD-0110").to_string() + torn;
    let s = active_for(&led, "peleda.aaaaaaaa");
    assert_eq!(s.unreadable, 1, "damage is counted, never silently dropped");
    assert!(
        s.active.is_some(),
        "an unreadable neighbour must not lose a card that IS readable"
    );
}

#[test]
fn an_open_row_with_a_malformed_body_does_not_invent_a_card() {
    let led = row(OPEN_TYPE, "peleda.aaaaaaaa", "open|only-two-fields");
    assert_eq!(active_for(&led, "peleda.aaaaaaaa").active, None);
}

#[test]
fn ordinary_verdict_rows_are_ignored_entirely() {
    let led = row("tool.bash", "peleda.aaaaaaaa", "allow|echo ok||")
        + &opened("peleda.aaaaaaaa", "CARD-0110")
        + &row("tool.bash", "peleda.aaaaaaaa", "deny|rm -rf /||why");
    let card = active_for(&led, "peleda.aaaaaaaa").active.expect("open");
    assert_eq!(card.id, "CARD-0110");
}

#[test]
fn the_body_helper_round_trips_through_the_shared_row_parser() {
    // The card body must stay readable by the SAME right-to-left splitter the
    // verdict rows use, or the ledger grows a second row grammar.
    let b = body("open", "CARD-0110", "C:/w/_card_0110.md", "abc123");
    let (verb, id) = crate::rows::split_body(&b).expect("splits like a verdict row");
    assert_eq!(verb, "open");
    assert_eq!(id, "CARD-0110");
}
