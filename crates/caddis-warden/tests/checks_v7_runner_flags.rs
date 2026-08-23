//! checks_v7_runner_flags.rs — CRITICAL 1 from the fourth audit: the runner
//! registry and its grammar were TWO registries, and that shape added names
//! without flags.
//!
//! The defect is structural, not three missing entries. RUNNERS listed names
//! while `runner_value_flags` listed grammar in a separate match, so every
//! wrapper added after an audit measured it landed in the `_ => &[]` arm —
//! the wrapper recognised, its value-taking flags treated as boolean, and the
//! flag's own argument eaten as the command word. At `9005284`:
//!
//!   stdbuf -o 0 git push --force origin main   ->  ALLOW  (value separated)
//!   ionice -c 3 git push --force origin main   ->  ALLOW
//!   timeout -s KILL 30 git push --force ...    ->  ALLOW  (flag value + duration)
//!   sudo --user root git push --force ...      ->  ALLOW  (long forms absent)
//!   env --chdir /repo git push --force ...     ->  ALLOW
//!
//! Only the ATTACHED spellings (`-o0`, `-c3`) were pinned — the "fixing ONE
//! instance of a shape is not fixing the shape" lesson, committed inside the
//! commit whose own doc comment states it. The fix puts (name, flags) in ONE
//! tuple so a name cannot be added without its grammar being decided in the
//! same breath, with short AND long forms taken from each tool's real options.
//! These tests pin the SHAPE — separated values, long forms, flag-plus-operand
//! runners — for the whole registry, not the spellings one audit measured.
//!
//! ⚠ ONE DELIBERATE BEHAVIOUR CHANGE rides along, same grammar: `nice` was
//! listed as taking a leading operand, but no real nice (GNU, POSIX, BSD)
//! accepts a positional adjustment — `nice 10 git push` runs the command
//! `10`, so the old skip DENIED a line that never ran git. A false deny on a
//! DENY-class gate is the wallpaper that trains the reader to skip the
//! channel. Only `timeout` keeps a leading operand, and there it is real.

use caddis_warden::checks::git;

// ------------------- the five measured holes, each with its controls

#[test]
fn separated_flag_values_do_not_hide_the_wrapped_push() {
    // The attached forms deny; the SAME flags with the value one token away
    // read ALLOW, because the value became the command word.
    for cmd in [
        "stdbuf -o 0 git push --force origin main",
        "stdbuf -e 0 git push --force origin main",
        "ionice -c 3 git push --force origin main",
        "ionice -n 3 git push --force origin main",
    ] {
        assert!(
            git::force_push_to_protected(cmd).is_some(),
            "a separated flag value must not hide the push: {cmd}"
        );
    }
    // Controls: the attached spellings still deny, and a BOOLEAN flag of the
    // same runner (`ionice -t`, ignore) still does not swallow `git`.
    for cmd in [
        "stdbuf -o0 git push --force origin main",
        "ionice -c3 git push --force origin main",
        "ionice -t git push --force origin main",
    ] {
        assert!(
            git::force_push_to_protected(cmd).is_some(),
            "control must still deny: {cmd}"
        );
    }
}

#[test]
fn a_flag_value_and_a_duration_operand_together_do_not_hide_the_push() {
    // `timeout` carries BOTH a value-taking flag and its duration operand
    // before the command. With `-s` mis-read as boolean, KILL took the one
    // leading-operand skip and `30` became the command word.
    for cmd in [
        "timeout -s KILL 30 git push --force origin main",
        "timeout --signal KILL 30 git push --force origin main",
        "timeout -k 5 30 git push --force origin main",
    ] {
        assert!(
            git::force_push_to_protected(cmd).is_some(),
            "flag value + duration operand must not hide the push: {cmd}"
        );
    }
    // Controls: the plain duration form (pinned since v5) still denies.
    for cmd in [
        "timeout 30 git push --force origin main",
        "timeout 5m git push --force origin main",
    ] {
        assert!(
            git::force_push_to_protected(cmd).is_some(),
            "control must still deny: {cmd}"
        );
    }
}

#[test]
fn long_form_runner_flags_do_not_hide_the_wrapped_push() {
    for cmd in [
        "sudo --user root git push --force origin main",
        "sudo --group wheel git push --force origin main",
        "env --chdir /repo git push --force origin main",
        "env --unset PATH git push --force origin main",
    ] {
        assert!(
            git::force_push_to_protected(cmd).is_some(),
            "a long-form flag value must not hide the push: {cmd}"
        );
    }
    // Controls: the short forms still deny, and the boolean flag that SHARES
    // a letter with another runner's value flag (`-S`: value for env, BOOLEAN
    // for sudo) still does not swallow `git` — the table stays per-runner.
    for cmd in [
        "sudo -u root git push --force origin main",
        "env -C /repo git push --force origin main",
        "sudo -S git push --force origin main",
    ] {
        assert!(
            git::force_push_to_protected(cmd).is_some(),
            "control must still deny: {cmd}"
        );
    }
}

// ------------------- the hole was not push-specific

#[test]
fn every_git_check_sees_through_the_wrappers() {
    // All git checks read the command through the same runner skip, so the
    // audit's force-push hole also hid signing bypass, blanket staging and
    // hard reset behind the same wrappers.
    assert!(
        git::bypasses_signing("stdbuf -o 0 git commit --no-gpg-sign").is_some(),
        "signing bypass must be seen through a separated stdbuf value"
    );
    assert!(
        git::blanket_stage("ionice -c 3 git add -A").is_some(),
        "blanket stage must be seen through a separated ionice value"
    );
    assert!(
        git::is_hard_reset("sudo --user root git reset --hard"),
        "hard reset must be seen through a long-form sudo flag"
    );
    // Controls: the unwrapped commands fire (the checks are alive), and the
    // same wrapper with a harmless subcommand does not.
    assert!(git::bypasses_signing("git commit --no-gpg-sign").is_some());
    assert!(git::blanket_stage("git add -A").is_some());
    assert!(git::is_hard_reset("git reset --hard"));
    assert!(git::bypasses_signing("stdbuf -o 0 git commit -m x").is_none());
}

// ------------------- the guard must not become a forward scan

#[test]
fn the_wrapper_guard_holds_under_the_new_flag_grammar() {
    // Skipping a flag AND its value must not become "scan forward for git".
    // In each of these `git` is an ARGUMENT to docker, not the command.
    for cmd in [
        "stdbuf -o 0 docker run --rm git push --force origin main",
        "ionice -c 3 docker run --rm git push --force origin main",
        "timeout -s KILL 30 docker run --rm git push --force origin main",
        "sudo --user root docker run --rm git push --force origin main",
        "env --chdir /repo docker run --rm git push --force origin main",
    ] {
        assert_eq!(
            git::force_push_to_protected(cmd),
            None,
            "`git` here is an ARGUMENT to docker, not the command: {cmd}"
        );
    }
}

// ------------------- the folded-in false-deny fix

#[test]
fn nice_takes_no_positional_adjustment() {
    // `nice 10 git push` runs the command `10` under every real nice — the
    // adjustment is `-n`/`--adjustment` or old-style `-12`, never a bare
    // positional. The old skip denied this line; the line never runs git.
    assert_eq!(
        git::force_push_to_protected("nice 10 git push --force origin main"),
        None,
        "`nice 10 ...` does not run git; denying it is a false positive"
    );
    // Controls: every REAL nice spelling of a wrapped force-push still denies.
    for cmd in [
        "nice git push --force origin main",
        "nice -n 10 git push --force origin main",
        "nice --adjustment 10 git push --force origin main",
        "nice -12 git push --force origin main",
    ] {
        assert!(
            git::force_push_to_protected(cmd).is_some(),
            "control must still deny: {cmd}"
        );
    }
}

// ------------------- reviewer findings on CARD-WARDEN-11 itself
//
// The independent reviewer measured the commit's claim — "short AND long
// forms taken from each tool's real options" — and found it FALSE for sudo:
// `-a/--auth-type` and `-D/--chdir` (sudo 1.9+, standard since 2020) were
// missing, so the estate's most common wrapper hid a force-push behind two
// more ordinary spellings. The claim is only as good as its derivation.

#[test]
fn sudo_auth_type_and_chdir_flags_do_not_hide_the_push() {
    for cmd in [
        "sudo -a pam git push --force origin main",
        "sudo --auth-type pam git push --force origin main",
        "sudo -D /repo git push --force origin main",
        "sudo --chdir /repo git push --force origin main",
        "env -u PATH sudo -D /repo git push --force origin main",
    ] {
        assert!(
            git::force_push_to_protected(cmd).is_some(),
            "a real sudo(8) value flag must not hide the push: {cmd}"
        );
    }
    // Controls: the inline `=` form was already caught and must stay caught,
    // and `-A` (askpass) is BOOLEAN — it must not swallow `git`.
    for cmd in [
        "sudo --chdir=/repo git push --force origin main",
        "sudo -A git push --force origin main",
    ] {
        assert!(
            git::force_push_to_protected(cmd).is_some(),
            "control must still deny: {cmd}"
        );
    }
}

#[test]
fn a_short_cluster_ending_in_a_value_flag_consumes_its_value() {
    // `-Eu root` is `-E` (boolean) clustered with `-u` (value): real getopt
    // reads `root` as -u's value and runs git push. The exact-match table saw
    // `-Eu` as an unknown boolean, `root` became the command word.
    assert!(
        git::force_push_to_protected("sudo -Eu root git push --force origin main").is_some(),
        "a cluster ending in a value-taking short flag must consume its value"
    );
    // Controls — the cluster rule must NOT fire when the value is ATTACHED
    // inside the token, or `-o0`/`-k5` would eat the command word:
    for cmd in [
        "sudo -uE git push --force origin main", // -u's value is attached
        "stdbuf -o0 git push --force origin main", // attached buffer spec
        "timeout -k5 30 git push --force origin main", // attached kill-after + duration
    ] {
        assert!(
            git::force_push_to_protected(cmd).is_some(),
            "control must still deny: {cmd}"
        );
    }
}
