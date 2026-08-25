//! Direct tests for the human-facing arguments (CARD-0107).
//!
//! `tests/cli_flags.rs` pins the same contract through a spawned process and
//! stays exactly as it is. These reach the decision itself, so a regression
//! names the function instead of a stdout diff — and they run without a spawn,
//! which is what let the whole module read 0% covered for a release.

use super::*;

#[test]
fn version_and_help_are_answered_on_both_spellings() {
    for arg in ["--version", "-V"] {
        match for_flag(arg) {
            Reply::Out(text) => assert!(text.starts_with("caddis-warden "), "got: {text}"),
            Reply::Err(text, code) => panic!("{arg} must not be an error ({code}): {text}"),
        }
    }
    for arg in ["--help", "-h"] {
        match for_flag(arg) {
            Reply::Out(text) => assert_eq!(text, USAGE),
            Reply::Err(text, code) => panic!("{arg} must not be an error ({code}): {text}"),
        }
    }
}

#[test]
fn an_unknown_argument_is_a_usage_error_and_never_a_verdict() {
    // THE DEFECT THIS CLOSES: an unrecognised argument used to fall through to
    // the frame parser, so `--version` was answered with a DENIAL and exit 0.
    match for_flag("--frobnicate") {
        Reply::Out(text) => panic!("a misuse must not succeed: {text}"),
        Reply::Err(text, code) => {
            assert_eq!(code, 2, "a misuse exits non-zero");
            assert!(text.contains("--frobnicate"), "name the offender: {text}");
            assert!(text.contains("USAGE"), "and show usage: {text}");
            assert!(
                !text.contains("\"verdict\""),
                "no verdict may be emitted for a misuse: {text}"
            );
        }
    }
}

#[test]
fn the_empty_argument_is_a_usage_error_not_a_silent_success() {
    match for_flag("") {
        Reply::Out(text) => panic!("an empty argument is not a request: {text}"),
        Reply::Err(_, code) => assert_eq!(code, 2),
    }
}

#[test]
fn version_says_which_release_it_is_or_says_plainly_that_it_is_not_one() {
    // A downloaded binary must be able to answer "which release am I". The
    // crate version alone cannot: crates keep their own versions while the
    // product version lives in the tag, so a v0.2.x download would report
    // "0.1.0" and its holder would reasonably conclude they had the wrong file.
    let line = version_line();
    assert!(line.starts_with("caddis-warden "), "got: {line}");
    assert!(line.contains(env!("CARGO_PKG_VERSION")), "got: {line}");
    let stamped = option_env!("CADDIS_RELEASE").unwrap_or("");
    if stamped.is_empty() {
        assert!(
            line.contains("unreleased build"),
            "an unstamped build must NOT imply a release it is not: {line}"
        );
    } else {
        assert!(
            line.contains(stamped),
            "the stamped release must show: {line}"
        );
    }
}

#[test]
fn usage_documents_every_path_a_caller_can_take() {
    // A usage text that omits a subcommand is how a feature becomes invisible.
    for path in [
        "--replay",
        "report",
        "card open",
        "card status",
        "card close",
        "--version",
        "--help",
    ] {
        assert!(USAGE.contains(path), "USAGE never mentions {path}");
    }
    assert!(
        USAGE.contains("stdin"),
        "the frame path is the one an adapter takes and must be documented"
    );
}
