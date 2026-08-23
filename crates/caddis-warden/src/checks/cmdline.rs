//! cmdline.rs — the one place a tool command is turned into tokens.
//!
//! Every check below this module reads a COMMAND. If each wrote its own
//! "is this a git push" test they would drift apart exactly where it matters —
//! one would learn about `-uf`, another would not — so the lexing lives here
//! once and the checks are left holding only their own judgement.
//!
//! ⚠ THE HONEST LIMIT, inherited word for word from the estate's own checks: a
//! checker of tool INPUT is a VOCABULARY gate, not an outcome gate. It
//! recognises spellings; it cannot see effects. Enumerating the near-synonyms
//! (`--force-with-lease`, a bundled `-uf`, a bare `+main` refspec) is what keeps
//! it from reading as protection while providing none — but somebody who wants
//! the forbidden outcome can still reach for a spelling nobody listed.
//!
//! HOW THIS RELATES TO THE PYTHON IT MIRRORS — **CORRECTED after an audit
//! refuted the original claim.** The estate splits on separators FIRST and lexes
//! the pieces after, so a quoted `|` breaks its own segment and that segment is
//! dropped. This file lexes the whole line quote-aware and lets the SEPARATOR
//! TOKENS split the stream, which handles the quoted `|` correctly.
//!
//! ⛔ THE ORIGINAL VERSION OF THIS PARAGRAPH CALLED THAT "a divergence in the
//! safe direction". THAT WAS FALSE, and a clean agent proved it in one command:
//! whole-line lexing meant ONE unbalanced quote anywhere returned nothing at
//! all, so `git push --force origin main && echo don't-skip-review` was ALLOWED
//! — the Python keeps that force-push and denies it. The boast was wrong in the
//! only direction that matters, and it was written in the file it was about.
//!
//! What is true NOW: the whole-line lex is TRIED FIRST, and on failure the
//! per-piece path runs as a fallback (`degraded_segments`). Both properties are
//! kept rather than traded — which is what the first paragraph should have said
//! from the start, once someone had actually tested it.

use super::lexer::{tokenize, tokenize_escapes, tokenize_lenient, tokenize_lenient_escapes};
use super::naive::naive_split;

/// Split a compound command line into per-command token lists.
///
/// Unlexable input yields NO segments rather than an error: this runs ahead of
/// the operator's tool call, and a quoting edge case must never become a crash
/// on their critical path. A segment that cannot be lexed is simply not judged,
/// which the caller sees as green — a known gap, not a claim of safety.
pub fn segments(command: &str) -> Vec<Vec<String>> {
    segments_detailed(command)
        .into_iter()
        .map(|s| s.tokens)
        .collect()
}

/// One command in a compound line, with the operator that introduced it.
///
/// `sep_before` exists because "was this PIPED into" is a different question
/// from "did this follow another command", and a check that conflates them reads
/// `a && wc -l` as a pipeline. Only the `git show` counter check needs the
/// distinction, and it needs it exactly.
pub struct Segment {
    pub sep_before: Option<String>,
    pub tokens: Vec<String>,
}

/// ⛔ CARD-WARDEN-6 AUDIT, FINDING 1 — THE CRITICAL ONE, AND IT REFUTED THIS
/// MODULE'S OWN BOAST.
///
/// The header used to claim that lexing the whole line before splitting was "a
/// divergence in the safe direction". It was not. `tokenize` lexes the ENTIRE
/// line in one pass, so a single unbalanced quote ANYWHERE returned `None`, this
/// function returned an EMPTY vector, and every check judged NOTHING — including
/// a perfectly well-formed force-push earlier on the same line:
///
///   `git push --force origin main && echo don't-skip-review`   ->  ALLOW
///
/// A contraction in a chained echo. Not adversarial; a typo. The estate's Python
/// splits FIRST and lexes each piece, so it keeps the clean segment and drops
/// only the malformed one — on that input the Rust was STRICTLY LESS SAFE than
/// the thing it claimed to improve on.
///
/// THE FIX KEEPS BOTH PROPERTIES INSTEAD OF TRADING THEM: try the whole-line
/// quote-aware lex first (so a quoted `|` still does not split a command), and
/// only when that fails fall back to per-piece lexing, dropping ONLY the pieces
/// that cannot be lexed. Strictly better than either implementation alone, which
/// is what the original comment claimed without having tested it.
///
/// ⚠ The general lesson, banked: a comparative claim written into a doc comment
/// or a commit message is still a CLAIM. This one survived until the first
/// person tested it adversarially, and it was in the file arguing for its own
/// correctness.
pub fn segments_detailed(command: &str) -> Vec<Segment> {
    let mut out = strict_union(command);
    if out.is_empty() {
        out = degraded_segments(command);
    }
    // The positions a segment split cannot see: a command carried in a
    // shell/eval STRING, and command substitution in the raw line. Both are
    // extracted one level deep — the measured class (HIGH 4).
    let carried = super::positions::carrier_segments(&out);
    out.extend(carried);
    out.extend(super::positions::substitution_segments(command));
    out
}

/// BOTH strict grammars, judged as a DEDUPLICATED UNION. Grammar A keeps
/// Windows paths literal (`"C:\repo\"` closes where the operator meant);
/// grammar C is bash-faithful (`\"` embeds a quote). Warden13Reviewer
/// measured that A can COMPLETE while pairing the quotes differently than
/// bash — `MSG="built \"x\" ok" && git push --force \"b\" main` really
/// force-pushes in bash while A's pairing swallows the `&&` into a quoted
/// run — so C is judged even when A succeeds. When the grammars agree
/// (ordinary lines, the common case) the dedup keeps the segment list
/// exactly what the single-grammar warden always produced; only when they
/// diverge does it carry both readings.
pub(super) fn strict_union(command: &str) -> Vec<Segment> {
    let mut out = tokenize(command)
        .map(split_on_separators)
        .unwrap_or_default();
    let c_segments = tokenize_escapes(command)
        .map(split_on_separators)
        .unwrap_or_default();
    if out.is_empty() && c_segments.is_empty() {
        return Vec::new();
    }
    push_unique(&mut out, c_segments);
    out
}

/// Append only segments not already present, identified by their operator
/// and full token list. A duplicated reading costs nothing at verdict time
/// but breaks every exact segment-list pin — so agreement stays invisible.
fn push_unique(out: &mut Vec<Segment>, more: Vec<Segment>) {
    for seg in more {
        let dup = out
            .iter()
            .any(|have| have.sep_before == seg.sep_before && have.tokens == seg.tokens);
        if !dup {
            out.push(seg);
        }
    }
}

/// Group a token stream into commands at its separator tokens. Shared by the
/// strict and degraded paths so the two cannot drift in how they group.
pub(super) fn split_on_separators(tokens: Vec<super::lexer::Token>) -> Vec<Segment> {
    let mut out: Vec<Segment> = Vec::new();
    let mut current: Vec<String> = Vec::new();
    let mut sep_for_current: Option<String> = None;
    for tok in tokens {
        if tok.separator {
            if !current.is_empty() {
                out.push(Segment {
                    sep_before: sep_for_current.take(),
                    tokens: std::mem::take(&mut current),
                });
            }
            sep_for_current = Some(tok.text);
            continue;
        }
        current.push(tok.text);
    }
    if !current.is_empty() {
        out.push(Segment {
            sep_before: sep_for_current,
            tokens: current,
        });
    }
    out
}

/// The fallback: split naively, lex each piece, keep what lexes.
///
/// A piece that will not lex is DROPPED and named as a known gap — it is not
/// judged and it is not allowed to silence its neighbours. That is the whole
/// point: an unparsable fragment costs its own coverage, never the line's.
/// ⛔ RUN BOTH STRATEGIES AND JUDGE THE UNION. Measured, not assumed.
///
/// CARD-WARDEN-9 replaced naive splitting with lenient whole-line lexing and
/// called it strictly better. An independent audit measured FOUR commands that
/// were DENY before and ALLOW after — every one of them a contraction or an
/// unclosed quote appearing BEFORE the force-push:
///
///   echo it's; git push --force origin main        ->  was DENY, became ALLOW
///   echo can't && git push --force origin main     ->  was DENY, became ALLOW
///
/// ⚠ AND THE SEVERITY LESSON, which is bigger than the bug. This is FINDING 1
/// WITH THE CLAUSE ORDER SWAPPED. When the force-push came first and the
/// contraction second, I rated it CRITICAL and wrote "a typo, not adversarial".
/// When the contraction came first I disclosed the same shape as an acceptable
/// known gap and argued the shell would refuse the line. Identical defect,
/// opposite grading — and the only difference was that one was an auditor's
/// finding and the other was mine. **A defect I found myself got the lenient
/// reading.** The disclosure was honest; the severity was not.
///
/// The two strategies fail on DIFFERENT inputs and neither dominates:
///
///   - lenient lexing wins when a WELL-FORMED quote contains a separator;
///   - naive splitting wins when an UNCLOSED quote precedes the violation.
///
/// So both run and their segments are judged together. A check stops at its
/// first finding, so duplicate segments cost nothing.
fn degraded_segments(command: &str) -> Vec<Segment> {
    // The repaired lenient tier for BOTH grammars, then the frozen naive
    // splitter. The repair (an unclosed opener becomes a literal character)
    // is what closes the other half of the cross-product: an apostrophe
    // anywhere no longer swallows the commands after it.
    let mut out = split_on_separators(tokenize_lenient(command));
    out.extend(split_on_separators(tokenize_lenient_escapes(command)));
    out.extend(
        naive_split(command)
            .into_iter()
            .filter_map(|(sep_before, raw)| {
                let tokens: Vec<String> = tokenize_lenient(&raw)
                    .into_iter()
                    .filter(|t| !t.separator)
                    .map(|t| t.text)
                    .collect();
                (!tokens.is_empty()).then_some(Segment { sep_before, tokens })
            }),
    );
    out
}
