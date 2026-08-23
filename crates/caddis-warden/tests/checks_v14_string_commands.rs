//! checks_v14_string_commands.rs — Warden16to19Reviewer findings: six
//! measured spellings that genuinely force-push while the warden allows.
//!
//! All six are the same class — a command string executed somewhere the
//! segment walk never looked:
//!
//!   eval git push --force origin main            — eval JOINS word args
//!   flock -c 'git push …' / runuser -c '…'       — a runner flag whose VALUE
//!   env -S 'git push …'                            is an executed string
//!   /bin/bash -c 'git push …'                    — path-prefixed carrier
//!   bash -c $'git push …'                        — the $ glues onto the token
//!   bash -c 'echo $(git push …)'                 — carrier × substitution
//!   sh -c git\ push\ --force\ origin\ main       — escaped spaces assemble
//!                                                  the string bash sees
//!
//! Every line was measured PUSH-RAN against a real bare remote by the
//! reviewer, and measured ALLOW by the warden at fceb94e.

use caddis_warden::checks::git;

#[test]
fn eval_joins_its_words_into_one_command_line() {
    for cmd in [
        "eval git push --force origin main",
        "eval git push origin +main",
    ] {
        assert!(
            git::force_push_to_protected(cmd).is_some(),
            "eval's argument list IS the command line: {cmd}"
        );
    }
    // Control: a harmless eval string stays green.
    assert_eq!(git::force_push_to_protected("eval \"echo done\""), None);
}

#[test]
fn string_valued_runner_flags_are_executed_command_lines() {
    for cmd in [
        "flock -c 'git push --force origin main'",
        "flock /tmp/l -c 'git push --force origin main'",
        "runuser -u root -c 'git push --force origin main'",
        "env -S 'git push --force origin main'",
        "sudo env -S 'git push --force origin main'",
    ] {
        assert!(
            git::force_push_to_protected(cmd).is_some(),
            "the value of this flag is a command string a shell executes: {cmd}"
        );
    }
    // Controls: the same flags carrying harmless strings, and the ordinary
    // (non-string) spellings that already denied.
    assert_eq!(git::force_push_to_protected("env -S 'echo hi'"), None);
    assert!(
        git::force_push_to_protected("flock /tmp/l git push --force origin main").is_some(),
        "control: the ordinary flock spelling still denies"
    );
}

#[test]
fn absolute_path_prefixes_do_not_disguise_a_carrier() {
    for cmd in [
        "/bin/bash -c 'git push --force origin main'",
        "/bin/sh -c 'git push --force origin main'",
        "C:/Git/usr/bin/bash.exe -c 'git push --force origin main'",
    ] {
        assert!(
            git::force_push_to_protected(cmd).is_some(),
            "a path-prefixed shell is the same shell: {cmd}"
        );
    }
    // Control: a RELATIVE path is not an identity claim — ./bash could be
    // anything, so it stays unrecognised rather than guessed.
    assert_eq!(
        git::force_push_to_protected("./bash -c 'git push --force origin main'"),
        None,
        "relative path: identity unknown, not judged as a carrier"
    );
}

#[test]
fn dollar_quoting_does_not_poison_the_carried_string() {
    // bash: $'…' is ANSI-C quoting — ONE string argument, no `$` in it. The
    // warden used to glue the `$` onto the token, so the re-lexed line began
    // with the word `$git` and the push vanished.
    assert!(
        git::force_push_to_protected("bash -c $'git push --force origin main'").is_some(),
        "$'…' is a quoted string; the carrier string must re-lex clean"
    );
    // Control: dollar-quoting elsewhere changes nothing.
    assert_eq!(git::force_push_to_protected("echo $'a b'"), None);
}

#[test]
fn a_carried_string_composes_with_substitution() {
    // The inner shell performs the substitution INSIDE the carried string:
    // one carrier level plus one substitution level, the natural composition
    // of the two spellings CARD-WARDEN-19 named.
    assert!(
        git::force_push_to_protected("bash -c 'echo $(git push --force origin main)'").is_some(),
        "substitution inside a carried string runs in the inner shell"
    );
    assert_eq!(
        git::force_push_to_protected("bash -c 'echo $(echo hi)'"),
        None
    );
}

#[test]
fn escaped_spaces_assemble_the_string_bash_sees() {
    // Every space escaped: bash hands sh ONE argument "git push --force
    // origin main". The warden saw five space-free tokens and no string.
    assert!(
        git::force_push_to_protected("sh -c git\\ push\\ --force\\ origin\\ main").is_some(),
        "the whole escaped run is one command line to the inner shell"
    );
    // Control: only the FIRST gap escaped — bash's argument is "git push",
    // the rest become positional args, and a plain push is not this check's
    // to deny. The join rule must fire only for a full escaped run.
    assert_eq!(
        git::force_push_to_protected("sh -c git\\ push --force origin main"),
        None,
        "partial escape: only 'git push' is the script; no force push runs"
    );
}
