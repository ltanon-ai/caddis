//! scan.rs — the character-level scanners: pure functions over `&[char]`,
//! no lexer state.
//!
//! Split out of `lexer.rs` under the 280-line file law. The seam is honest:
//! everything here answers "what does THIS run of characters form" — a
//! quoted run, a comment, a separator — and touches no `Lexer` state, while
//! `lexer.rs` owns the state machine that stitches them into tokens.

/// Shell operators that separate independent commands inside one tool call.
/// A check that reads only the first command misses `cd x && git push --force`,
/// which is how the command actually tends to arrive.
pub(crate) const SEPARATORS: &[&str] = &["&&", "||", "|", ";", "\n"];

/// A quoted run as `(content, index past the closing quote)`, or `None` when
/// the quote never closes.
///
/// Scans FIRST and commits only on success, so a failed run has no side
/// effects — that is what makes the repair rule in `step` sound: on failure
/// the caller re-reads from just past the opener with nothing half-pushed.
/// Under grammar C and DOUBLE quotes, `\"` and `\\` are escape pairs exactly
/// as bash reads them. Single quotes are literal in both grammars, so a
/// Windows path inside `'...'` survives even under C.
pub(crate) fn quoted_run(
    chars: &[char],
    open: usize,
    quote: char,
    escapes: bool,
) -> Option<(String, usize)> {
    let mut content = String::new();
    let mut i = open + 1;
    while i < chars.len() {
        let c = chars[i];
        let escaped_pair = escapes
            && quote == '"'
            && c == '\\'
            && matches!(chars.get(i + 1), Some('"') | Some('\\'));
        if escaped_pair {
            content.push(chars[i + 1]);
            i += 2;
            continue;
        }
        if c == quote {
            return Some((content, i + 1));
        }
        content.push(c);
        i += 1;
    }
    None
}

/// Index of the next newline — the comment's end, NOT consumed, so the loop
/// still sees the newline as a separator. End of input when no newline.
pub(crate) fn comment_end(chars: &[char], from: usize) -> usize {
    let mut i = from;
    while i < chars.len() && chars[i] != '\n' {
        i += 1;
    }
    i
}

/// The separator beginning at `i`, longest match first so `||` never reads as
/// two `|`.
pub(crate) fn separator_at(chars: &[char], i: usize) -> Option<&'static str> {
    SEPARATORS
        .iter()
        .find(|sep| {
            let want: Vec<char> = sep.chars().collect();
            chars.len() >= i + want.len() && chars[i..i + want.len()] == want[..]
        })
        .copied()
}
