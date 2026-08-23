//! naive.rs — the quote-BLIND splitter, kept deliberately dumber than the
//! lexer it backs up.
//!
//! Split out of `lexer.rs` under the 280-line file law when the two-grammar
//! fix grew the lexer past the cap. The split is honest about what this file
//! IS: the one place in the crate allowed to ignore quotes entirely, because
//! its job is to be a second opinion that never shares the primary parser's
//! blind spot.
//!
//! ⛔ DO NOT MAKE THIS SMARTER. CARD-WARDEN-9 deleted it; an audit restored
//! it after measuring four DENY→ALLOW regressions. Widening it was then
//! considered for audit 4's cross-product hole and REJECTED — the widening
//! direction is exactly what produced HIGH 3's false deny. It stays
//! quote-blind on purpose; completeness comes from the grammar union in
//! `cmdline`, never from cleverness here.

use super::scan::separator_at;

/// Split raw text on separators with NO understanding of quotes.
///
/// ⛔ RESTORED AFTER A MEASURED REGRESSION. CARD-WARDEN-9 deleted this, believing
/// lenient whole-line lexing strictly dominated it. An independent audit
/// measured FOUR commands that were DENY before and ALLOW after:
///
///   echo it's; git push --force origin main
///   echo can't && git push --force origin main
///   echo "unbalanced; git push --force origin main
///
/// The reason is the gap CARD-WARDEN-9 wrote down and then reasoned away: when
/// the FIRST unclosed quote appears BEFORE the dangerous command, lenient lexing
/// swallows the rest of the line into one token and the force-push disappears.
/// Naive splitting cuts at the separator first and keeps it.
///
/// **NEITHER STRATEGY DOMINATES.** Lenient lexing wins when a well-formed quote
/// contains a separator; naive splitting wins when an unclosed quote precedes
/// the violation. So the degraded path runs BOTH and judges the union —
/// which is what "keep both properties" should have meant the first time.
/// (Audit 4 later closed the remaining cross-product in the LEXER — two strict
/// grammars plus repair — rather than here; this file is frozen on purpose.)
pub(crate) fn naive_split(command: &str) -> Vec<(Option<String>, String)> {
    let chars: Vec<char> = command.chars().collect();
    let mut out: Vec<(Option<String>, String)> = Vec::new();
    let mut sep: Option<String> = None;
    let mut buf = String::new();
    let mut i = 0;
    while i < chars.len() {
        match separator_at(&chars, i) {
            Some(found) => {
                out.push((sep.take(), std::mem::take(&mut buf)));
                sep = Some(found.to_string());
                i += found.chars().count();
            }
            None => {
                buf.push(chars[i]);
                i += 1;
            }
        }
    }
    out.push((sep, buf));
    out.retain(|(_, raw)| !raw.trim().is_empty());
    out
}
