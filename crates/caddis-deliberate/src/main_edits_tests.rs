//! main_edits_tests.rs — P4 s4a gates for the `edits` CLI CONTRACT the verbs
//! stand on: path derivation from `--home`, the estate warden default (it
//! must NOT follow `--home` — a sandbox home still gates against the real
//! ledger), transport-identity defaults, unknown-flag and missing-value
//! rejection. Verb BEHAVIOR (propose/confirm/refuse semantics) is proven by
//! edits_tests.rs at the library seam and .e2e-deliberate-edits.py against
//! the release binary — these tests pin only the CLI edge.

use super::*;

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

#[test]
fn edits_args_derive_stream_view_journal_from_home() {
    let a = parse_edits_args(&args(&["--home", "C:/tmp/sbx"])).unwrap();
    assert_eq!(a.stream, PathBuf::from("C:/tmp/sbx").join("seats.jsonl"));
    assert_eq!(a.view, PathBuf::from("C:/tmp/sbx").join("seats-view.json"));
    assert_eq!(a.journal, PathBuf::from("C:/tmp/sbx").join("edits.jsonl"));
}

#[test]
fn edits_args_warden_default_is_the_estate_ledger_not_the_home() {
    // --home points the EDIT paths at a sandbox; the warden GATE must still
    // read the estate ledger unless --warden explicitly overrides. If the
    // default followed --home, a sandbox home would silently gate itself
    // against a ledger that can never hold a card.
    let a = parse_edits_args(&args(&["--home", "C:/tmp/sbx"])).unwrap();
    assert!(
        a.warden.ends_with(".caddis/warden-ledger.jsonl"),
        "warden default must be the estate ledger, got {}",
        a.warden.display()
    );
    let b = parse_edits_args(&args(&["--warden", "C:/tmp/w.jsonl"])).unwrap();
    assert_eq!(b.warden, PathBuf::from("C:/tmp/w.jsonl"));
}

#[test]
fn edits_args_identity_defaults_to_the_terminal_transport() {
    let a = parse_edits_args(&args(&[])).unwrap();
    assert_eq!(a.actor, "terminal");
    assert_eq!(a.actor_kind, "terminal");
    let b = parse_edits_args(&args(&["--actor", "world.panel", "--actor-kind", "world"])).unwrap();
    assert_eq!(b.actor, "world.panel");
    assert_eq!(b.actor_kind, "world");
}

#[test]
fn edits_args_reject_unknown_flags_and_missing_values() {
    assert!(parse_edits_args(&args(&["--bogus"])).is_err());
    assert!(
        parse_edits_args(&args(&["--op"])).is_err(),
        "valueless --op"
    );
    assert!(
        parse_edits_args(&args(&["--card"])).is_err(),
        "valueless --card"
    );
    assert!(
        parse_edits_args(&args(&["--id"])).is_err(),
        "valueless --id"
    );
    assert!(
        parse_edits_args(&args(&["--warden"])).is_err(),
        "valueless --warden"
    );
}
