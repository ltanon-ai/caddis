//! attest_cli.rs — the cherry, driven end to end through the real binary
//! (CARD-0114).
//!
//! ⛔ THE TWO TAMPER CASES ARE THE POINT. A verifier nobody has watched go RED
//! is `assert(true == true)` with extra steps, so this file edits a bundle by
//! hand and requires `--verify` to refuse it.

use std::process::{Command, Stdio};

fn tmp(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "caddis-att-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ))
}

struct Env {
    ledger: std::path::PathBuf,
    card: std::path::PathBuf,
    bundle: std::path::PathBuf,
}

impl Env {
    fn new(tag: &str) -> Self {
        let e = Self {
            ledger: tmp(&format!("l{tag}")).with_extension("jsonl"),
            card: tmp(&format!("c{tag}")).with_extension("md"),
            bundle: tmp(&format!("b{tag}")).with_extension("json"),
        };
        // swallow: best-effort-cleanup
        let _ = std::fs::remove_file(&e.ledger);
        std::fs::write(
            &e.card,
            "---\nid: CARD-ATT\nclass: fix\nowner: t\n---\n# a\n\n# Done-When\n- done\n\n\
             # RED-TEST\ncargo test --workspace fails before this change\n\n\
             # EXECUTION\nlevel: L1\nblast: 2\nclaims-forbidden: true\n\
             allowlist:\n  - src/a.rs\nanchors:\n  - path: a.rs\n      content: |\n        x\n",
        )
        .expect("card written");
        e
    }

    fn cmd(&self) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_caddis-warden"));
        c.env("CADDIS_WARDEN_LEDGER", &self.ledger)
            .env("CADDIS_WARDEN_FROM", "peleda.aaaaaaaa")
            .stdin(Stdio::null());
        c
    }

    fn run(&self, args: &[&str]) -> (String, String, i32) {
        let o = self.cmd().args(args).output().expect("spawn");
        (
            String::from_utf8_lossy(&o.stdout).into_owned(),
            String::from_utf8_lossy(&o.stderr).into_owned(),
            o.status.code().unwrap_or(-1),
        )
    }

    /// Open a card, record one write, close it — a whole unit of work.
    fn do_a_unit_of_work(&self, write_to: &str) {
        let card = self.card.to_string_lossy().into_owned();
        assert_eq!(self.run(&["card", "open", &card]).2, 0, "open failed");
        // The write goes through the FRAME path so it is judged and recorded
        // exactly as a real tool call would be.
        let frame = format!(
            "tool 5\nwrite\ncommand 0\n\npath {}\n{}\ncontent 0\n\n",
            write_to.len(),
            write_to
        );
        let mut ch = self
            .cmd()
            .stdout(Stdio::null())
            .stdin(Stdio::piped())
            .spawn()
            .expect("spawn");
        {
            use std::io::Write;
            ch.stdin
                .as_mut()
                .expect("piped")
                .write_all(frame.as_bytes())
                .expect("frame");
        }
        ch.wait().expect("finish");
        assert_eq!(self.run(&["card", "close"]).2, 0, "close failed");
    }

    fn make_bundle(&self) -> String {
        let (out, err, code) = self.run(&["attest", "--card", "CARD-ATT", "--json"]);
        assert_eq!(code, 0, "attest failed: {out}{err}");
        std::fs::write(&self.bundle, &out).expect("bundle written");
        out
    }

    fn verify(&self) -> (String, String, i32) {
        let p = self.bundle.to_string_lossy().into_owned();
        self.run(&["attest", "--verify", &p])
    }

    fn tamper(&self, from: &str, to: &str) {
        let j = std::fs::read_to_string(&self.bundle).expect("bundle readable");
        assert!(
            j.contains(from),
            "tamper target `{from}` absent from bundle"
        );
        std::fs::write(&self.bundle, j.replace(from, to)).expect("tampered");
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        for p in [&self.ledger, &self.card, &self.bundle] {
            // swallow: best-effort-cleanup
            let _ = std::fs::remove_file(p);
        }
    }
}

#[test]
fn a_unit_worked_inside_its_allowlist_attests_clean_and_verifies() {
    let e = Env::new("clean");
    e.do_a_unit_of_work("src/a.rs");
    let json = e.make_bundle();
    assert!(json.contains("\"card_id\":\"CARD-ATT\""), "{json}");
    assert!(json.contains("\"files_outside_allowlist\":[]"), "{json}");

    let (out, err, code) = e.verify();
    assert_eq!(code, 0, "a true bundle must verify: {out}{err}");
    assert!(out.contains("ALL"), "{out}");
    assert!(out.contains("CONFIRMED"), "{out}");
    assert!(!out.contains("CONTRADICTED"), "{out}");
}

#[test]
fn a_write_outside_the_allowlist_appears_in_the_bundle() {
    // The warden DENIES this write, so it is recorded as a denial rather than
    // as a file — and the bundle shows the denial, which is the honest record
    // of what happened. Both halves matter: the deny count moves, and a reader
    // can see the unit did something its card did not declare.
    let e = Env::new("stray");
    e.do_a_unit_of_work("src/STRAY.rs");
    let json = e.make_bundle();
    assert!(json.contains("\"deny\":1"), "{json}");
    let (out, _, code) = e.verify();
    assert_eq!(code, 0, "{out}");
}

#[test]
fn a_bundle_with_an_edited_verdict_count_is_refused() {
    // ⛔ RED CASE 1. If this passes, `--verify` is doing nothing at all.
    let e = Env::new("tamper1");
    e.do_a_unit_of_work("src/a.rs");
    e.make_bundle();
    assert_eq!(e.verify().2, 0, "the untouched bundle must verify first");

    e.tamper("\"deny\":0", "\"deny\":7");
    let (out, _, code) = e.verify();
    assert_ne!(code, 0, "a tampered bundle must be refused: {out}");
    assert!(out.contains("CONTRADICTED"), "{out}");
    assert!(out.contains("deny"), "name the contradicted claim: {out}");
}

#[test]
fn a_bundle_whose_outside_list_was_emptied_is_refused() {
    // ⛔ RED CASE 2 — the most attractive field to tamper with, because it is
    // the one a dishonest bundle most wants to be empty.
    let e = Env::new("tamper2");
    e.do_a_unit_of_work("src/a.rs");
    e.make_bundle();
    // Give the ledger a real stray write to list, recorded directly so the
    // gate's denial does not turn it into a verdict instead of a file.
    let extra = "{\"seq\":9001,\"v\":1,\"id\":\"z\",\"idem_key\":\"z\",\"type\":\"tool.write\",\
         \"from\":\"peleda.aaaaaaaa\",\"to\":\"warden\",\"body\":\"allow|x|src/LATE.rs|\",\
         \"ts\":10}\n";
    let text = std::fs::read_to_string(&e.ledger).expect("readable");
    // Splice it INSIDE the window: before the close row, which is last.
    let (head, close) = text.rsplit_once('\n').map(|(h, _)| (h, "")).expect("rows");
    let close_row = head.rsplit_once('\n').map(|(_, c)| c).expect("close row");
    let body = head.rsplit_once('\n').map(|(h, _)| h).expect("head");
    std::fs::write(&e.ledger, format!("{body}\n{extra}{close_row}\n{close}")).expect("spliced");

    let json = e.make_bundle();
    assert!(
        json.contains("src/LATE.rs"),
        "the stray must be listed: {json}"
    );
    assert_eq!(e.verify().2, 0, "the honest bundle verifies");

    e.tamper("[\"src/LATE.rs\"]", "[]");
    let (out, _, code) = e.verify();
    assert_ne!(code, 0, "an emptied outside-list must be refused: {out}");
    assert!(out.contains("files_outside_count"), "{out}");
}

#[test]
fn attesting_a_card_that_was_never_opened_is_an_error() {
    let e = Env::new("missing");
    let (out, err, code) = e.run(&["attest", "--card", "CARD-NOPE"]);
    assert_ne!(code, 0);
    assert!(err.contains("CARD-NOPE"), "{err}");
    assert!(out.is_empty(), "no bundle may be emitted: {out}");
}

#[test]
fn attest_without_an_argument_is_a_usage_error() {
    let e = Env::new("usage");
    let (_, err, code) = e.run(&["attest"]);
    assert_eq!(code, 2);
    assert!(err.contains("usage:"), "{err}");
}
