//! checks_v3_audit.rs — the defects a CLEAN AGENT found in CARD-WARDEN-6.
//!
//! Every test here failed when it was written. They exist because an independent
//! audit re-executed the landed binary and found what the builder could not see
//! from the inside — including one finding that REFUTED a claim the commit
//! message made in its own defence.
//!
//! ⭐ THE ONE THAT MATTERS MOST (finding 1). CARD-WARDEN-6 argued that lexing
//! the whole line BEFORE splitting on separators was "a divergence in the safe
//! direction — it judges commands the other would skip". That was FALSE, and
//! measurably so: because `tokenize` lexes the entire line in one pass, a single
//! unbalanced quote ANYWHERE returns `None`, `segments()` yields an empty Vec,
//! and NOTHING is judged — not even a well-formed dangerous segment earlier in
//! the line. The estate's Python splits first and lexes each piece, so it keeps
//! the clean segment and drops only the malformed one. On that input the Rust
//! was STRICTLY LESS SAFE than the thing it claimed to improve on.
//!
//! The lesson is not "the lexer had a bug". It is that a claim of superiority
//! written into a commit message is still just a claim, and this one was refuted
//! by the first person to test it adversarially.

use caddis_warden::checks::cmdline::segments;
use caddis_warden::checks::{git, incidents};

// ---------------------------------------------- finding 1: quote-blindness

#[test]
fn an_unbalanced_quote_later_in_the_line_does_not_blind_the_dangerous_part_before_it() {
    // A contraction in a chained echo. Not adversarial — the kind of thing that
    // gets typed by accident.
    let cmd = "git push --force origin main && echo don't-skip-review";
    assert!(
        git::force_push_to_protected(cmd).is_some(),
        "a stray apostrophe must not disable a deny-class check for the \
         well-formed force-push that precedes it"
    );
}

#[test]
fn the_unlexable_segment_is_the_only_thing_dropped() {
    let got = segments("git push --force origin main && echo don't-close");
    assert!(
        !got.is_empty(),
        "degrade to per-segment lexing, never to judging nothing"
    );
    assert_eq!(
        got[0],
        vec!["git", "push", "--force", "origin", "main"],
        "the clean segment must survive intact, got {got:?}"
    );
}

#[test]
fn a_quoted_separator_still_does_not_split_when_the_line_lexes_cleanly() {
    // The good property of whole-line lexing is KEPT: this must not regress
    // into the Python behaviour now that a fallback exists.
    let got = segments("git commit -m \"first | second\"");
    assert_eq!(got.len(), 1, "quoted pipe must not split, got {got:?}");
    assert_eq!(got[0][3], "first | second");
}

// --------------------------------------- finding 2: sudo/env with any flag

#[test]
fn a_runner_prefix_with_flags_does_not_hide_the_git_subcommand() {
    for cmd in [
        "sudo -u root git push --force origin main",
        "env -i git push --force origin main",
        "env -u PATH FOO=bar git push --force origin main",
    ] {
        assert!(
            git::force_push_to_protected(cmd).is_some(),
            "a flagged runner prefix must not defeat the check: {cmd}"
        );
    }
}

#[test]
fn the_unflagged_runner_prefix_still_works() {
    assert!(git::force_push_to_protected("sudo git push --force origin main").is_some());
    assert!(git::force_push_to_protected("sudo env git push --force origin main").is_some());
}

#[test]
fn a_two_token_git_global_not_in_the_table_does_not_hide_the_subcommand() {
    assert!(
        git::force_push_to_protected("git --super-prefix /x push --force origin main").is_some(),
        "--super-prefix is a real two-token git global"
    );
}

#[test]
fn a_runner_prefix_does_not_make_an_unrelated_command_read_as_git() {
    // The skip must not become "scan forward until you find the word git".
    assert_eq!(
        git::force_push_to_protected("sudo docker run --rm git push --force origin main"),
        None,
        "`git` here is an ARGUMENT to docker, not the command being run"
    );
}

// --------------------------- finding 3: the rewrite latch never read the cwd

#[test]
fn being_inside_the_incident_repo_is_caught_even_when_the_command_does_not_name_it() {
    let inc = vec![incidents::Incident {
        repo: "E:\\Tool\\_worktrees\\bee-build-laisvas".to_string(),
        reference: "refs/remotes/origin/master".to_string(),
        old: "781cc406aaaa".to_string(),
        new: String::new(),
    }];
    let cwd = std::path::Path::new("E:/Tool/_worktrees/bee-build-laisvas");
    assert!(
        incidents::push_into_rewritten_repo_in("git push origin master", cwd, &inc).is_some(),
        "the doc comment claims `simply by already being there` is covered — it must be"
    );
}

#[test]
fn an_unrelated_cwd_is_not_caught() {
    let inc = vec![incidents::Incident {
        repo: "E:\\Tool\\_worktrees\\bee-build-laisvas".to_string(),
        reference: "refs/remotes/origin/master".to_string(),
        old: "781cc406aaaa".to_string(),
        new: String::new(),
    }];
    let cwd = std::path::Path::new("C:/Users/ashpac/scratch/caddis-workshop");
    assert_eq!(
        incidents::push_into_rewritten_repo_in("git push origin master", cwd, &inc),
        None
    );
}

// ------------------------- finding 4: a null field hallucinated a neighbour

#[test]
fn a_null_json_field_does_not_borrow_the_next_keys_name_as_its_value() {
    // THIS SHAPE IS ON DISK TODAY: both currently-unresolved incidents carry
    // `"new": null, "verdict": "vanished"`. The old scanner searched for the
    // next quote after the colon and found the NEXT KEY, so the finding read
    // "... is not an ancestor of verdict".
    let log = concat!(
        "{\"repo\": \"E:\\\\Tool\\\\repo-a\", \"ref\": \"refs/heads/main\", ",
        "\"old\": \"655f64d2aaaa\", \"new\": null, \"verdict\": \"vanished\"}\n"
    );
    let got = incidents::open_incidents_from(log);
    assert_eq!(got.len(), 1);
    assert_eq!(
        got[0].new, "",
        "a null must read as absent, not as the next key's name — got {:?}",
        got[0].new
    );
    assert_eq!(got[0].old, "655f64d2aaaa", "the real fields still parse");

    // The latch must be POINTED at the repo, or correctly staying silent would
    // masquerade as the bug under test. Standing inside it is the case finding 3
    // added, so use that.
    let finding = incidents::push_into_rewritten_repo_in(
        "git push origin main",
        std::path::Path::new("E:/Tool/repo-a"),
        &got,
    )
    .expect("an open incident in the cwd must produce a finding");
    assert!(
        !finding.contains("verdict"),
        "the finding must not fabricate a SHA from a neighbouring key: {finding}"
    );
}

// ------------------- finding 6: backslashes stripped from unquoted Windows paths

#[test]
fn an_unquoted_windows_path_survives_lexing() {
    let got = segments("git -C C:\\Users\\ashpac\\repo push --force origin main");
    assert!(!got.is_empty());
    assert_eq!(
        got[0][2], "C:\\Users\\ashpac\\repo",
        "a Windows path must not be mangled into C:Usersashpacrepo — any finding \
         that echoes the command back would misquote what was typed"
    );
}

#[test]
fn a_posix_escaped_space_is_deliberately_no_longer_honoured() {
    // ⚠ THIS ASSERTION WAS CHANGED BY ITS OWN AUTHOR, AND THE REASON MATTERS.
    // I wrote the previous version (`ls a\ b` -> ["ls", "a b"]) one card ago.
    // A second audit then proved the rule behind it caused a MISSED VIOLATION:
    // treating backslash as a general escape made `\"` swallow a closing quote,
    // so `git -C "C:\repo\" push --force origin main` vanished entirely.
    //
    // On this estate Windows paths are constant and POSIX escapes are rare, so
    // backslash now escapes ONLY a quote. `ls a\ b` becomes two tokens. That is
    // a real regression in POSIX fidelity, accepted deliberately, and pinned
    // here so the next author sees a DECISION rather than an accident — and so
    // that flipping it back requires arguing with this comment first.
    let got = segments("ls a\\ b");
    assert_eq!(got[0], vec!["ls", "a\\", "b"], "got {got:?}");
}

#[test]
fn a_backslash_can_still_escape_a_quote_outside_quotes() {
    // The one escape that survives, and the reason `is_escapable` is not empty.
    let got = segments("echo it\\'s");
    assert_eq!(got[0], vec!["echo", "it's"], "got {got:?}");
}
