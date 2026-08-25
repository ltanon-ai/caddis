//! Direct tests for caller identity and the ledger path (CARD-0110).
//!
//! These use process-wide environment variables, so they must not run
//! concurrently with each other. `cargo test` threads a test binary, so each
//! case takes a shared mutex rather than trusting luck — a flaky identity test
//! would be worse than none, because it would train a reader to re-run it.

use super::*;
use std::sync::{Mutex, MutexGuard, OnceLock};

fn env_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[test]
fn an_unset_caller_falls_back_rather_than_losing_the_row() {
    let _g = env_lock();
    std::env::remove_var("CADDIS_WARDEN_FROM");
    // The envelope rejects an empty `from`, so an unusable value must degrade
    // to a name, never to nothing.
    assert_eq!(caller_id(), "omp");
}

#[test]
fn a_session_scoped_caller_survives_sanitizing_intact() {
    let _g = env_lock();
    std::env::set_var("CADDIS_WARDEN_FROM", "peleda.a1b2c3d4");
    assert_eq!(caller_id(), "peleda.a1b2c3d4");
    std::env::remove_var("CADDIS_WARDEN_FROM");
}

#[test]
fn a_hostile_caller_cannot_corrupt_the_jsonl_row() {
    let _g = env_lock();
    std::env::set_var("CADDIS_WARDEN_FROM", "pe\"le|da\nx");
    let id = caller_id();
    for bad in ['"', '|', '\n', '\\'] {
        assert!(!id.contains(bad), "{bad:?} survived into {id}");
    }
    std::env::remove_var("CADDIS_WARDEN_FROM");
}

#[test]
fn an_overlong_caller_is_capped_and_never_truncates_into_the_next_field() {
    let _g = env_lock();
    std::env::set_var("CADDIS_WARDEN_FROM", "a".repeat(200));
    assert_eq!(caller_id().len(), 32);
    std::env::remove_var("CADDIS_WARDEN_FROM");
}

#[test]
fn session_scoping_needs_both_halves_not_just_a_dot() {
    assert!(is_session_scoped("peleda.a1b2c3d4"));
    assert!(is_session_scoped("omp.deadbeef"));
    // A bare harness label holds no card: every session in it stamps the same
    // string, so one session's card would bound another's writes.
    assert!(!is_session_scoped("peleda"));
    assert!(!is_session_scoped("omp"));
    assert!(!is_session_scoped(""));
    // A dot alone is not a session.
    assert!(!is_session_scoped("peleda."));
    assert!(!is_session_scoped(".a1b2c3d4"));
}

#[test]
fn the_ledger_override_wins_and_an_empty_one_does_not() {
    let _g = env_lock();
    std::env::set_var("CADDIS_WARDEN_LEDGER", "C:/somewhere/led.jsonl");
    assert_eq!(
        ledger_path(),
        std::path::PathBuf::from("C:/somewhere/led.jsonl")
    );
    // An empty override is not an override; falling through to the default is
    // what keeps a blank environment variable from redirecting the ledger to "".
    std::env::set_var("CADDIS_WARDEN_LEDGER", "");
    assert!(ledger_path().ends_with("warden-ledger.jsonl"));
    std::env::remove_var("CADDIS_WARDEN_LEDGER");
}

#[test]
fn the_hash_changes_when_one_byte_changes() {
    // What the card hash is FOR: detecting an edit between open and close.
    assert_ne!(
        fnv1a("# CARD-0110\nblast: 1"),
        fnv1a("# CARD-0110\nblast: 2")
    );
    assert_eq!(fnv1a("same"), fnv1a("same"));
}
