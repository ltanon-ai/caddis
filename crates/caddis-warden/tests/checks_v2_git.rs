//! checks_v2.rs — every check the shared law corpus names, driven both ways.
//!
//! ⭐ EACH CHECK GETS A RED CASE **AND** A GREEN CASE, and the green case is not
//! padding. A check that fires on everything is indistinguishable from a check
//! that fires on the right thing when you only ever test the red — and it is the
//! more expensive failure, because it trains the reader to skip the channel and
//! takes every OTHER finding down with it. `a GREEN check emits NOTHING` is a
//! behaviour, so it is asserted like one.
//!
//! ⚠ THE LEXER IS VERIFIED THROUGH ITSELF, never by how the fixture LOOKS. A
//! fixture's textual layout does not determine its parsed structure: a string
//! that reads to a human as two piped commands may be one segment, and then the
//! test that claims to exercise the pipeline path silently exercises the
//! ordinary one. So the tokeniser's actual output is asserted first, and the
//! checks that depend on it are written against what it really produces.

use caddis_warden::checks::cmdline::{segments, segments_detailed};
use caddis_warden::checks::git;

// ---------------------------------------------------------------- the lexer

#[test]
fn the_lexer_splits_a_compound_command_into_its_real_commands() {
    let got = segments("cd x && git push --force origin main");
    assert_eq!(
        got,
        vec![
            vec!["cd".to_string(), "x".to_string()],
            vec![
                "git".to_string(),
                "push".to_string(),
                "--force".to_string(),
                "origin".to_string(),
                "main".to_string()
            ],
        ],
        "a check reading only the first command would miss the push entirely"
    );
}

#[test]
fn a_quoted_separator_does_not_split_the_command() {
    // The estate's Python splits on separators BEFORE lexing, so this line
    // breaks its own segment and gets dropped. Asserting the improvement here
    // rather than only claiming it in a comment.
    let got = segments("git commit -m \"first | second\"");
    assert_eq!(
        got.len(),
        1,
        "the pipe is inside quotes: one command, got {got:?}"
    );
    assert_eq!(got[0][3], "first | second");
}

#[test]
fn an_unterminated_quote_degrades_instead_of_judging_nothing() {
    // ⛔ THIS ASSERTION WAS INVERTED ON PURPOSE, and it is the one place in the
    // suite where that is the right move. It used to demand `is_empty()` —
    // "unlexable input is judged as nothing" — which reads as caution and was
    // the exact defect two audits found: a stray apostrophe made a force-push
    // to master invisible. Judging nothing is not the safe answer; it is the
    // silent one. The contract now is DEGRADE: lex leniently, keep every
    // well-formed part, never panic.
    let got = segments("git commit -m \"never closed");
    assert!(
        !got.is_empty(),
        "an unterminated quote must not blank the whole line"
    );
    assert_eq!(
        got[0][0], "git",
        "the well-formed head must survive: {got:?}"
    );
}

#[test]
fn the_segmenter_records_which_operator_introduced_each_command() {
    let got = segments_detailed("git show HEAD:a.rs | wc -l");
    assert_eq!(got.len(), 2);
    assert_eq!(got[0].sep_before, None);
    assert_eq!(
        got[1].sep_before.as_deref(),
        Some("|"),
        "the pipe must be distinguishable from `&&`, or the counter check reads \
         `a && wc` as a pipeline"
    );
}

// ------------------------------------------------------- git.hooks.skipped

#[test]
fn a_commit_that_skips_the_hooks_is_found() {
    // Assembled at runtime: this file may not spell the flag (see law.rs).
    let flag = ["--no", "-verify"].concat();
    let f = git::skips_hooks(&format!("git commit {flag} -m x"))
        .expect("a hook-skipping commit must produce a finding");
    assert!(f.contains(&flag), "the finding names the flag: {f}");
}

#[test]
fn an_ordinary_commit_is_silent() {
    assert_eq!(git::skips_hooks("git commit -m \"a real message\""), None);
}

#[test]
fn a_dry_run_push_is_not_mistaken_for_a_hook_skip() {
    // `-n` on push means --dry-run and is harmless; on commit it is the skip
    // flag. Treating them alike would deny a dry run — a false positive on a
    // DENY-class check, which is how a blocking mechanism gets switched off.
    assert_eq!(git::skips_hooks("git push -n origin feature"), None);
    assert!(git::skips_hooks("git commit -n -m x").is_some());
}

// ---------------------------------------------------- git.signing.bypassed

#[test]
fn dropping_the_signature_is_found_and_named_as_signing() {
    let f = git::bypasses_signing("git commit --no-gpg-sign -m x").expect("finding");
    assert!(
        f.contains("signing"),
        "the estate learned this the expensive way: a signing bypass reported as \
         a hook bypass is a correct verdict with a false reason — {f}"
    );
    assert!(!f.contains("hooks"), "and it must not blame the hooks: {f}");
}

#[test]
fn a_signed_commit_is_silent() {
    assert_eq!(git::bypasses_signing("git commit -S -m x"), None);
}

// ----------------------------------------------- git.push.force-to-protected

#[test]
fn a_force_push_to_a_protected_branch_is_found() {
    let f = git::force_push_to_protected("git push --force origin main").expect("finding");
    assert!(f.contains("main"), "the finding names the branch: {f}");
}

#[test]
fn a_leading_plus_refspec_is_a_force_with_no_flag_at_all() {
    // The case the previous ad-hoc substring rule could not see.
    let f = git::force_push_to_protected("git push origin +master").expect("finding");
    assert!(f.contains("refspec"), "{f}");
}

#[test]
fn a_bundled_short_flag_still_counts_as_force() {
    assert!(git::force_push_to_protected("git push -uf origin main").is_some());
}

#[test]
fn force_pushing_a_feature_branch_is_allowed_even_when_its_name_contains_main() {
    // The old substring test denied this. Force-pushing your OWN branch is
    // routine and legitimate, and denying it is exactly how a gate gets
    // switched off.
    assert_eq!(
        git::force_push_to_protected("git push --force origin feature-main-thing"),
        None
    );
}

#[test]
fn an_unprovable_force_push_is_left_green_as_a_named_gap() {
    // A bare `--force` targets whatever is checked out, which this check cannot
    // see. Guessing would make a deny-class check fire on the routine case.
    assert_eq!(git::force_push_to_protected("git push --force"), None);
}

// ------------------------------------ git.stage.blanket-in-shared-worktree

#[test]
fn blanket_staging_is_found() {
    assert!(git::blanket_stage("git add -A").is_some());
    assert!(git::blanket_stage("git add .").is_some());
    assert!(git::blanket_stage("git commit -am wip").is_some());
}

#[test]
fn staging_named_paths_is_silent() {
    assert_eq!(git::blanket_stage("git add src/lib.rs src/law.rs"), None);
}
