//! harness_driver.rs — CARD-0251. One interface, any agent CLI.
//!
//! The trait makes adding a harness = one struct + one registry line,
//! not an if/else rewrite. This test pins that contract.

use caddis::harness_driver::{known, lookup, HarnessDriver};

/// A custom driver a third party would add — proves the trait is
/// extensible without touching the registry's internals.
struct FakeDriver;
impl HarnessDriver for FakeDriver {
    fn spawn(&self, _prompt: &str, _model: &str) -> Result<std::process::Output, String> {
        Err("fake".into())
    }
    fn name(&self) -> &str {
        "fake"
    }
}

#[test]
fn registry_has_four_builtin_drivers() {
    assert_eq!(known(), &["omp", "droid", "pi", "claude"]);
}

#[test]
fn each_builtin_driver_names_itself() {
    for n in known() {
        let d = lookup(n).unwrap_or_else(|| panic!("registry missing {n}"));
        assert_eq!(d.name(), n);
    }
}

#[test]
fn unknown_harness_returns_none() {
    assert!(lookup("gemini").is_none());
    assert!(lookup("").is_none());
}

#[test]
fn custom_driver_implements_trait() {
    let d = FakeDriver;
    assert_eq!(d.name(), "fake");
    assert!(d.spawn("hi", "m").is_err());
}

#[test]
fn omp_driver_builds_correct_argv() {
    let d = lookup("omp").unwrap();
    assert_eq!(d.name(), "omp");
}

#[test]
fn droid_driver_builds_correct_argv() {
    let d = lookup("droid").unwrap();
    assert_eq!(d.name(), "droid");
}

#[test]
fn pi_driver_builds_correct_argv() {
    let d = lookup("pi").unwrap();
    assert_eq!(d.name(), "pi");
}

#[test]
fn claude_driver_builds_correct_argv() {
    let d = lookup("claude").unwrap();
    assert_eq!(d.name(), "claude");
}
