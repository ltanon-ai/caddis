//! Direct tests for bundle assembly (CARD-0114).

use super::*;

fn r(seq: u64, from: &str, typ: &str, body: &str) -> String {
    format!(
        "{{\"seq\":{seq},\"v\":1,\"id\":\"x\",\"idem_key\":\"k\",\"type\":\"{typ}\",\
         \"from\":\"{from}\",\"to\":\"warden\",\"body\":\"{body}\",\"ts\":10}}\n"
    )
}

/// A strict card on disk, so `declared()` and the RED-TEST needle have
/// something real to read.
struct CardFile(std::path::PathBuf);

impl CardFile {
    fn new(tag: &str, allowlist: &[&str]) -> Self {
        let p = std::env::temp_dir().join(format!(
            "caddis-attest-{tag}-{}-{:?}.md",
            std::process::id(),
            std::thread::current().id()
        ));
        let items: String = allowlist.iter().map(|x| format!("  - {x}\n")).collect();
        std::fs::write(
            &p,
            format!(
                "---\nid: CARD-A\nclass: fix\nowner: t\n---\n# a\n\n# Done-When\n- done\n\n\
                 # RED-TEST\ncargo test --workspace fails before this change\n\n\
                 # EXECUTION\nlevel: L1\nblast: 2\nclaims-forbidden: true\n\
                 allowlist:\n{items}anchors:\n  - path: a.rs\n      content: |\n        x\n"
            ),
        )
        .expect("card written");
        Self(p)
    }
    fn path(&self) -> String {
        self.0.to_string_lossy().replace('\\', "/")
    }
}

impl Drop for CardFile {
    fn drop(&mut self) {
        // swallow: best-effort-cleanup
        let _ = std::fs::remove_file(&self.0);
    }
}

fn ledger(card: &CardFile, inner: &str) -> String {
    r(
        1,
        "t.s1",
        "card.open",
        &format!("open|CARD-A|{}|abc123", card.path()),
    ) + inner
        + &r(
            99,
            "t.s1",
            "card.close",
            &format!("close|CARD-A|{}|abc123", card.path()),
        )
}

#[test]
fn a_card_worked_inside_its_allowlist_has_nothing_outside() {
    let c = CardFile::new("inside", &["src/a.rs", "src/b.rs"]);
    let led = ledger(
        &c,
        &(r(2, "t.s1", "tool.write", "allow|x|src/a.rs|")
            + &r(3, "t.s1", "tool.write", "allow|x|src/b.rs|")),
    );
    let b = build(&led, "CARD-A").expect("bundle");
    assert_eq!(b.card_id, "CARD-A");
    assert_eq!(b.from, "t.s1");
    assert_eq!(b.allow, 2);
    assert_eq!(b.files.len(), 2);
    assert!(b.outside.is_empty(), "{:?}", b.outside);
    assert_eq!(b.blast, 2);
}

#[test]
fn a_write_outside_the_allowlist_is_listed_not_summarised_away() {
    // ⛔ A bundle that omitted this would be the reassuring artifact the whole
    // program exists against.
    let c = CardFile::new("outside", &["src/a.rs"]);
    let led = ledger(
        &c,
        &(r(2, "t.s1", "tool.write", "allow|x|src/a.rs|")
            + &r(3, "t.s1", "tool.write", "allow|x|src/SNEAKY.rs|")),
    );
    let b = build(&led, "CARD-A").expect("bundle");
    assert_eq!(b.outside, vec!["src/SNEAKY.rs".to_string()]);
}

#[test]
fn the_same_stray_file_written_twice_is_listed_once() {
    let c = CardFile::new("dedup", &["src/a.rs"]);
    let led = ledger(
        &c,
        &(r(2, "t.s1", "tool.write", "allow|x|src/x.rs|")
            + &r(3, "t.s1", "tool.write", "allow|x|src/x.rs|")),
    );
    assert_eq!(build(&led, "CARD-A").expect("bundle").outside.len(), 1);
}

#[test]
fn verdicts_and_laws_inside_the_window_are_counted() {
    let c = CardFile::new("verdicts", &["src/a.rs"]);
    let led = ledger(
        &c,
        &(r(2, "t.s1", "tool.bash", "allow|echo a||")
            + &r(3, "t.s1", "tool.bash", "steer|git x||one.law")
            + &r(
                4,
                "t.s1",
                "tool.bash",
                "deny|rm -rf /||caddis-warden [fs.rmrf]: no",
            )),
    );
    let b = build(&led, "CARD-A").expect("bundle");
    assert_eq!((b.allow, b.steer, b.deny), (1, 1, 1));
    assert_eq!(b.laws.get("one.law"), Some(&1));
    assert_eq!(b.laws.get("fs.rmrf"), Some(&1));
}

#[test]
fn a_fingerprint_tail_is_not_counted_as_a_law() {
    let c = CardFile::new("fp", &["src/a.rs"]);
    let led = ledger(
        &c,
        &r(
            2,
            "t.s1",
            "tool.bash",
            "steer|git x||one.law|deadbeefdeadbeef",
        ),
    );
    let b = build(&led, "CARD-A").expect("bundle");
    assert_eq!(b.steer, 1);
    assert_eq!(b.laws.get("one.law"), Some(&1));
    assert!(
        !b.laws.contains_key("deadbeefdeadbeef"),
        "fp counted as law: {:?}",
        b.laws
    );
}

#[test]
fn rows_outside_the_window_are_not_attested() {
    let c = CardFile::new("window", &["src/a.rs"]);
    let before = r(0, "t.s1", "tool.bash", "allow|echo before||");
    let after = r(100, "t.s1", "tool.bash", "allow|echo after||");
    let led = before + &ledger(&c, &r(2, "t.s1", "tool.bash", "allow|echo inside||")) + &after;
    assert_eq!(build(&led, "CARD-A").expect("bundle").allow, 1);
}

#[test]
fn another_callers_rows_inside_the_window_are_not_attributed_to_this_card() {
    // Two agents work concurrently against one shared ledger; attributing one's
    // writes to the other's card would be worse than attesting nothing.
    let c = CardFile::new("caller", &["src/a.rs"]);
    let led = ledger(
        &c,
        &(r(2, "t.s1", "tool.write", "allow|x|src/a.rs|")
            + &r(3, "other.s9", "tool.write", "allow|x|src/THEIRS.rs|")),
    );
    let b = build(&led, "CARD-A").expect("bundle");
    assert_eq!(b.files.len(), 1);
    assert!(b.outside.is_empty(), "{:?}", b.outside);
}

#[test]
fn a_red_test_attempt_is_recorded_as_attempted_and_nothing_stronger() {
    let c = CardFile::new("red", &["src/a.rs"]);
    let led = ledger(
        &c,
        &r(2, "t.s1", "tool.bash", "allow|cargo test --workspace||"),
    );
    assert!(build(&led, "CARD-A").expect("bundle").red_test_seen);

    let none = ledger(&c, &r(2, "t.s1", "tool.bash", "allow|echo unrelated||"));
    assert!(!build(&none, "CARD-A").expect("bundle").red_test_seen);
}

#[test]
fn a_card_whose_file_is_gone_reports_unknown_and_never_clean() {
    // ⛔ THE REASSURING ARTIFACT THIS WHOLE PROGRAM EXISTS AGAINST. `declared()`
    // used to return an empty allowlist for EVERY failure, and an empty
    // allowlist makes the fold skip the outside-check entirely — so a bundle
    // for a card that had been DELETED printed `OUTSIDE: none`, which reads as
    // clean. The one case where nothing could be checked is the case where a
    // reader most needs to be told so. (Pre-push review, finding #6.)
    let c = CardFile::new("gone", &["src/a.rs"]);
    let path = c.path();
    let led = ledger(&c, &r(2, "t.s1", "tool.write", "allow|x|src/SNEAKY.rs|"));
    // Delete the card AFTER the ledger records it, exactly as a later attest
    // over old history would find it.
    drop(c);
    assert!(std::fs::read_to_string(&path).is_err(), "the card is gone");

    let b = build(&led, "CARD-A").expect("the bundle still assembles");
    assert!(
        !b.card_readable,
        "the bundle must know its card is unreadable"
    );
    let text = crate::attest_verify::render_text(&b);
    assert!(text.contains("OUTSIDE     : UNKNOWN"), "{text}");
    assert!(text.contains("NOTHING was checked"), "{text}");
    assert!(!text.contains("OUTSIDE     : none"), "{text}");
    assert!(
        crate::attest_verify::render_json(&b).contains("\"card_readable\":false"),
        "a machine reader must see it too"
    );
}

#[test]
fn a_readable_card_reports_its_outside_list_normally() {
    let c = CardFile::new("present", &["src/a.rs"]);
    let led = ledger(&c, &r(2, "t.s1", "tool.write", "allow|x|src/a.rs|"));
    let b = build(&led, "CARD-A").expect("bundle");
    assert!(b.card_readable);
    let text = crate::attest_verify::render_text(&b);
    assert!(text.contains("OUTSIDE     : none"), "{text}");
}

#[test]
fn a_card_never_opened_is_an_error_not_an_empty_bundle() {
    assert!(build("", "CARD-MISSING").is_err());
}

#[test]
fn a_card_opened_but_never_closed_is_an_error() {
    // Attesting over a guessed window would invent evidence.
    let c = CardFile::new("unclosed", &["src/a.rs"]);
    let led = r(
        1,
        "t.s1",
        "card.open",
        &format!("open|CARD-A|{}|abc123", c.path()),
    ) + &r(2, "t.s1", "tool.write", "allow|x|src/a.rs|");
    assert!(build(&led, "CARD-A").is_err());
}

#[test]
fn torn_rows_are_counted_into_the_bundle() {
    let c = CardFile::new("torn", &["src/a.rs"]);
    let torn = "{\"seq\":{\"seq\":538,\"v\":5381,\"v\":1,\"id\":\",\"id\":\"x\"}\n";
    let led = ledger(&c, &r(2, "t.s1", "tool.write", "allow|x|src/a.rs|")) + torn;
    assert_eq!(build(&led, "CARD-A").expect("bundle").unreadable, 1);
}

#[test]
fn every_bundle_carries_its_own_limits() {
    // A reader who only ever sees the JSON must still see what it cannot prove.
    assert!(LIMITS.iter().any(|l| l.contains("no exit code")));
    assert!(LIMITS.iter().any(|l| l.contains("never passed")));
    assert!(LIMITS.iter().any(|l| l.contains("under-reports bash")));
}
