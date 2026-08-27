//! ledger_body_multiline.rs — CARD-LEDGER-1, found by the fourth harness
//! during onboarding (DEFECT-ledger-truncates-at-first-newline): the ledger
//! body kept only the text up to the first newline, so a multi-line command
//! with the offence on line two was recorded as `deny|echo harmless|` — the
//! refused command appeared NOWHERE in its own audit row, and the row read as
//! a false positive. The row must carry the whole truth, bounded and honest
//! about elision: newlines preserved (the ledger JSON-escapes them), a hard
//! byte cap with an explicit truncation marker.

use std::io::Write;
use std::process::{Command, Stdio};

fn frame(tool: &str, command: &str) -> Vec<u8> {
    let mut v = Vec::new();
    for (name, body) in [
        ("tool", tool),
        ("command", command),
        ("path", ""),
        ("content", ""),
    ] {
        v.extend_from_slice(format!("{name} {}\n", body.len()).as_bytes());
        v.extend_from_slice(body.as_bytes());
        v.extend_from_slice(b"\n");
    }
    v
}

fn judge(command: &str, tag: &str) -> String {
    let ledger =
        std::env::temp_dir().join(format!("caddis-body-{}-{tag}.jsonl", std::process::id()));
    let _ = std::fs::remove_file(&ledger);
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_caddis-warden"))
        .env("CADDIS_WARDEN_LEDGER", &ledger)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn warden binary");
    cmd.stdin
        .take()
        .expect("stdin")
        .write_all(&frame("bash", command))
        .expect("write frame");
    let _ = cmd.wait_with_output().expect("warden exits");
    let rows = std::fs::read_to_string(&ledger).unwrap_or_default();
    let _ = std::fs::remove_file(&ledger);
    rows
}

#[test]
fn the_offending_line_survives_in_its_own_row() {
    // The defect's row B: danger on line TWO. Judgement must still deny, and
    // the row must now carry the line that was refused.
    let rows = judge("echo harmless\ngit push --force origin main", "ml");
    assert!(
        rows.contains("git push --force origin main"),
        "the refused command must appear in its own audit row, got: {rows}"
    );
    assert_eq!(
        rows.lines().count(),
        1,
        "one JSON row — the ledger escapes newlines"
    );
}

#[test]
fn a_single_line_command_is_recorded_whole() {
    let rows = judge("echo ok", "sl");
    assert!(
        rows.contains("allow|echo ok|"),
        "the anchor case is unchanged, got: {rows}"
    );
}

#[test]
fn an_overlong_command_says_it_was_truncated() {
    let long = format!("echo {}", "a".repeat(5_000));
    let rows = judge(&long, "cap");
    assert!(
        rows.contains("bytes truncated]"),
        "elision must be explicit, never masquerading as the whole command, got: {rows}"
    );
    assert!(
        rows.len() < 1_200,
        "the row stays bounded, got {} bytes",
        rows.len()
    );
}

#[test]
fn an_elided_head_still_explains_its_own_refusal() {
    // The reporter's re-verification (section 9): >500 bytes of padding with
    // the offence past the cap. The head is worthless — the row must still
    // say WHICH LAW fired: the reason's first line names the law id and, for
    // shell-grammar laws, quotes the spelling it fired on. The promise lives
    // in the reason, not in the head.
    let padding = format!("echo {}\n", "a".repeat(600));
    let rows = judge(&format!("{padding}git push --force origin main"), "elide");
    assert!(
        rows.contains("git.push.force-to-protected"),
        "the law id must survive its own elision, got: {rows}"
    );
}

#[test]
fn a_secret_refusal_never_persists_the_secret() {
    // The refusal's reason teaches the doctrine without quoting the literal;
    // the row inherits that discipline — and the HEAD too: a command that
    // carries a credential-shaped run is masked at rest, so the audit trail
    // cannot become a keychain. The literal is built at runtime from pieces,
    // as the warden's own guidance prescribes for tests needing the shape.
    let key = format!("s{}{}", "k-", "aB1".repeat(11));
    let rows = judge(&format!("echo key = \"{key}\""), "secret");
    assert!(
        rows.contains("***redacted"),
        "a credential-shaped run in the head is masked at rest, got: {rows}"
    );
    assert!(
        !rows.contains(&key),
        "the secret literal must never persist into the ledger, got: {rows}"
    );
}

#[test]
fn a_secret_row_carries_pre_redaction_fingerprint() {
    let key = format!("s{}{}", "k-", "aB1".repeat(11));
    let command = format!("echo key = \"{key}\"");
    let rows = judge(&command, "fp");
    let fp = format!("{:016x}", caddis_warden::identity::fnv1a(&command));
    assert!(
        rows.contains(&fp),
        "pre-redaction fingerprint {fp} missing: {rows}"
    );
    assert!(
        !rows.contains(&key),
        "the secret literal must never persist: {rows}"
    );
}
