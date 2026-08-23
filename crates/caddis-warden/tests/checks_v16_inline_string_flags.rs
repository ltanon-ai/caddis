//! checks_v16_inline_string_flags.rs — Warden16to19Reviewer's re-measure of
//! 09eb5c6: the string-carrying flags were only collected in their SEPARATED
//! spelling. The inline (`--split-string='…'`), attached (`-S'…'`, `-c'…'`)
//! and after-operand inline forms all ran real pushes under Git Bash while
//! the warden allowed — `walk_flags` skipped `=`-tokens outright and had no
//! attached-value rule. And `env.exe` slipped the RUNNER lookup because the
//! `.exe` identity rule had landed in CARRIERS only.
//!
//! bash-truth note from the reviewer: env -S truth must be measured under a
//! Git Bash child — in the outer hybrid shell `env git --version` silently
//! no-ops (MSYS exec quirk).

use caddis_warden::checks::git;

#[test]
fn inline_and_attached_string_flag_values_are_command_lines() {
    for cmd in [
        "env --split-string='git push --force origin main'",
        "env -S'git push --force origin main'",
        "flock --command='git push --force origin main'",
        "flock -c'git push --force origin main'",
        "runuser -u root --command='git push --force origin main'",
        "runuser -u root -c'git push --force origin main'",
        "flock /tmp/l --command='git push --force origin main'",
    ] {
        assert!(
            git::force_push_to_protected(cmd).is_some(),
            "an inline or attached string value is still an executed command line: {cmd}"
        );
    }
    // Controls: the same spellings carrying harmless strings.
    for cmd in [
        "env --split-string='echo hi'",
        "env -S'echo hi'",
        "flock -c'echo hi'",
    ] {
        assert_eq!(
            git::force_push_to_protected(cmd),
            None,
            "control must stay green: {cmd}"
        );
    }
}

#[test]
fn the_exe_suffix_is_windows_spelling_for_runners_too() {
    // env.exe is the same real binary the reviewer measured PUSH-RAN as env.
    assert!(
        git::force_push_to_protected("env.exe -S 'git push --force origin main'").is_some(),
        "env.exe is env"
    );
    assert!(
        git::force_push_to_protected("sudo.exe git push --force origin main").is_some(),
        "sudo.exe is sudo"
    );
    // Controls: the suffix does not create runners, and a benign exe-wrapped
    // command stays green.
    assert_eq!(
        git::force_push_to_protected("notenv.exe -S 'git push --force origin main'"),
        None,
    );
    assert_eq!(git::force_push_to_protected("env.exe git status"), None);
}
