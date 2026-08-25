//! ledger_row_cap.rs — the row cap the atomicity guarantee rests on.
//!
//! RED-first for the pre-push review's CRITICAL. `Ledger::append` promised
//! atomicity on the grounds that "body.rs caps the body at 500 bytes" — a cap
//! in a DIFFERENT crate, over only the COMMAND rather than the whole body, and
//! `envelope::validate` has no body limit at all. So the kernel stated an
//! unconditional guarantee resting on a downstream crate it cannot see, while
//! `card.rs` and the organs canary appended without ever passing through it.
//!
//! These tests are the enforcement: a row is bounded before it is written, and
//! an elided body says it was elided rather than passing as the whole one.

use caddis_core::envelope;
use caddis_core::ledger::Ledger;

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("caddis-rowcap-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.join("l.jsonl")
}

fn env_with_body(body: &str) -> envelope::Envelope {
    envelope::validate(
        1,
        "rowcap-0001",
        "idem-rowcap-1",
        "signal/rowcap",
        "test",
        "ledger",
        body,
        "2026-08-25T00:00:00Z",
    )
    .expect("envelope::validate imposes no body limit — that is the point")
}

#[test]
fn an_enormous_body_still_writes_one_bounded_row() {
    let path = tmp("huge");
    let mut led = Ledger::open(&path).unwrap();
    // 200 KB, fifty times over any plausible single-syscall threshold.
    led.append(&env_with_body(&"A".repeat(200_000))).unwrap();

    let text = std::fs::read_to_string(&path).unwrap();
    let rows: Vec<&str> = text.lines().collect();
    assert_eq!(rows.len(), 1, "one append is one row");
    assert!(
        rows[0].len() <= 4096,
        "the row must fit the cap the guarantee rests on, got {}",
        rows[0].len()
    );
}

#[test]
fn a_truncated_body_says_it_was_truncated() {
    let path = tmp("says");
    let mut led = Ledger::open(&path).unwrap();
    led.append(&env_with_body(&"B".repeat(50_000))).unwrap();

    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        text.contains("bytes truncated"),
        "an elided row must never masquerade as the whole one: {}",
        &text[..text.len().min(300)]
    );
}

#[test]
fn an_ordinary_body_is_untouched() {
    let path = tmp("small");
    let mut led = Ledger::open(&path).unwrap();
    led.append(&env_with_body("deny|rm -rf /|/etc|D-014"))
        .unwrap();

    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("deny|rm -rf /|/etc|D-014"), "{text}");
    assert!(
        !text.contains("truncated"),
        "a body under the cap must not be elided: {text}"
    );
}

#[test]
fn the_row_stays_readable_after_elision() {
    // Truncating must not cut a row into something the reader cannot parse —
    // the counter recovery walks intact rows, and a mangled one is invisible.
    let path = tmp("readable");
    let mut led = Ledger::open(&path).unwrap();
    led.append(&env_with_body(&"C".repeat(100_000))).unwrap();

    let reopened = Ledger::open(&path).unwrap();
    assert_eq!(reopened.unreadable(), 0, "the elided row must still parse");
    assert_eq!(reopened.seq(), 1, "and still carry its seq");
}

#[test]
fn oversized_metadata_still_writes_a_bounded_row() {
    // Capping only the BODY bounded the body, not the ROW. `envelope::validate`
    // puts no upper bound on `from` either, so a caller with an enormous
    // endpoint wrote past the cap however small its body was.
    let path = tmp("meta");
    let mut led = Ledger::open(&path).unwrap();
    let env = envelope::validate(
        1,
        &"i".repeat(50_000),
        &"k".repeat(50_000),
        &format!("t{}", "y".repeat(50_000)),
        &"f".repeat(50_000),
        &"o".repeat(50_000),
        "small body",
        &"2026-08-25T00:00:00Z".repeat(1_000),
    )
    .expect("validate imposes no length bound on any of these");
    led.append(&env).unwrap();

    let text = std::fs::read_to_string(&path).unwrap();
    let rows: Vec<&str> = text.lines().collect();
    assert_eq!(rows.len(), 1);
    assert!(
        rows[0].len() <= 4096,
        "metadata must be capped too, got {}",
        rows[0].len()
    );
    let reopened = Ledger::open(&path).unwrap();
    assert_eq!(reopened.unreadable(), 0, "and the row must still parse");
}

#[test]
fn a_one_to_six_escape_expansion_is_budgeted_on_the_escaped_length() {
    // `esc` turns one C0 control byte into six (`\u0001`). Budgeting on the RAW
    // length would let ~2400 raw bytes through as "fitting" and then write a
    // ~14 KB row — the exact mistake elide_body's comment exists to retire, and
    // nothing else in this file would catch a regression to it.
    let path = tmp("expand");
    let mut led = Ledger::open(&path).unwrap();
    led.append(&env_with_body(&"\u{1}".repeat(4_000))).unwrap();

    let text = std::fs::read_to_string(&path).unwrap();
    let rows: Vec<&str> = text.lines().collect();
    assert_eq!(rows.len(), 1, "escaping must not split the row");
    assert!(
        rows[0].len() <= 4096,
        "budget is measured on the ESCAPED length, got {}",
        rows[0].len()
    );
    let reopened = Ledger::open(&path).unwrap();
    assert_eq!(reopened.unreadable(), 0);
}
