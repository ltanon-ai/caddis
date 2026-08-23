//! checks_v9_comments.rs — reviewer finding P3: the repair made the warden's
//! comment-blindness visible as a false deny.
//!
//! `git push # don't --force origin main` runs a bare `git push` in bash —
//! everything after `#` at a word start is a comment. The warden never knew
//! comments; before CARD-WARDEN-13 the apostrophe in the comment happened to
//! swallow the rest of the line (accidental correctness), and the repair
//! un-swallowed it, so the commented `--force origin main` started reading as
//! push arguments and a harmless line denied. The fix is to know what bash
//! knows: `#` at a word start comments to end of line.

use caddis_warden::checks::git;

#[test]
fn a_comment_does_not_feed_tokens_to_the_command() {
    assert_eq!(
        git::force_push_to_protected("git push # don't --force origin main"),
        None,
        "everything after `#` is a comment; the command is a bare `git push`"
    );
}

#[test]
fn a_command_before_a_comment_still_fires() {
    // bash runs the force-push; a trailing comment must not hide it.
    assert!(
        git::force_push_to_protected("git push --force origin main # don't").is_some(),
        "the command before the comment is real and must still deny"
    );
    // A comment ending one line does not protect the next.
    assert!(
        git::force_push_to_protected("echo hi # ok\ngit push --force origin main").is_some(),
        "the next line after a comment still runs"
    );
}

#[test]
fn a_hash_mid_word_is_not_a_comment() {
    // bash: `a#b` is one word — the `#` is literal because it does not BEGIN
    // the word; and a hash inside quotes is content, never a comment.
    assert_eq!(
        git::force_push_to_protected("echo a#b"),
        None,
        "mid-word `#` is literal; there is no push here at all"
    );
    assert!(
        git::force_push_to_protected("git push --force 'x#y' origin main").is_some(),
        "a quoted hash is content; the force-push to main must still fire"
    );
}
