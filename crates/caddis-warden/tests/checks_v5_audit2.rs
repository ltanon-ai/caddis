//! checks_v5_audit2.rs — the defects a SECOND clean agent found in the FIX.
//!
//! ⭐ THE STANDING LESSON, now proven twice: CARD-WARDEN-7 was itself a fix
//! commit written under the belief that the problem was understood, and it
//! carried a CRITICAL bypass of the very check it was fixing plus a NEW
//! regression it introduced while calling that area "no wrong verdict". A fix
//! commit is the highest-risk kind, and the builder cannot audit it.
//!
//! Every test here failed when written.

use caddis_warden::checks::cmdline::segments;
use caddis_warden::checks::{git, incidents, shell};

// ------------------- 1 · CRITICAL: the fallback dropped the whole line

#[test]
fn a_quoted_separator_inside_the_dangerous_segment_survives_the_fallback() {
    // An unbalanced quote LATER forces the fallback; the naive splitter then cut
    // through the quotes in the force-push itself, leaving every piece
    // mismatched, so all were dropped and the verdict was ALLOW.
    for cmd in [
        "git push --force \"x && y\" origin main && echo don't",
        "git push --force origin \"a;b\" main && echo don't",
    ] {
        assert!(
            git::force_push_to_protected(cmd).is_some(),
            "the force-push must survive the degraded path: {cmd}"
        );
    }
}

#[test]
fn the_fallback_keeps_a_quoted_separator_as_one_token() {
    let got = segments("git push --force \"x && y\" origin main && echo don't");
    assert!(!got.is_empty(), "the line must still yield segments");
    assert!(
        got[0].iter().any(|t| t == "x && y"),
        "a well-formed quote must not be cut through even on the degraded path: {got:?}"
    );
}

// -------- 2 · HIGH: runner flags that take no value swallowed `git`

#[test]
fn a_boolean_runner_flag_does_not_swallow_the_command() {
    // `sudo -S` (read password from stdin) and `sudo -h` are BOOLEAN. Treating
    // them as value-taking consumed the literal token `git`.
    for cmd in [
        "sudo -S git push --force origin main",
        "sudo -h git push --force origin main",
        "command -p git push --force origin main",
    ] {
        assert!(
            git::force_push_to_protected(cmd).is_some(),
            "a boolean runner flag must not hide the push: {cmd}"
        );
    }
}

#[test]
fn a_genuinely_value_taking_runner_flag_still_skips_its_value() {
    assert!(git::force_push_to_protected("sudo -u root git push --force origin main").is_some());
    assert!(git::force_push_to_protected("env -u PATH git push --force origin main").is_some());
}

#[test]
fn the_common_wrapper_commands_are_recognised() {
    for cmd in [
        "timeout 30 git push --force origin main",
        "nice -n 10 git push --force origin main",
        "doas git push --force origin main",
    ] {
        assert!(
            git::force_push_to_protected(cmd).is_some(),
            "wrapper not recognised: {cmd}"
        );
    }
}

#[test]
fn a_wrapper_around_a_non_git_command_is_still_not_git() {
    for cmd in [
        "sudo docker run --rm git push --force origin main",
        "timeout 30 docker run git push --force origin main",
    ] {
        assert_eq!(
            git::force_push_to_protected(cmd),
            None,
            "`git` here is an ARGUMENT, not the command: {cmd}"
        );
    }
}

/// ⛔ SUPERSEDED BY CARD-WARDEN-19, and the supersession is the finding. This
/// file used to pin `sudo sh -c "git push --force origin main"` as ALLOW with
/// the rationale "`git` here is an ARGUMENT" — but `sh -c` makes the string
/// an EXECUTED command line, and sudo runs it as root. The pin encoded the
/// carrier-class blind spot (HIGH 4) as expected behaviour; audit 4 listed
/// `bash -c 'git push …'` → ALLOW among the defects. It now denies, pinned
/// here so the change is deliberate rather than discovered.
#[test]
fn a_shell_carrier_string_is_an_executed_command_line() {
    assert!(
        git::force_push_to_protected("sudo sh -c \"git push --force origin main\"").is_some(),
        "the string after -c runs; it is not an argument"
    );
}

// ---- 3 · HIGH: a REGRESSION I INTRODUCED — trailing backslash on a path

#[test]
fn a_windows_path_ending_in_a_backslash_does_not_hide_the_push() {
    // Utterly ordinary on this estate: a path pasted from Explorer, or
    // tab-completed, ends in a backslash. My finding-6 fix made `\"` escape the
    // closing quote, so the whole command became one unterminated token.
    for cmd in [
        "git -C \"C:\\Users\\ashpac\\repo\\\" push --force origin main",
        "git -C C:\\Users\\ashpac\\repo\\ push --force origin main",
    ] {
        assert!(
            git::force_push_to_protected(cmd).is_some(),
            "a trailing backslash must not hide a force-push: {cmd}"
        );
    }
}

#[test]
fn a_unc_path_keeps_both_leading_backslashes() {
    let got = segments("git -C \\\\server\\share\\repo push --force origin main");
    assert!(!got.is_empty());
    assert_eq!(
        got[0][2], "\\\\server\\share\\repo",
        "a UNC path must round-trip intact into any finding that echoes it: {got:?}"
    );
}

// ------------- 4 · MEDIUM: is_resolved dropped a genuinely-open incident

#[test]
fn a_false_resolved_beside_the_word_true_is_still_open() {
    let log = "{\"resolved\": false, \"x\": \"true\", \"repo\": \"E:\\\\T\\\\myrepo\", \
               \"ref\": \"main\", \"old\": \"aaaabbbb\", \"new\": null}\n";
    let got = incidents::open_incidents_from(log);
    assert_eq!(
        got.len(),
        1,
        "a coincidental `true` nearby must not mark the incident resolved"
    );
}

#[test]
fn a_nested_resolved_key_does_not_decide_the_row() {
    let log = "{\"note\": {\"resolved\": true}, \"resolved\": false, \
               \"repo\": \"E:\\\\T\\\\myrepo2\", \"ref\": \"main\", \"old\": \"ccccdddd\"}\n";
    let got = incidents::open_incidents_from(log);
    assert_eq!(
        got.len(),
        1,
        "only the TOP-LEVEL resolved key decides the row"
    );
}

#[test]
fn a_genuinely_resolved_row_is_still_dropped() {
    let log = "{\"repo\": \"E:\\\\T\\\\done\", \"resolved\": true}\n";
    assert!(incidents::open_incidents_from(log).is_empty());
}

// ---- 7 · MEDIUM: the CI-skip check fired on prose written through bash

#[test]
fn documenting_the_marker_through_a_shell_redirect_is_not_denied() {
    // The audit proved my "the prose exemption still works" claim FALSE: the
    // registry check read the whole command with no notion of where the text was
    // going, so writing documentation about the marker via bash was denied.
    let marker = format!("[{} {}]", "skip", "ci");
    for cmd in [
        format!("echo \"never write the bracketed {marker} marker\" >> NOTES.md"),
        format!("printf '%s' 'the {marker} marker is forbidden' > docs/rules.md"),
    ] {
        assert_eq!(
            shell::skip_ci_marker(&cmd),
            None,
            "prose about the marker, not a commit message: {cmd}"
        );
    }
}

#[test]
fn the_whole_decision_exempts_prose_written_through_a_shell_redirect() {
    // ⭐ MY OWN TEST GAP, AND IT IS THE LESSON OF THIS CARD. The test above
    // exercises `skip_ci_marker` DIRECTLY and passed the moment the registry
    // check was scoped — while the real verdict was still `deny`, because
    // `law.rs`'s generic suppression scan denied it one layer down. A unit test
    // of the layer you just changed cannot tell you what the SYSTEM decides.
    // Caught by driving the binary, not by the suite.
    use caddis_warden::{decide_in, ToolCall, Verdict};
    let marker = format!("[{} {}]", "skip", "ci");
    let cmd = format!("echo \"never write the bracketed {marker} marker\" >> NOTES.md");
    assert_eq!(
        decide_in(
            &ToolCall::new("bash").command(&cmd),
            std::path::Path::new(".")
        ),
        Verdict::Allow,
        "documenting the rule must not be forbidden by the rule"
    );
}

#[test]
fn a_suppression_marker_redirected_into_a_code_file_is_still_denied() {
    // The exemption is granted by RECOGNISING prose, never by failing to
    // recognise a destination. A redirect into a source file stays in scope.
    use caddis_warden::{decide_in, ToolCall};
    let marker = ["# no", "sec"].concat();
    let cmd = format!("echo \"{marker}\" >> src/main.rs");
    assert!(
        decide_in(
            &ToolCall::new("bash").command(&cmd),
            std::path::Path::new(".")
        )
        .is_deny(),
        "a marker written into code is the case the rule exists for"
    );
}

#[test]
fn the_marker_in_an_actual_commit_message_is_still_denied() {
    // The dispute stands on its merits: CI honours the marker in a commit
    // message whatever the author meant.
    let marker = format!("[{} {}]", "skip", "ci");
    assert!(
        shell::skip_ci_marker(&format!("git commit -m \"wip {marker}\"")).is_some(),
        "the true positive must survive the false-positive fix"
    );
    assert!(
        shell::skip_ci_marker(&format!("git commit -am \"wip {marker}\"")).is_some(),
        "-am carries a message too"
    );
}
