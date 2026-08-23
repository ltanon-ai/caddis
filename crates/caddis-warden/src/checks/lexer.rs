//! lexer.rs — turning one command line into tokens, quote-aware.
//!
//! Split out of `cmdline.rs` under the repo's 280-line file law. The division is
//! not arbitrary: everything here answers "what are the WORDS of this command",
//! and everything left in `cmdline.rs` answers "what does this command MEAN"
//! (is it a git push, where is its refspec). Mixing the two is what makes a
//! lexer accumulate git knowledge it has no business holding. The character
//! scanners themselves live in `scan.rs`, split out under the same law.
//!
//! TWO GRAMMARS, because the quoting question has two defensible answers and
//! the estate needs both (audit 4, CRITICAL 2):
//!
//! - Grammar A (literal): nothing escapes inside quotes, so `"C:\repo\"`
//!   closes where a Windows path means it to.
//! - Grammar C (escape-aware): inside DOUBLE quotes `\"` embeds a quote and
//!   `\\` a backslash, the way bash reads it, so `echo "a \"b\""` pairs as
//!   the shell pairs it.
//!
//! Neither dominates; the segmenter tries both and judges the union.
//!
//! THE LENIENT TIER REPAIRS rather than swallows: when a quote never closes,
//! the opener becomes a literal character and lexing CONTINUES — the damage
//! is confined to the one word instead of eating the rest of the line. The
//! old swallow-to-end behaviour is what let one apostrophe hide every command
//! after it: CARD-WARDEN-9's regression, re-found by audit 4 as the half of
//! the cross-product the union still lost.

use super::scan::{comment_end, quoted_run, separator_at};

/// Characters a leading backslash may escape OUTSIDE quotes.
///
/// ⛔ THIS RULE HAS NOW BEEN WRONG THREE TIMES, TWICE IN OPPOSITE DIRECTIONS.
/// Recorded in full because the trade is not obvious:
///
/// - v1 escaped EVERY character, so `C:\Users\ashpac\repo` lexed to
///   `C:Usersashpacrepo` (audit 1, finding 6 — cosmetic, no wrong verdict).
/// - v2 escaped whitespace, quotes and backslash "like POSIX". That made `\"`
///   escape a CLOSING quote, so the utterly ordinary
///   `git -C "C:\Users\ashpac\repo\" push --force origin main` became one
///   unterminated token and the force-push VANISHED (audit 2 — a missed
///   violation on a HARD check, in the area v1 had called harmless).
/// - v3 escaped ONLY quotes — and audit 4's LOW 6 then measured the mirror:
///   `echo hello\; git push --force origin main` SPLIT at the `;`, because an
///   escaped separator still read as a separator, and a line where bash runs
///   one harmless echo DENIED as a force-push.
///
/// v4, and the reasoning that decides it: **this estate's commands carry
/// Windows paths constantly and POSIX escapes almost never.** A dropped
/// command is a missed violation on a deny-class gate; a mis-parsed POSIX
/// escape is cosmetic. So a backslash escapes quotes AND the separator
/// characters (`;`, `|`, `&`) — enough to embed a literal quote or a literal
/// separator, and nothing else. Whitespace and backslash are literal, which
/// keeps `C:\repo\`, `\\server\share` and `a\b` all intact.
///
/// **Consequences accepted deliberately:** `ls a\ b` lexes to two tokens
/// rather than one (unchanged since v3, pinned). A line that mixes an escaped
/// separator with a genuinely unclosed quote still falls to the degraded
/// tier, where the quote-blind naive splitter cuts at the raw `;` — that
/// direction has the estate's standing ruling for lines bash refuses.
/// INSIDE double quotes, grammar C (`tokenize_escapes`) honours `\"` and `\\`
/// only; this function never applies there.
fn is_escapable(c: char) -> bool {
    c == '"' || c == '\'' || c == ';' || c == '|' || c == '&'
}

/// One lexed token, and whether it was an unquoted shell separator.
pub(crate) struct Token {
    pub(crate) text: String,
    pub(crate) separator: bool,
}

/// State carried through the tokeniser, kept in one struct so the loop body
/// stays a flat sequence of small decisions rather than a nest of branches.
struct Lexer {
    out: Vec<Token>,
    buf: String,
    started: bool,
    escapes: bool,
}

impl Lexer {
    fn new(escapes: bool) -> Self {
        Self {
            out: Vec::new(),
            buf: String::new(),
            started: false,
            escapes,
        }
    }

    fn push_char(&mut self, c: char) {
        self.buf.push(c);
        self.started = true;
    }

    /// Commit a quoted run's content. Empty content still counts as a word
    /// (`""` is an argument), hence `started`.
    fn push_quoted(&mut self, content: &str) {
        self.buf.push_str(content);
        self.started = true;
    }

    fn flush(&mut self) {
        if self.started {
            self.out.push(Token {
                text: std::mem::take(&mut self.buf),
                separator: false,
            });
            self.started = false;
        }
    }

    fn push_separator(&mut self, text: &str) {
        self.flush();
        self.out.push(Token {
            text: text.to_string(),
            separator: true,
        });
    }
}

/// Quote-aware tokenisation under grammar A (literal quotes). `None` means an
/// unterminated quote.
///
/// The caller does NOT then judge nothing — that was the old contract and it
/// was the defect. `cmdline::segments_detailed` tries grammar C next, then the
/// lenient tier.
pub(crate) fn tokenize(command: &str) -> Option<Vec<Token>> {
    tokenize_mode(command, false, false)
}

/// Grammar C: identical, except `\"` and `\\` escape inside DOUBLE quotes,
/// bash-faithful. Single quotes never escape in either grammar.
pub(crate) fn tokenize_escapes(command: &str) -> Option<Vec<Token>> {
    tokenize_mode(command, false, true)
}

/// Grammar A on a line strict lexing rejected: an unclosed quote is REPAIRED
/// to a literal character and lexing continues.
///
/// ⛔ THE OLD LENIENT CONTRACT SWALLOWED THE REST OF THE LINE into one token.
/// That only worked by luck of clause order — an unclosed quote BEFORE a
/// violation hid it, which is one half of audit 4's CRITICAL 2 cross-product
/// (the other half being a quoted separator inside the violation, which the
/// naive splitter then cut blind). Repair confines the damage to the single
/// word; everything after it lexes normally.
///
/// WHAT REPAIR CANNOT PROMISE, stated rather than implied: a quote that truly
/// runs to the end (`git push --force origin "main`) now yields the token
/// `main"` and no longer proves a push. The swallow used to deny that line by
/// accident; bash refuses it outright, and the prove-only ruling judges what
/// runs — nothing runs.
pub(crate) fn tokenize_lenient(command: &str) -> Vec<Token> {
    tokenize_mode(command, true, false).unwrap_or_default()
}

/// Grammar C's lenient sibling, same repair rule.
pub(crate) fn tokenize_lenient_escapes(command: &str) -> Vec<Token> {
    tokenize_mode(command, true, true).unwrap_or_default()
}

fn tokenize_mode(command: &str, lenient: bool, escapes: bool) -> Option<Vec<Token>> {
    let chars: Vec<char> = command.chars().collect();
    let mut lex = Lexer::new(escapes);
    let mut i = 0;
    while i < chars.len() {
        i = step(&chars, i, lenient, &mut lex)?;
    }
    lex.flush();
    Some(lex.out)
}

/// Consume ONE lexical unit at `i` and return the next index.
///
/// Extracted from the loop because the combined form measured CCN 11 against
/// the repo's cap of 10. `None` means an unterminated quote in STRICT mode —
/// the only way lexing fails.
fn step(chars: &[char], i: usize, lenient: bool, lex: &mut Lexer) -> Option<usize> {
    let c = chars[i];
    if c == '\'' || c == '"' {
        return quoted_step(chars, i, i, c, lex.escapes, lenient, lex);
    }
    // `$'…'` / `$"…"` — ANSI-C/locale quoting. The `$` belongs to the QUOTE,
    // not the word: without this, `bash -c $'git push …'` re-lexed as the
    // word `$git` and the push vanished (Warden16to19Reviewer, P2).
    if c == '$' && matches!(chars.get(i + 1), Some('\'') | Some('"')) {
        return quoted_step(chars, i, i + 1, chars[i + 1], true, lenient, lex);
    }
    if let Some(next) = boundary(chars, i, lex) {
        return Some(next);
    }
    if let Some(sep) = separator_at(chars, i) {
        lex.push_separator(sep);
        return Some(i + sep.chars().count());
    }
    if c.is_whitespace() {
        lex.flush();
        return Some(i + 1);
    }
    lex.push_char(c);
    Some(i + 1)
}

/// Consume a quoted run opening at `open`. `at` is the index the caller saw
/// (the `$` for ANSI-C runs) — the LENIENT REPAIR pushes that character as a
/// literal and lexing continues just past it; strict mode propagates the
/// failure. Split from `step` after the dollar-quote rule pushed it to CCN
/// 12 against the cap of 10 — the gate caught it, and the fix is a split.
fn quoted_step(
    chars: &[char],
    at: usize,
    open: usize,
    quote: char,
    escapes: bool,
    lenient: bool,
    lex: &mut Lexer,
) -> Option<usize> {
    match quoted_run(chars, open, quote, escapes) {
        Some((content, next)) => {
            lex.push_quoted(&content);
            Some(next)
        }
        None if lenient => {
            lex.push_char(chars[at]);
            Some(at + 1)
        }
        None => None,
    }
}

/// Word-boundary constructs — an escape outside quotes, and a `#` comment at
/// a word start (bash's rule: mid-word `a#b` is literal, quoted hashes are
/// consumed inside `quoted_run` and never reach here). Extracted from `step`
/// after the comment rule pushed it to CCN 12 against the cap of 10 — the
/// gate caught it, and the fix is a split, never a trim.
fn boundary(chars: &[char], i: usize, lex: &mut Lexer) -> Option<usize> {
    let c = chars[i];
    if c == '\\' && i + 1 < chars.len() && is_escapable(chars[i + 1]) {
        lex.push_char(chars[i + 1]);
        return Some(i + 2);
    }
    if c == '#' && !lex.started {
        return Some(comment_end(chars, i));
    }
    None
}
