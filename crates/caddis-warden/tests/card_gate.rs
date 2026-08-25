//! card_gate.rs — the open card BOUNDS the edits, driven through the real
//! binary (CARD-0111).
//!
//! The verdicts are asserted on the JSON an adapter actually reads, because a
//! gate that behaves one way in-process and another when spawned would be worse
//! than no gate at all.

use std::io::Write;
use std::process::{Command, Stdio};

fn tmp(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "caddis-gate-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ))
}

/// A STRICT card: it declares an EXECUTION contract, so it has an allowlist to
/// enforce. A v1 card would bound nothing, which `card open` says out loud.
fn strict_card(allowlist: &[&str], blast: u32) -> String {
    let items: String = allowlist.iter().map(|p| format!("  - {p}\n")).collect();
    format!(
        "---\nid: CARD-GATE-1\nclass: fix\nowner: t\n---\n\
         # gate fixture\n\n# Done-When\n- done\n\n# RED-TEST\nred\n\n\
         # EXECUTION\nlevel: L1\nblast: {blast}\nclaims-forbidden: true\n\
         allowlist:\n{items}anchors:\n  - path: a.rs\n      content: |\n        x\n"
    )
}

struct Env {
    ledger: std::path::PathBuf,
    card: std::path::PathBuf,
}

impl Env {
    fn new(tag: &str, card_text: &str) -> Self {
        let ledger = tmp(&format!("led-{tag}")).with_extension("jsonl");
        let card = tmp(&format!("card-{tag}")).with_extension("md");
        // swallow: best-effort-cleanup
        let _ = std::fs::remove_file(&ledger);
        std::fs::write(&card, card_text).expect("card written");
        Self { ledger, card }
    }

    fn cmd(&self) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_caddis-warden"));
        c.env("CADDIS_WARDEN_LEDGER", &self.ledger)
            .env("CADDIS_WARDEN_FROM", "peleda.aaaaaaaa");
        c
    }

    fn open_card(&self) {
        let out = self
            .cmd()
            .arg("card")
            .arg("open")
            .arg(&self.card)
            .stdin(Stdio::null())
            .output()
            .expect("spawn");
        assert!(
            out.status.success(),
            "card open failed: {}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// One request frame in, one verdict tag out.
    fn verdict(&self, tool: &str, command: &str, path: &str) -> String {
        let frame = format!(
            "tool {}\n{tool}\ncommand {}\n{command}\npath {}\n{path}\ncontent 0\n\n",
            tool.len(),
            command.len(),
            path.len()
        );
        let mut child = self
            .cmd()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn");
        child
            .stdin
            .as_mut()
            .expect("piped")
            .write_all(frame.as_bytes())
            .expect("frame written");
        let out = child.wait_with_output().expect("finish");
        String::from_utf8_lossy(&out.stdout).into_owned()
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        // swallow: best-effort-cleanup
        let _ = std::fs::remove_file(&self.ledger);
        // swallow: best-effort-cleanup
        let _ = std::fs::remove_file(&self.card);
    }
}

fn tag_of(reply: &str) -> String {
    // The reply is one JSON object: {"verdict":"allow",...}
    let at = reply.find("\"verdict\":\"").expect("a verdict field");
    reply[at + 11..]
        .split('"')
        .next()
        .unwrap_or_default()
        .to_string()
}

#[test]
fn with_no_card_open_every_verdict_is_what_it_always_was() {
    // ⛔ THE REGRESSION THAT MATTERS MOST. The gate must be INVISIBLE until
    // someone opts in by opening a card; if this ever fails, CARD-0111 has
    // leaked into every session that never asked for it.
    let e = Env::new("nocard", &strict_card(&["src/a.rs"], 1));
    assert_eq!(tag_of(&e.verdict("write", "", "src/anything.rs")), "allow");
    assert_eq!(tag_of(&e.verdict("write", "", "/etc/passwd")), "allow");
    assert_eq!(tag_of(&e.verdict("bash", "echo x > src/b.rs", "")), "allow");
}

#[test]
fn a_declared_path_is_allowed() {
    let e = Env::new("declared", &strict_card(&["src/a.rs"], 1));
    e.open_card();
    assert_eq!(tag_of(&e.verdict("write", "", "src/a.rs")), "allow");
}

#[test]
fn an_undeclared_path_is_denied_and_the_reason_names_both() {
    let e = Env::new("undeclared", &strict_card(&["src/a.rs"], 1));
    e.open_card();
    let reply = e.verdict("write", "", "src/b.rs");
    assert_eq!(tag_of(&reply), "deny", "{reply}");
    assert!(reply.contains("CARD-GATE-1"), "name the card: {reply}");
    assert!(reply.contains("src/b.rs"), "name the path: {reply}");
    assert!(
        reply.contains("src/a.rs"),
        "show what WAS declared: {reply}"
    );
}

#[test]
fn a_literal_redirect_target_is_certain_enough_to_deny() {
    let e = Env::new("redirect", &strict_card(&["src/a.rs"], 1));
    e.open_card();
    let reply = e.verdict("bash", "echo hello > src/b.rs", "");
    assert_eq!(tag_of(&reply), "deny", "{reply}");
    assert!(reply.contains("src/b.rs"), "{reply}");
}

#[test]
fn a_target_inferred_from_a_write_verb_only_steers() {
    // Recovered from command text rather than handed over, so it is good enough
    // to warn and never to refuse.
    let e = Env::new("inferred", &strict_card(&["src/a.rs"], 1));
    e.open_card();
    let reply = e.verdict("bash", "cp build/out.bin src/b.rs", "");
    assert_eq!(tag_of(&reply), "steer", "{reply}");
    assert!(reply.contains("card.allowlist"), "{reply}");
}

#[test]
fn an_opaque_command_with_no_recoverable_target_says_nothing() {
    let e = Env::new("opaque", &strict_card(&["src/a.rs"], 1));
    e.open_card();
    assert_eq!(
        tag_of(&e.verdict("bash", "cargo build --release", "")),
        "allow"
    );
}

#[test]
fn the_ledger_is_exempt_whatever_the_card_declares() {
    let e = Env::new("exempt", &strict_card(&["src/a.rs"], 1));
    e.open_card();
    let led = e.ledger.to_string_lossy().into_owned();
    assert_eq!(tag_of(&e.verdict("write", "", &led)), "allow");
    assert_eq!(tag_of(&e.verdict("write", "", "target/debug/x")), "allow");
}

#[test]
fn a_card_may_not_rewrite_itself() {
    let e = Env::new("selfedit", &strict_card(&["src/a.rs"], 1));
    e.open_card();
    let card = e.card.to_string_lossy().into_owned();
    let reply = e.verdict("write", "", &card);
    assert_eq!(tag_of(&reply), "deny", "{reply}");
    assert!(reply.contains("may not rewrite itself"), "{reply}");
}

#[test]
fn reading_is_never_taxed_by_a_card() {
    let e = Env::new("read", &strict_card(&["src/a.rs"], 1));
    e.open_card();
    assert_eq!(tag_of(&e.verdict("read", "", "src/b.rs")), "allow");
    assert_eq!(tag_of(&e.verdict("grep", "", "src/b.rs")), "allow");
}

#[test]
fn a_subtree_declaration_covers_what_is_under_it() {
    let e = Env::new("subtree", &strict_card(&["src/"], 1));
    e.open_card();
    assert_eq!(tag_of(&e.verdict("write", "", "src/deep/a.rs")), "allow");
    assert_eq!(tag_of(&e.verdict("write", "", "other/a.rs")), "deny");
}

#[test]
fn a_stronger_law_keeps_its_own_reason_when_both_would_fire() {
    // The allowlist runs LAST on purpose: every rule above names a defect in
    // the work itself, and the reader deserves that reason rather than a
    // bookkeeping one.
    let e = Env::new("priority", &strict_card(&["src/a.rs"], 1));
    e.open_card();
    let reply = e.verdict("write", "", "some/.credentials.json");
    assert_eq!(tag_of(&reply), "deny");
    assert!(
        reply.contains("never-read-or-expose"),
        "the sensitive-path reason must survive: {reply}"
    );
}
