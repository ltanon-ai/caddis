//! checks_v6_regress.rs — FOUR MEASURED REGRESSIONS, and the severity lesson.
//!
//! ⛔ THE ASYMMETRY IS THE FINDING, not the bug. CARD-WARDEN-7 fixed a case I
//! rated CRITICAL in my own words — *"a contraction in a chained echo. Not
//! adversarial — a typo."* CARD-WARDEN-9 then DISCLOSED the mirror image as a
//! tolerable known gap and argued the shell would refuse the line:
//!
//!   git push --force origin main && echo don't   ->  DENY   (rated CRITICAL)
//!   echo don't; git push --force origin main     ->  ALLOW  (called a known gap)
//!
//! Same defect. Clauses swapped. Opposite grading — and the ONLY difference was
//! that one arrived as an auditor's finding and the other was mine. A defect I
//! found myself got the lenient reading, and the more natural writing order (say
//! something, then act) is the one that defeated the gate.
//!
//! The disclosure was honest. The SEVERITY was not, and an honest note at the
//! wrong severity leaves the hole open just as effectively as hiding it.

use caddis_warden::checks::git;

/// Every one of these was DENY at `6d4a861` and ALLOW at `e29eb36`, measured by
/// an independent audit driving the real binary over the real wire protocol.
#[test]
fn a_contraction_before_the_force_push_does_not_hide_it() {
    for cmd in [
        "echo it's; git push --force origin main",
        "echo don't; git push --force origin main",
        "echo can't && git push --force origin main",
        "echo \"unbalanced; git push --force origin main",
    ] {
        assert!(
            git::force_push_to_protected(cmd).is_some(),
            "REGRESSION: an unclosed quote BEFORE the push must not hide it: {cmd}"
        );
    }
}

#[test]
fn the_original_clause_order_still_denies() {
    // The case WARDEN-7 fixed must not be traded away by fixing its mirror.
    for cmd in [
        "git push --force origin main && echo don't",
        "git push --force \"x && y\" origin main && echo don't",
    ] {
        assert!(
            git::force_push_to_protected(cmd).is_some(),
            "the previously-fixed order must keep working: {cmd}"
        );
    }
}

#[test]
fn a_well_formed_quoted_separator_is_still_not_split() {
    // The property lenient lexing was introduced FOR. Running both strategies
    // must not resurrect the naive splitter's own defect.
    use caddis_warden::checks::cmdline::segments;
    let got = segments("git commit -m \"first | second\"");
    assert_eq!(got.len(), 1, "quoted pipe must not split: {got:?}");
}

// ------------------------------ wrapper commands still missing from the list

#[test]
fn detaching_wrappers_do_not_hide_a_force_push() {
    // `setsid` is the standard way to detach a process, which makes it the one
    // that matters most here.
    for cmd in [
        "setsid git push --force origin main",
        "stdbuf -o0 git push --force origin main",
        "ionice -c3 git push --force origin main",
    ] {
        assert!(
            git::force_push_to_protected(cmd).is_some(),
            "wrapper not recognised: {cmd}"
        );
    }
}

#[test]
fn the_wrapper_guard_still_holds_for_all_of_them() {
    // Widening the list must not turn `git` as an ARGUMENT into a git command.
    for cmd in [
        "setsid docker run --rm git push --force origin main",
        "sudo docker run --rm git push --force origin main",
    ] {
        assert_eq!(
            git::force_push_to_protected(cmd),
            None,
            "`git` here is an argument, not the command: {cmd}"
        );
    }
}

/// ⚠ `xargs` is deliberately NOT treated as a wrapper, and this test pins the
/// decision so it is re-argued rather than re-discovered.
///
/// The audit raised it as a weaker claim and was right to hedge: `xargs` takes
/// its arguments from STDIN and `-I{}` rewrites them, so the token sequence on
/// the line is not the command that runs. Calling it a runner would mean judging
/// a command whose real arguments this checker cannot see — inventing a verdict
/// rather than measuring one.
#[test]
fn xargs_is_not_treated_as_a_wrapper_by_design() {
    assert_eq!(
        git::force_push_to_protected("xargs -I{} git push --force origin main"),
        None,
        "if this ever fires, the reasoning in the doc comment must be revisited first"
    );
}
