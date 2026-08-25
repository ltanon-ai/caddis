//! Direct tests for the receipt fold (CARD-0112).

use super::*;

fn row(seq: u64, from: &str, ts: u64, typ: &str, body: &str) -> String {
    format!(
        "{{\"seq\":{seq},\"v\":1,\"id\":\"x\",\"idem_key\":\"k\",\"type\":\"{typ}\",\
         \"from\":\"{from}\",\"to\":\"warden\",\"body\":\"{body}\",\"ts\":{ts}}}\n"
    )
}

fn all() -> Filters {
    Filters {
        from: None,
        since_hours: None,
    }
}

fn of(text: &str) -> Receipt {
    build("L", text, &all(), 1_000_000)
}

#[test]
fn verdicts_are_counted_per_tag() {
    let led = row(1, "t", 10, "tool.bash", "allow|echo a||")
        + &row(2, "t", 20, "tool.bash", "steer|git reset||some.law")
        + &row(
            3,
            "t",
            30,
            "tool.bash",
            "deny|rm -rf /||caddis-warden [fs.rmrf]: no",
        );
    let r = of(&led);
    assert_eq!((r.rows, r.allow, r.steer, r.deny), (3, 1, 1, 1));
    assert_eq!(r.first_ts, Some(10));
    assert_eq!(r.last_ts, 30);
    assert_eq!(r.by_tool.get("tool.bash"), Some(&3));
}

#[test]
fn a_denial_is_grouped_by_law_and_cites_its_row() {
    let led = row(
        7,
        "t",
        1,
        "tool.bash",
        "deny|rm -rf /||caddis-warden [fs.rmrf]: no",
    ) + &row(
        9,
        "t",
        2,
        "tool.bash",
        "deny|rm -rf /x||caddis-warden [fs.rmrf]: no",
    );
    let r = of(&led);
    assert_eq!(r.deny_by_law.get("fs.rmrf"), Some(&vec![7, 9]));
    assert_eq!(r.law_fires.get("fs.rmrf"), Some(&2));
}

#[test]
fn an_unattributed_denial_is_grouped_rather_than_dropped() {
    let r = of(&row(
        1,
        "t",
        1,
        "tool.bash",
        "deny|cat secret||a sensitive path",
    ));
    assert_eq!(r.deny, 1);
    assert!(r.deny_by_law.contains_key("(unattributed)"));
}

#[test]
fn a_file_written_twice_is_one_distinct_file_with_a_count_of_two() {
    // "How many files did it touch" and "how many writes did it make" are
    // different questions; conflating them overstates the blast radius.
    let led = row(1, "t", 1, "tool.write", "allow|x|src/a.rs|")
        + &row(2, "t", 2, "tool.write", "allow|x|src/a.rs|")
        + &row(3, "t", 3, "tool.write", "allow|x|src/b.rs|");
    let r = of(&led);
    assert_eq!(r.files.len(), 2);
    assert_eq!(r.files.get("src/a.rs"), Some(&2));
    assert_eq!(r.files.get("src/b.rs"), Some(&1));
}

#[test]
fn an_empty_path_contributes_no_file_entry() {
    let r = of(&row(1, "t", 1, "tool.bash", "allow|echo a||"));
    assert!(r.files.is_empty());
}

#[test]
fn a_withheld_command_is_counted_as_withheld_not_as_absent() {
    // A masked command HAPPENED; the ledger simply did not keep its contents.
    // Counting it as nothing would understate what the agent did.
    let led = row(1, "t", 1, "tool.bash", "allow|export K=***redacted||")
        + &row(
            2,
            "t",
            2,
            "tool.bash",
            "allow|cat big [4096 bytes truncated]||",
        );
    let r = of(&led);
    assert_eq!(r.withheld, 2);
    assert_eq!(r.rows, 2, "they still count as rows");
    assert_eq!(r.allow, 2, "and still as verdicts");
}

#[test]
fn a_torn_row_is_counted_unreadable_and_not_folded() {
    let torn = "{\"seq\":{\"seq\":538,\"v\":5381,\"v\":1,\"id\":\",\"id\":\"x\"}\n";
    let r = of(&(row(1, "t", 1, "tool.bash", "allow|echo a||") + torn));
    assert_eq!(r.unreadable, 1);
    assert_eq!(r.rows, 1);
}

#[test]
fn card_rows_are_listed_and_never_counted_as_verdicts() {
    let led = row(1, "t", 1, "card.open", "open|CARD-1|_card_1.md|abc")
        + &row(2, "t", 2, "card.close", "close|CARD-1|_card_1.md|abc")
        + &row(3, "t", 3, "card.open", "open|CARD-2|_card_2.md|def");
    let r = of(&led);
    assert_eq!(r.cards_opened, vec!["CARD-1", "CARD-2"]);
    assert_eq!(r.cards_closed, vec!["CARD-1"]);
    assert_eq!((r.allow, r.steer, r.deny), (0, 0, 0));
    assert_eq!(r.rows, 3);
}

#[test]
fn the_from_filter_matches_a_lane_and_its_sessions_but_not_a_neighbour() {
    let led = row(1, "peleda", 1, "tool.bash", "allow|a||")
        + &row(2, "peleda.a1b2c3d4", 2, "tool.bash", "allow|b||")
        + &row(3, "peleda-two", 3, "tool.bash", "allow|c||");
    let f = Filters {
        from: Some("peleda".into()),
        since_hours: None,
    };
    assert_eq!(build("L", &led, &f, 1_000_000).rows, 2);
}

#[test]
fn a_since_window_excludes_older_rows_and_unknown_timestamps() {
    let now = 1_000_000u64;
    let led = row(1, "t", now - 100, "tool.bash", "allow|recent||")
        + &row(2, "t", now - 90_000, "tool.bash", "allow|old||")
        + &row(3, "t", 0, "tool.bash", "allow|unknown||");
    let f = Filters {
        from: None,
        since_hours: Some(1),
    };
    let r = build("L", &led, &f, now);
    assert_eq!(r.rows, 1, "only the recent row is inside a 1h window");
}

#[test]
fn an_empty_window_is_a_receipt_not_an_error() {
    let r = build("L", "", &all(), 1_000_000);
    assert_eq!(r.rows, 0);
    assert_eq!(r.unreadable, 0);
}

#[test]
fn the_command_head_is_one_line_and_capped() {
    assert_eq!(command_head("allow|echo one\necho two||"), "echo one");
    assert_eq!(command_head("not a body"), "");
}
