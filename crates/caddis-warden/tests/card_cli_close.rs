//! card_cli_close.rs — close --verify tests split from card_cli.rs (280 cap).

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
fn close_verify_nonzero_leaves_card_open() {
    let f = Fixture::new("v1");
    let from = "peleda.aaaaaaaa";
    assert_eq!(card(&f.ledger, from, &["open", &f.card_arg()]).2, 0);
    let (out, err, code) = card(
        &f.ledger,
        from,
        &[
            "close",
            "--verify",
            "--",
            "python",
            "-c",
            "raise SystemExit(1)",
        ],
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
        &[
            "close",
            "--verify",
            "--",
            "python",
            "-c",
            "raise SystemExit(0)",
        ],
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
        &[
            "close",
            "--verify",
            "--",
            "python",
            "-c",
            "raise SystemExit(0)",
        ],
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
