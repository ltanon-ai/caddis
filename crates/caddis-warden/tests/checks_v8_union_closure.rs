//! checks_v8_union_closure.rs — CRITICAL 2 from the fourth audit: the two
//! fallback strategies each lose a DIFFERENT half, and a line carrying BOTH
//! conditions at once defeats the whole union.
//!
//! The degraded path runs lenient-lex ∪ naive-split, and the two tests
//! guarding it each covered ONE condition ALONE:
//!
//!   echo "a \"b\"" && git push --force origin main    -> deny (unclosed quote
//!       routes to the fallback; no quoted separator in the violation)
//!   git push --force "x && y" origin main             -> deny (quoted
//!       separator, but the line is balanced and never reaches the fallback)
//!   echo "a \"b\"" && git push --force "x && y" …     -> ALLOW (both at once)
//!   ls # don't <newline> git push --force "x && y" …  -> ALLOW (the only
//!       difference from a denying twin is an apostrophe inside a comment)
//!
//! WHY THE UNION LOSES: lenient lexing swallows everything after an unclosed
//! quote into one word; naive splitting cuts blind through the quoted `&&`
//! INSIDE the violation, mangling the refspec so neither piece proves a push
//! to a protected branch.
//!
//! The fix closes the strategy set rather than widening either member:
//! 1. A SECOND strict grammar joins the tier — escape-aware double quotes
//!    (`\"` embeds a quote, bash-faithful) — so `\"`-laden lines pair their
//!    quotes the way the shell does and never degrade at all.
//! 2. Lenient lexing REPAIRS an unclosed quote LOCALLY: the opener becomes a
//!    literal character and lexing continues, confining the damage to one
//!    word instead of swallowing the rest of the line. This is also what
//!    removes HIGH 3's measured false deny (`main"` stops matching `main`).
//! 3. naive_split is left exactly as it was — widening it is what produced
//!    HIGH 3 in the first place.
//!
//! ⚠ THE TEST MATRIX IS THE LESSON: a fallback's tests are the conditions
//! that ROUTE to it × the conditions it must still HANDLE. A per-condition
//! list feels complete and hides the intersection — which is where the hole
//! lived.

use caddis_warden::checks::cmdline::segments;
use caddis_warden::checks::git;

// ------------------- the intersection, RED at the audit's head

#[test]
fn both_fallback_conditions_at_once_still_find_the_push() {
    for cmd in [
        "echo \"a \\\"b\\\"\" && git push --force \"x && y\" origin main",
        "ls # don't\ngit push --force \"x && y\" origin main",
        "echo \"a \\\"b\\\"\" && echo don't && git push --force \"x && y\" origin main",
    ] {
        assert!(
            git::force_push_to_protected(cmd).is_some(),
            "unclosed quote + quoted separator together must not hide the push: {cmd}"
        );
    }
}

// ------------------- each condition alone, still denying

#[test]
fn each_condition_alone_still_denies() {
    for cmd in [
        // The routing condition alone: an unclosed quote before the push.
        "echo \"a \\\"b\\\"\" && git push --force origin main",
        // The quoted separator alone: balanced line, strict path.
        "git push --force \"x && y\" origin main",
        // Both SHAPES present but balanced — strict path, unchanged.
        "ls # ok\ngit push --force \"x && y\" origin main",
        // The CARD-WARDEN-9 regressions, carried by the degraded path.
        "echo it's; git push --force origin main",
        "git push --force origin main && echo don't",
        "echo \"unbalanced; git push --force origin main",
        // The Windows ruling stands: `"C:\repo\"` closes where the operator
        // meant it to, and the push is still seen (grammar A behaviour).
        "git -C \"C:\\repo\\\" push --force origin main",
    ] {
        assert!(
            git::force_push_to_protected(cmd).is_some(),
            "control must still deny: {cmd}"
        );
    }
}

#[test]
fn a_quoted_separator_still_never_splits_a_segment() {
    // The degraded-routing twin of the check the v6 test INTENDED to pin: the
    // vacuous original used balanced quotes with no apostrophe and never
    // reached the fallback at all. This one routes there via `don't`.
    let cmd = "git commit -m \"first | second\" && echo don't";
    let got = segments(cmd);
    // The union may carry duplicate segments from its members — a documented,
    // harmless property ("a check stops at its first finding"). The CONTRACT
    // is that the quoted separator survives whole in at least one strategy's
    // parse, and no verdict fires on the line.
    assert!(
        got.iter()
            .any(|seg| seg.iter().any(|t| t == "first | second")),
        "the quoted separator stays one token somewhere: {got:?}"
    );
    assert_eq!(git::force_push_to_protected(cmd), None);
}

// ------------------- false denies removed by the repair

#[test]
fn the_harmless_quoted_echo_is_no_longer_denied() {
    // HIGH 3's measured line: prints a string, lists the directory, exit 0.
    // The deny came from naive's quote-blind cut leaving a piece whose
    // swallowed tail kept `main` clean enough to match. The repair makes the
    // trailing `"` literal, so the destination token is `main"` — not a
    // provable push, and a harmless line goes back to green.
    for cmd in [
        "echo \"cmd | git push --force origin main\"; ls",
        "echo \"cmd | git push --force origin main\"; ls # don't",
    ] {
        assert_eq!(
            git::force_push_to_protected(cmd),
            None,
            "a harmless line must stay green: {cmd}"
        );
    }
}

#[test]
fn a_destination_inside_a_broken_quote_is_not_provable() {
    // `git push --force origin "main` — bash refuses the line outright, and
    // the destination sits inside the broken region. Prove-only says green,
    // the same ruling as a bare `git push --force`: the warden judges what
    // runs, and nothing runs.
    assert_eq!(
        git::force_push_to_protected("git push --force origin \"main"),
        None
    );
}

// ------------------- reviewer finding P2: the A-first trade is reachable
//
// Warden13Reviewer measured that grammar A can COMPLETE while pairing the
// quotes differently than bash — and when it does, the `&&` that bash sees as
// a separator can land INSIDE one of A's quoted runs, hiding a push bash
// would really run:
//
//   MSG="built \"x\" ok" && git push --force \"b\" main
//
// bash: MSG is assigned `built "x" ok`, then a forced push of `b` onto
// `main` RUNS. Grammar A pairs `"built \` + `"` and swallows the `&&`, so
// the line reads as one segment and the push vanishes. The fix unions BOTH
// strict parses — deduplicated, so ordinary lines (where A and C agree
// exactly) keep the same segment list they have always had.

#[test]
fn a_bash_runnable_push_that_grammar_a_pairs_away_is_still_found() {
    assert!(
        git::force_push_to_protected(
            "MSG=\"built \\\"x\\\" ok\" && git push --force \\\"b\\\" main"
        )
        .is_some(),
        "grammar C must be judged even when grammar A completes"
    );
}

#[test]
fn the_union_still_deduplicates_when_the_grammars_agree() {
    // A and C parse this identically, so the segment list must be exactly
    // what it always was — the union must not double it.
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
                "main".to_string(),
            ],
        ],
        "identical parses must not be duplicated: {got:?}"
    );
}
