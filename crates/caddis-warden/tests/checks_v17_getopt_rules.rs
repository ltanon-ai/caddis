//! checks_v17_getopt_rules.rs — Warden16to19Reviewer's closing observations:
//! the runner grammar implements getopt's FLAGS but not getopt's own RULES.
//!
//! 1. getopt_long accepts UNAMBIGUOUS ABBREVIATIONS: `flock /tmp/l --comma
//!    'git push …'` runs in real util-linux (--comma resolves to --command),
//!    while the exact-match registry never saw it. The abbreviation resolves
//!    when exactly one listed long option begins with the prefix; an
//!    ambiguous prefix (`--co` for flock: --command vs --conflict-exit-code)
//!    makes getopt ERROR, so nothing runs and the line must stay green.
//! 2. The GLUED LONG form `--command'x'` is NOT getopt: attached values are a
//!    SHORT-option rule (`-S'x'`); a long option carries its value only after
//!    `=`. Real getopt rejects `--command'x'` as unknown-option — the line
//!    errors, and over-collecting it denied a line that never runs.

use caddis_warden::checks::git;

#[test]
fn unambiguous_long_abbreviations_resolve() {
    for cmd in [
        // --comma -> --command (flock's only long flag starting "comma")
        "flock /tmp/l --comma 'git push --force origin main'",
        "flock --comm='git push --force origin main'",
        // --split -> --split-string (env's only long starting "split")
        "env --split='git push --force origin main'",
        // --use -> --user, separated value form
        "sudo --use root git push --force origin main",
    ] {
        assert!(
            git::force_push_to_protected(cmd).is_some(),
            "an unambiguous abbreviation is the flag (getopt_long): {cmd}"
        );
    }
    // Ambiguous prefix: getopt ERRORS ("option '--co' is ambiguous"), the
    // line never runs, and the warden must not invent a reading.
    assert_eq!(
        git::force_push_to_protected("flock --co 'git push --force origin main'"),
        None,
        "ambiguous abbreviation: getopt refuses the line, nothing runs"
    );
}

#[test]
fn glued_long_values_are_not_a_getopt_spelling() {
    // `-S'x'` is getopt (short, attached); `--command'x'` is NOT — a long
    // option carries its value only after `=`. Real getopt rejects the token
    // as unknown-option, the line errors, and the string is never executed.
    assert_eq!(
        git::force_push_to_protected("flock --command'git push --force origin main'"),
        None,
        "glued long value: getopt rejects it; nothing runs"
    );
    // Controls: the legal spellings of the same flags still deny.
    assert!(
        git::force_push_to_protected("flock --command='git push --force origin main'").is_some(),
        "inline `=` remains the long form"
    );
    assert!(
        git::force_push_to_protected("flock -c'git push --force origin main'").is_some(),
        "attached remains the SHORT form"
    );
    // `=` in ordinary arguments still changes nothing.
    assert_eq!(git::force_push_to_protected("echo x=y"), None);
    assert_eq!(git::force_push_to_protected("git commit -m \"a=b\""), None);
}

// ------------------- the boolean-cluster + attached-string seam
//
// Warden16to19Reviewer's last observation, corrected by measurement against
// real getopt (the tool, measured in this repo): the seam is real but the
// example was not. `flock -Ec'…'` gives -E — a VALUE flag — the rest of the
// cluster (`getopt -o 'E:c:' -- -Ec'test value'` -> `-E 'ctest value'`), so
// no -c runs and the line does nothing: ALLOW is correct, pinned below. The
// runnable spelling is a genuinely BOOLEAN letter before the string flag:
// `env -iS'git push …'` (`getopt -o 'iS:…' -- -iS'test value'` ->
// `-i -S 'test value'`) — env executes the string.

#[test]
fn a_boolean_cluster_before_an_attached_string_flag_still_runs_it() {
    assert!(
        git::force_push_to_protected("env -iS'git push --force origin main'").is_some(),
        "`-i` is boolean; `-S` takes the attached value; env runs it"
    );
    // Control: the same cluster carrying a harmless string.
    assert_eq!(git::force_push_to_protected("env -iS'echo hi'"), None);
}

#[test]
fn a_value_flag_letter_owns_the_rest_of_the_cluster() {
    // Measured: getopt gives the FIRST value-taking letter the remainder.
    // `flock -Ec'…'` -> -E takes "c'…'" — no -c exists, nothing runs.
    assert_eq!(
        git::force_push_to_protected("flock -Ec'git push --force origin main'"),
        None,
        "-E owns the rest; the reviewer's example is a correct allow"
    );
    // `env -iuS'…'` -> -u takes "S'…'" — no -S command, nothing runs.
    assert_eq!(
        git::force_push_to_protected("env -iuS'git push --force origin main'"),
        None,
        "-u owns the rest; env prints its environment and runs nothing"
    );
}

// ------------------- reviewer's closing truth nuances (measured)
//
// 1. `env -iS'git push …'` DENIES, and the deny is right, but on THIS host
//    the example does not complete a push: `-i` strips the environment, so
//    git cannot resolve its helpers. The reviewer measured the string class
//    still EXECUTES (absolute-path marker through -i) — the class is real,
//    only the example's push is host-incomplete.
// 2. `env -0S'git push …'` denies as a spelling-class conservatism: real env
//    itself REFUSES `-0` with a command ("cannot specify --null (-0) with
//    command" — measured). Denying a line env rejects is the accepted
//    deny-direction cost of judging by spelling; pinned so it is deliberate.

#[test]
fn the_closing_nuances_are_pinned() {
    assert!(
        git::force_push_to_protected("env -0S'git push --force origin main'").is_some(),
        "spelling-class conservatism: env itself refuses -0 with a command"
    );
    // The reviewer's Linux-true spellings of the cluster class.
    assert!(
        git::force_push_to_protected("flock /tmp/l -nc'git push --force origin main'").is_some(),
        "-n is boolean; -c takes the attached value"
    );
    assert!(
        git::force_push_to_protected("flock /tmp/l -n -c 'git push --force origin main'").is_some(),
        "separated boolean + string flag"
    );
}
