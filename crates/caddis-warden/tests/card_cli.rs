//! card_cli.rs — the six RED behaviours of CARD-0110, driven through the real
//! binary because that is what a user runs.
//!
//! Each test gets its OWN ledger and its own card file, named by process and
//! thread, so the suite is parallel-safe and never touches the operator's real
//! ledger at `~/.caddis/warden-ledger.jsonl`.

use std::process::{Command, Stdio};

fn tmp(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "caddis-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ))
}

const CARD_V1: &str = "---\nid: CARD-TEST-1\nclass: fix\nowner: t\n---\n\
# a test card\n\n# Done-When\n- the test passes\n\n# RED-TEST\nit failed before\n";

/// Run `card ...` as the caller `from`, against the ledger at `ledger`.
fn card(ledger: &std::path::Path, from: &str, args: &[&str]) -> (String, String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_caddis-warden"))
        .arg("card")
        .args(args)
        .env("CADDIS_WARDEN_LEDGER", ledger)
        .env("CADDIS_WARDEN_FROM", from)
        .stdin(Stdio::null())
        .output()
        .expect("the binary must spawn");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

struct Fixture {
    ledger: std::path::PathBuf,
    card_path: std::path::PathBuf,
}

impl Fixture {
    fn new(tag: &str) -> Self {
        let ledger = tmp(&format!("led-{tag}")).with_extension("jsonl");
        let card_path = tmp(&format!("card-{tag}")).with_extension("md");
        // swallow: best-effort-cleanup
        let _ = std::fs::remove_file(&ledger);
        std::fs::write(&card_path, CARD_V1).expect("card fixture written");
        Self { ledger, card_path }
    }
    fn card_arg(&self) -> String {
        self.card_path.to_string_lossy().into_owned()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // swallow: best-effort-cleanup
        let _ = std::fs::remove_file(&self.ledger);
        // swallow: best-effort-cleanup
        let _ = std::fs::remove_file(&self.card_path);
    }
}

#[test]
fn open_then_status_reports_the_card() {
    let f = Fixture::new("s1");
    let (out, err, code) = card(&f.ledger, "peleda.aaaaaaaa", &["open", &f.card_arg()]);
    assert_eq!(code, 0, "open failed: {out}{err}");
    assert!(out.contains("CARD-TEST-1"), "{out}");

    let (out, _, code) = card(&f.ledger, "peleda.aaaaaaaa", &["status"]);
    assert_eq!(code, 0);
    assert!(out.contains("card open for peleda.aaaaaaaa"), "{out}");
    assert!(out.contains("CARD-TEST-1"), "{out}");
    // Always stated, so "nothing open" is distinguishable from "damaged".
    assert!(out.contains("ledger lines unreadable: 0"), "{out}");
}

#[test]
fn a_v1_card_says_plainly_that_it_bounds_nothing() {
    // Most of this repository's own cards are v1: no EXECUTION section, so no
    // allowlist, so nothing for a gate to bound writes WITH. Saying so at open
    // is the difference between a mechanism and a reassuring noise.
    let f = Fixture::new("s1b");
    let (out, _, code) = card(&f.ledger, "peleda.aaaaaaaa", &["open", &f.card_arg()]);
    assert_eq!(code, 0);
    assert!(out.contains("NOT BOUNDED"), "{out}");
}

#[test]
fn opening_twice_refuses_and_names_the_card_already_open() {
    let f = Fixture::new("s2");
    let (_, _, code) = card(&f.ledger, "peleda.aaaaaaaa", &["open", &f.card_arg()]);
    assert_eq!(code, 0);
    let (_, err, code) = card(&f.ledger, "peleda.aaaaaaaa", &["open", &f.card_arg()]);
    assert_ne!(code, 0, "a second open must refuse");
    assert!(err.contains("CARD-TEST-1"), "name the offender: {err}");
    assert!(err.contains("already open"), "{err}");
}

#[test]
fn a_caller_with_no_session_component_cannot_hold_a_card() {
    let f = Fixture::new("s3");
    let (_, err, code) = card(&f.ledger, "peleda", &["open", &f.card_arg()]);
    assert_ne!(code, 0, "a bare harness label must refuse");
    assert!(err.contains("harness, not a session"), "{err}");
    assert!(
        err.contains("CADDIS_WARDEN_FROM"),
        "say how to fix it: {err}"
    );
}

#[test]
fn editing_the_card_between_open_and_close_refuses_the_close() {
    let f = Fixture::new("s4");
    let (_, _, code) = card(&f.ledger, "peleda.aaaaaaaa", &["open", &f.card_arg()]);
    assert_eq!(code, 0);
    // ONE byte. An executor that can rewrite its own allowlist mid-card has no
    // card at all.
    std::fs::write(&f.card_path, format!("{CARD_V1}x")).expect("card edited");
    let (_, err, code) = card(&f.ledger, "peleda.aaaaaaaa", &["close"]);
    assert_ne!(code, 0, "a changed card must not close: {err}");
    assert!(err.contains("changed since it was opened"), "{err}");
}

#[test]
fn an_unedited_card_closes_and_then_no_card_is_open() {
    let f = Fixture::new("s4b");
    assert_eq!(
        card(&f.ledger, "peleda.aaaaaaaa", &["open", &f.card_arg()]).2,
        0
    );
    let (out, err, code) = card(&f.ledger, "peleda.aaaaaaaa", &["close"]);
    assert_eq!(code, 0, "{out}{err}");
    assert!(out.contains("card close: CARD-TEST-1"), "{out}");
    // The honest caveat travels with the close, every time.
    assert!(out.contains("cannot prove the RED-TEST passed"), "{out}");

    let (out, _, _) = card(&f.ledger, "peleda.aaaaaaaa", &["status"]);
    assert!(out.contains("no card open"), "{out}");
}

#[test]
fn one_sessions_card_is_invisible_to_another_session() {
    // THE TEST A SIDE STATE FILE WOULD SILENTLY PASS, and the whole reason the
    // quorum ruled this state must be derived from the ledger.
    let f = Fixture::new("s5");
    assert_eq!(
        card(&f.ledger, "peleda.aaaaaaaa", &["open", &f.card_arg()]).2,
        0
    );

    let (out, _, code) = card(&f.ledger, "peleda.bbbbbbbb", &["status"]);
    assert_eq!(code, 0);
    assert!(
        out.contains("no card open for peleda.bbbbbbbb"),
        "session B must not see session A's card: {out}"
    );
    // And B cannot close A's card either.
    let (_, err, code) = card(&f.ledger, "peleda.bbbbbbbb", &["close"]);
    assert_ne!(code, 0, "B closed A's card: {err}");
}

#[test]
fn a_torn_ledger_row_is_counted_and_still_resolves_the_card() {
    let f = Fixture::new("s6");
    assert_eq!(
        card(&f.ledger, "peleda.aaaaaaaa", &["open", &f.card_arg()]).2,
        0
    );
    // A real interleaved row from the live ledger, appended after the open.
    let torn = "{\"seq\":{\"seq\":538,\"v\":5381,\"v\":1,\"id\":\",\"id\":\"wardnf7acdbc1\"}\n";
    let mut text = std::fs::read_to_string(&f.ledger).expect("readable");
    text.push_str(torn);
    std::fs::write(&f.ledger, text).expect("written");

    let (out, _, code) = card(&f.ledger, "peleda.aaaaaaaa", &["status"]);
    assert_eq!(code, 0);
    assert!(
        out.contains("CARD-TEST-1"),
        "damage must not lose a good row: {out}"
    );
    assert!(out.contains("ledger lines unreadable: 1"), "{out}");
}

#[test]
fn closing_with_nothing_open_is_an_error_not_a_silent_success() {
    let f = Fixture::new("s7");
    let (_, err, code) = card(&f.ledger, "peleda.aaaaaaaa", &["close"]);
    assert_ne!(code, 0);
    assert!(err.contains("no card is open"), "{err}");
}

#[test]
fn a_file_that_is_not_a_card_is_refused_with_the_schema_reason() {
    let f = Fixture::new("s8");
    std::fs::write(&f.card_path, "just some prose, no frontmatter").expect("written");
    let (_, err, code) = card(&f.ledger, "peleda.aaaaaaaa", &["open", &f.card_arg()]);
    assert_ne!(code, 0);
    assert!(err.to_lowercase().contains("card"), "{err}");
}

#[test]
fn an_unknown_card_subcommand_is_a_usage_error() {
    let f = Fixture::new("s9");
    let (_, err, code) = card(&f.ledger, "peleda.aaaaaaaa", &["frobnicate"]);
    assert_eq!(code, 2);
    assert!(err.contains("usage:"), "{err}");
}

#[test]
fn close_verify_nonzero_leaves_card_open() {
    let f = Fixture::new("v1");
    let from = "peleda.aaaaaaaa";
    assert_eq!(card(&f.ledger, from, &["open", &f.card_arg()]).2, 0);
    let (out, err, code) = card(
        &f.ledger,
        from,
        &["close", "--verify", "--", "python", "-c", "raise SystemExit(1)"],
    );
    assert_ne!(code, 0, "nonzero verify must not close: {out}{err}");
    let (st, _, _) = card(&f.ledger, from, &["status"]);
    assert!(st.contains("CARD-TEST-1"), "card must stay open: {st}");
}

#[test]
fn close_verify_zero_closes() {
    let f = Fixture::new("v0");
    let from = "peleda.aaaaaaaa";
    assert_eq!(card(&f.ledger, from, &["open", &f.card_arg()]).2, 0);
    let (out, err, code) = card(
        &f.ledger,
        from,
        &["close", "--verify", "--", "python", "-c", "raise SystemExit(0)"],
    );
    assert_eq!(code, 0, "zero verify must close: {out}{err}");
    let led = std::fs::read_to_string(&f.ledger).unwrap();
    assert!(led.contains("card.verify"), "verify row missing: {led}");
    let (st, _, _) = card(&f.ledger, from, &["status"]);
    assert!(st.contains("no card open"), "{st}");
}

#[test]
fn close_verify_writes_attest_bundle() {
    let f = Fixture::new("va");
    let from = "peleda.aaaaaaaa";
    assert_eq!(card(&f.ledger, from, &["open", &f.card_arg()]).2, 0);
    let (out, err, code) = card(
        &f.ledger,
        from,
        &["close", "--verify", "--", "python", "-c", "raise SystemExit(0)"],
    );
    assert_eq!(code, 0, "zero verify must close: {out}{err}");
    let bundle = f.card_path.with_extension("attest.json");
    let body = std::fs::read_to_string(&bundle).unwrap_or_else(|e| {
        panic!(
            "bundle missing at {}: {e}\nout={out}\nerr={err}",
            bundle.display()
        )
    });
    assert!(body.contains("CARD-TEST-1"), "{body}");
    let _ = std::fs::remove_file(&bundle);
}

