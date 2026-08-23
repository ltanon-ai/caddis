//! checks_v13_command_positions.rs — HIGH 4 from the fourth audit: shell
//! grammar hides the command, and it is NOT the runner class.
//!
//! `then`/`do`/`else`/`if`/`!`/`{` occupy the command position without BEING
//! the command; `$( … )` and backticks run a command from inside a token;
//! `bash -c '…'` and `eval "…"` run a command from inside a STRING. At the
//! audit's head every one of these hid a force-push:
//!
//!   if [ -d .git ]; then git push --force origin main; fi   -> ALLOW
//!   for r in a b; do git push --force origin main; done     -> ALLOW
//!   exec git push --force origin main                       -> ALLOW
//!   OUT=$(git push --force origin main)                     -> ALLOW
//!   bash -c 'git push --force origin main'                  -> ALLOW
//!
//! while `cd repo && git push --force origin main` denied — inconsistent, not
//! conservative. Three bounded mechanisms close the class: keyword skip at a
//! segment's start (like assignments: provable decoration), one-level
//! extraction of command substitution (single-quote aware — `'$()'` is literal
//! text in bash), and one-level re-lexing of the string argument after a
//! shell/eval carrier.

use caddis_warden::checks::git;

#[test]
fn shell_grammar_keywords_do_not_hide_the_command() {
    for cmd in [
        "if [ -d .git ]; then git push --force origin main; fi",
        "for r in a b; do git push --force origin main; done",
        "while true; do git push --force origin main; done",
        "exec git push --force origin main",
        "if git push --force origin main; then echo pushed; fi",
        "! git push --force origin main",
        "{ git push --force origin main; }",
        // The control that makes the misses inconsistent rather than
        // uniformly conservative — an ordinary `cd` chain already denied.
        "cd repo && git push --force origin main",
    ] {
        assert!(
            git::force_push_to_protected(cmd).is_some(),
            "a command-position keyword must not hide the push: {cmd}"
        );
    }
}

#[test]
fn command_substitution_runs_however_it_is_quoted() {
    for cmd in [
        // Unquoted and double-quoted substitution both execute in bash.
        "OUT=$(git push --force origin main)",
        "echo \"done: $(git push --force origin main)\"",
        "X=`git push --force origin main`",
    ] {
        assert!(
            git::force_push_to_protected(cmd).is_some(),
            "$( ) and backticks run a command from inside a token: {cmd}"
        );
    }
}

#[test]
fn single_quoted_substitution_is_literal_text() {
    // Bash does NOT execute `$( )` inside single quotes — `git commit -m
    // '$(git push --force origin main)'` commits a MESSAGE, it pushes
    // nothing. Extracting there would be a false deny on an ordinary commit.
    assert_eq!(
        git::force_push_to_protected("git commit -m '$(git push --force origin main)'"),
        None,
        "single-quoted substitution is text, not a command"
    );
}

#[test]
fn shell_carriers_run_the_string_they_carry() {
    for cmd in [
        "bash -c 'git push --force origin main'",
        "sh -c 'cd repo && git push --force origin main'",
        "eval \"git push --force origin main\"",
        "sudo bash -c 'git push --force origin main'",
    ] {
        assert!(
            git::force_push_to_protected(cmd).is_some(),
            "the string after -c/eval is a command line that really runs: {cmd}"
        );
    }
}

#[test]
fn the_new_positions_do_not_false_fire() {
    for cmd in [
        // `then` mid-segment is echo's argument, not a keyword position.
        "echo then git push --force origin main",
        // A harmless carrier string.
        "bash -c 'echo done'",
        // Harmless substitution.
        "echo \"result: $(echo hi)\"",
    ] {
        assert_eq!(
            git::force_push_to_protected(cmd),
            None,
            "no command position here — must stay green: {cmd}"
        );
    }
}
