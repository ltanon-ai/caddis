//! checks_v12_escaped_separators.rs — LOW 6 from the fourth audit: a
//! backslashed separator is a WORD CHARACTER, not a split.
//!
//! `echo hello\; git push --force origin main` runs ONE echo in bash — `\;`
//! hands echo a literal semicolon and nothing is pushed. The lexer's v3
//! ruling let a backslash escape ONLY quotes, so the `;` still split segments
//! and the warden read a force-push that never runs.
//!
//! v4 extends the ruling, not overturns it: a backslash escapes quotes AND
//! the separator characters (`;`, `|`, `&`). Whitespace and backslash stay
//! literal — `ls a\ b` is still two tokens and `C:\repo\` is still one word,
//! exactly as the Windows-path ruling pinned.

use caddis_warden::checks::cmdline::segments;
use caddis_warden::checks::git;

#[test]
fn an_escaped_separator_is_a_word_character() {
    // RED at the audit's head: this denied as a force-push.
    assert_eq!(
        git::force_push_to_protected("echo hello\\; git push --force origin main"),
        None,
        "`\\;` is literal; the whole line is one echo and nothing is pushed"
    );
    assert!(
        git::force_push_to_protected("echo a\\|b; git push --force origin main").is_some(),
        "the LATER unescaped separator still splits and the push still denies"
    );
}

#[test]
fn the_unescaped_separator_still_splits() {
    // The mirror control: identical line without the backslash must deny.
    assert!(
        git::force_push_to_protected("echo hello; git push --force origin main").is_some(),
        "an unescaped `;` still separates a real push"
    );
}

#[test]
fn the_windows_ruling_survives_v4() {
    // Whitespace is still NOT escapable (`ls a\ b` = two tokens, pinned since
    // v3), and backslashes before letters stay literal, so paths lex whole.
    let got = segments("ls a\\ b");
    assert_eq!(
        got[0],
        vec!["ls".to_string(), "a\\".to_string(), "b".to_string()]
    );
    assert!(
        git::force_push_to_protected("git -C C:\\x\\y\\ push --force origin main").is_some(),
        "unquoted Windows paths still lex whole and still deny"
    );
}
