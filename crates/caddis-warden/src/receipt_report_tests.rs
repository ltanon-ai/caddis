//! Direct tests for the receipt renderer (CARD-0112).
//!
//! The rendering is the part that can mislead a handoff auditor, so it is
//! asserted line by line rather than smoke-tested.

use super::*;
use crate::receipt::{build, Filters};

fn row(seq: u64, ts: u64, typ: &str, body: &str) -> String {
    format!(
        "{{\"seq\":{seq},\"v\":1,\"id\":\"x\",\"idem_key\":\"k\",\"type\":\"{typ}\",\
         \"from\":\"t\",\"to\":\"warden\",\"body\":\"{body}\",\"ts\":{ts}}}\n"
    )
}

fn of(text: &str) -> Receipt {
    build(
        "L",
        text,
        &Filters {
            from: None,
            since_hours: None,
        },
        1_000_000,
    )
}

#[test]
fn an_empty_window_says_so_and_still_states_its_coverage() {
    let text = render_text(&of(""));
    assert!(text.contains("NOTHING IN THIS WINDOW"), "{text}");
    // Even here, because an empty receipt over a DAMAGED ledger is a different
    // fact from an empty receipt over a clean one.
    assert!(text.contains("coverage:"), "{text}");
}

#[test]
fn coverage_is_printed_even_when_both_counts_are_zero() {
    let text = render_text(&of(&row(1, 5, "tool.bash", "allow|echo a||")));
    assert!(
        text.contains("0 withheld command(s) IN THIS WINDOW"),
        "{text}"
    );
    assert!(text.contains("0 unreadable line(s) FILE-WIDE"), "{text}");
}

#[test]
fn the_digest_states_counts_files_denials_and_laws() {
    let led = row(1, 10, "tool.bash", "allow|echo a||")
        + &row(2, 20, "tool.write", "allow|x|src/a.rs|")
        + &row(
            3,
            30,
            "tool.bash",
            "deny|rm -rf /||caddis-warden [fs.rmrf]: no",
        );
    let text = render_text(&of(&led));
    assert!(
        text.contains("rows: 3  allow: 2  steer: 0  deny: 1"),
        "{text}"
    );
    assert!(text.contains("window: ts 10 .. 30"), "{text}");
    assert!(text.contains("tools:"), "{text}");
    assert!(text.contains("files written: 1 distinct"), "{text}");
    assert!(text.contains("src/a.rs (x1)"), "{text}");
    assert!(text.contains("fs.rmrf x1 (seq 3)"), "{text}");
    assert!(text.contains("laws fired: fs.rmrf=1"), "{text}");
}

#[test]
fn a_long_file_list_says_how_many_it_did_not_show() {
    let led: String = (0..25)
        .map(|n| row(n, n, "tool.write", &format!("allow|x|src/f{n}.rs|")))
        .collect();
    let text = render_text(&of(&led));
    assert!(text.contains("files written: 25 distinct"), "{text}");
    assert!(text.contains("... and 5 more not shown"), "{text}");
}

#[test]
fn a_card_left_open_is_called_out_by_name() {
    // The single most useful line for a handoff: work that was declared and
    // never closed is work whose bounds nobody ever confirmed.
    let led = row(1, 1, "card.open", "open|CARD-1|_card_1.md|abc")
        + &row(2, 2, "card.open", "open|CARD-2|_card_2.md|def")
        + &row(3, 3, "card.close", "close|CARD-1|_card_1.md|abc");
    let text = render_text(&of(&led));
    assert!(text.contains("cards: 2 opened, 1 closed"), "{text}");
    assert!(text.contains("STILL OPEN: CARD-2"), "{text}");
    assert!(!text.contains("STILL OPEN: CARD-1"), "{text}");
}

#[test]
fn nothing_left_open_prints_no_still_open_line() {
    let led = row(1, 1, "card.open", "open|CARD-1|_card_1.md|abc")
        + &row(2, 2, "card.close", "close|CARD-1|_card_1.md|abc");
    assert!(!render_text(&of(&led)).contains("STILL OPEN"));
}

#[test]
fn the_json_is_one_object_and_escapes_what_it_embeds() {
    let led = row(1, 5, "tool.write", "allow|x|C:\\\\w\\\\a.rs|");
    let json = render_json(&of(&led));
    assert!(json.starts_with('{') && json.ends_with('}'), "{json}");
    assert!(json.contains("\"rows\":1"), "{json}");
    assert!(json.contains("\"unreadable\":0"), "{json}");
    assert!(json.contains("\"withheld\":0"), "{json}");
    // A Windows path must survive as JSON rather than breaking the object.
    assert!(json.contains("C:\\\\w\\\\a.rs"), "{json}");
}

#[test]
fn the_json_states_the_window_it_covers() {
    // render_text has always printed `scope: from=… since=…`; render_json
    // omitted both, so a JSON consumer could not tell a whole-ledger receipt
    // from a one-caller, one-hour slice. Two readers of one struct disagreeing
    // about which window they describe is the same defect class this release
    // is named for.
    let unfiltered = render_json(&of(""));
    assert!(
        unfiltered.contains("\"from\":null"),
        "an unset caller must render as null, never as a filter that matched nothing: {unfiltered}"
    );
    assert!(unfiltered.contains("\"since_hours\":null"), "{unfiltered}");

    let scoped = build(
        "L",
        "",
        &Filters {
            from: Some("peleda".into()),
            since_hours: Some(24),
        },
        1_000_000,
    );
    let json = render_json(&scoped);
    assert!(json.contains("\"from\":\"peleda\""), "{json}");
    assert!(json.contains("\"since_hours\":24"), "{json}");
    assert!(json.starts_with('{') && json.ends_with('}'), "{json}");
}

#[test]
fn the_json_carries_empty_containers_rather_than_omitting_them() {
    // A consumer must be able to read `files: {}` as "none" instead of having
    // to distinguish an absent key from an empty one.
    let json = render_json(&of(""));
    for key in [
        "\"tools\":{}",
        "\"files\":{}",
        "\"deny_by_law\":{}",
        "\"law_fires\":{}",
        "\"cards_opened\":[]",
        "\"cards_closed\":[]",
    ] {
        assert!(json.contains(key), "{key} missing from {json}");
    }
}

#[test]
fn the_scope_line_names_what_was_asked_for() {
    let r = build(
        "L",
        "",
        &Filters {
            from: Some("peleda".into()),
            since_hours: Some(6),
        },
        1_000_000,
    );
    let text = render_text(&r);
    assert!(text.contains("scope: from=peleda since=6h"), "{text}");
}

#[test]
fn an_unscoped_receipt_says_so_rather_than_leaving_it_blank() {
    let text = render_text(&of(""));
    assert!(text.contains("from=(everyone)"), "{text}");
    assert!(text.contains("since=(all history)"), "{text}");
}
