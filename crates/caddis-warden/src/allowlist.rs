//! allowlist.rs — the open card BOUNDS the edits (CARD-0111, unit B's teeth).
//!
//! CARD-0110 made "a card is open" a fact in the ledger and enforced nothing.
//! This connects the two halves that have always existed separately: the schema
//! has declared `allowlist` since CARD-0003, and the warden has seen every write
//! path since it was born.
//!
//! ⚠ WHAT THIS IS, STATED SO NOBODY OVERSELLS IT LATER. It is a DECLARATION
//! gate over write targets the warden is HANDED, not a filesystem sandbox.
//! Measured over the live ledger: file-write tools are 13.5% of rows and shell
//! is 75.6%, and a shell command's write targets are not recoverable in general.
//! It makes an agent's own declared bounds mechanical. It does not contain a
//! determined process, and the program record says so in REVISION 1.

use crate::card_state;
use crate::identity::{caller_id, ledger_path};
use std::path::Path;

/// Paths no card may fence off, whatever it declares.
///
/// THE LEDGER IS FIRST FOR A REASON: a gate that can stop the warden from
/// recording its own verdicts breaks the audit trail the entire trust argument
/// rests on. The rest are build and VCS machinery no card means to govern, and
/// fencing them would make the gate obstructive — which is how a warden gets
/// switched off, and a switched-off warden protects nothing.
const EXEMPT_DIRS: [&str; 4] = ["target/", "node_modules/", ".git/", ".cargo/"];

/// How sure the warden is about the write target, which decides the verdict.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Certainty {
    /// A file tool's own `path`, or a literal `>`/`>>` target. Handed over,
    /// not inferred.
    Certain,
    /// Recovered from shell text. Good enough to warn, never to refuse.
    Inferred,
}

/// The breach, if this call writes outside the open card's allowlist.
pub struct Breach {
    pub card_id: String,
    pub target: String,
    pub declared: Vec<String>,
    pub certainty: Certainty,
}

impl Breach {
    pub fn reason(&self) -> String {
        format!(
            "caddis-warden [card.allowlist]: {} declares an allowlist that does not include \
             `{}`. Declared: {}. Either the write is outside the work the card describes, or \
             the card understates its own blast radius — both are worth a moment before \
             continuing. Widen the card and reopen it, or `caddis-warden card close` first.",
            self.card_id,
            self.target,
            if self.declared.is_empty() {
                "(nothing)".to_string()
            } else {
                self.declared.join(", ")
            }
        )
    }
}

/// Fold a path to the form the allowlist is compared in.
///
/// Backslashes fold to `/` because the same repository is addressed both ways on
/// Windows; case folds ONLY on Windows, so two genuinely different files on
/// Linux never collide; trailing slashes go so `src` and `src/` are one entry.
pub fn normalize(p: &str) -> String {
    // ⚠ TRIM FIRST. A path arriving with a stray newline or space — a
    // hand-built frame, a miscounted length, a shell that appended one — would
    // otherwise never equal its own allowlist entry, so a DECLARED file gets
    // denied and lands in the attest bundle's OUTSIDE list. Found by driving
    // the whole cherry end to end; every fixture had already been clean.
    // The old behaviour failed CLOSED, which is the safe direction, but a gate
    // that refuses the file you declared is a gate people switch off.
    let slashed = p.trim().replace('\\', "/");
    let folded = if cfg!(windows) {
        slashed.to_lowercase()
    } else {
        slashed
    };
    folded.trim_end_matches('/').to_string()
}

/// The write target as a repo-relative path, when it is inside `cwd`.
///
/// An absolute path outside the working directory keeps its own shape rather
/// than being forced into a relative one it does not have.
pub fn relative_to(target: &str, cwd: &Path) -> String {
    let t = normalize(target);
    let root = normalize(&cwd.to_string_lossy());
    match t.strip_prefix(&root) {
        Some(rest) => rest.trim_start_matches('/').to_string(),
        None => t,
    }
}

fn is_exempt(rel: &str, ledger: &str) -> bool {
    if rel == normalize(ledger) || rel.ends_with("warden-ledger.jsonl") {
        return true;
    }
    // A lock file beside the ledger is the ledger's own machinery.
    if rel.ends_with("warden-ledger.lock") {
        return true;
    }
    let temp = normalize(&std::env::temp_dir().to_string_lossy());
    if !temp.is_empty() && rel.starts_with(&temp) {
        return true;
    }
    EXEMPT_DIRS
        .iter()
        .any(|d| rel.starts_with(d) || rel.contains(&format!("/{d}")))
}

/// Does `rel` fall inside one declared entry?
///
/// Exact match, or subtree when the entry ends in `/`. NO GLOBS: a glob in a
/// declaration is a promise nobody can check by reading it, and the card law
/// wants the allowlist to be the exact editable paths.
pub fn declared_covers(declared: &[String], rel: &str) -> bool {
    declared.iter().any(|entry| {
        let e = normalize(entry);
        if e.is_empty() {
            return false;
        }
        // A `..` escape never matches: a declaration cannot reach outward.
        if e.contains("..") {
            return false;
        }
        if entry.trim_end().ends_with('/') {
            return rel == e || rel.starts_with(&format!("{e}/"));
        }
        rel == e
    })
}

/// The whole check: what does the open card say about this write?
///
/// `None` means "nothing to say" — no card, a card that declares no allowlist,
/// no identifiable target, or a target the card covers. The overwhelmingly
/// common case is the first, and it costs one ledger read.
pub fn breach(target: &str, certainty: Certainty, cwd: &Path) -> Option<Breach> {
    if target.trim().is_empty() {
        return None;
    }
    let ledger = ledger_path();
    let text = std::fs::read_to_string(&ledger).ok()?;
    let active = card_state::active_for(&text, &caller_id()).active?;
    let rel = relative_to(target, cwd);
    // ⛔ THE CARD'S OWN FILE IS NEVER WRITABLE UNDER IT, AND THIS OUTRANKS THE
    // EXEMPTIONS. Otherwise an executor edits its allowlist to fit the work it
    // has already decided to do, and the declaration means nothing. Ordering it
    // after `is_exempt` made a card living under the OS temp directory freely
    // self-editable — found by the test, not by reading the code.
    // `card close` also refuses on the hash, so this is the second of two
    // independent stops rather than the only one.
    if rel == relative_to(&active.path, cwd) {
        return Some(Breach {
            card_id: active.id,
            target: rel,
            declared: vec!["(a card may not rewrite itself)".to_string()],
            certainty: Certainty::Certain,
        });
    }
    let ledger_str = ledger.to_string_lossy().into_owned();
    if is_exempt(&rel, &ledger_str) {
        return None;
    }
    let declared = declared_for(&active.path)?;
    if declared_covers(&declared, &rel) {
        return None;
    }
    Some(Breach {
        card_id: active.id,
        target: rel,
        declared,
        certainty,
    })
}

/// The card's declared allowlist, or `None` when it declares none.
///
/// A v1 card (Done-When and RED-TEST, no EXECUTION section) is a legitimate
/// card and most of this repository's own cards are v1. It bounds nothing, and
/// `card open` already says so out loud rather than implying otherwise.
fn declared_for(card_path: &str) -> Option<Vec<String>> {
    let bytes = std::fs::read_to_string(card_path).ok()?;
    let card = caddis_card::Card::parse(&bytes).ok()?;
    let exec = card.execution()?;
    if exec.allowlist.is_empty() {
        return None;
    }
    Some(exec.allowlist)
}

/// The whole CARD-0111 branch of the law, kept here rather than in `law.rs` so
/// the target extraction, the matching and the verdict mapping read together —
/// and so `law.rs` stays inside the 280-line cap it was already close to.
pub fn verdict_for(call: &crate::ToolCall, cwd: &Path) -> Option<crate::Verdict> {
    let b = breach_for(call, cwd)?;
    Some(match b.certainty {
        Certainty::Certain => crate::Verdict::Deny { reason: b.reason() },
        // The target was recovered from shell text rather than handed over, and
        // a guess is good enough to warn but never to refuse.
        Certainty::Inferred => crate::Verdict::Steer {
            law: "card.allowlist".to_string(),
            why: b.reason(),
        },
    })
}

/// The write target and how sure we are of it, then the allowlist check.
///
/// CERTAIN means the warden was HANDED the destination: a file tool's own
/// `path`, or a literal `>`/`>>` redirect it parsed out of the command line.
fn breach_for(call: &crate::ToolCall, cwd: &Path) -> Option<Breach> {
    let (target, certainty) = if !call.path.is_empty() {
        (call.path.clone(), Certainty::Certain)
    } else if let Some(t) = crate::law::redirect_target(&call.command) {
        (t, Certainty::Certain)
    } else {
        (inferred_target(&call.command)?, Certainty::Inferred)
    };
    breach(&target, certainty, cwd)
}

/// A write target RECOVERED from a command, for the handful of verbs whose
/// destination is unambiguous and positional.
///
/// ⚠ DELIBERATELY TINY, and the reason is the verdict it produces. It can only
/// ever STEER, so a miss costs a message nobody needed — but the channel it
/// spends is scarce: steer is 2.2% of the live ledger today, which is exactly
/// why it still carries meaning. A broad heuristic here would take that toward
/// noise and train agents to ignore the one verdict that teaches.
///
/// Everything else a shell command might write is NOT recoverable in general
/// and nothing here pretends otherwise: no target, no opinion.
fn inferred_target(command: &str) -> Option<String> {
    for segment in crate::checks::cmdline::segments(command) {
        let words: Vec<&str> = segment.iter().map(String::as_str).collect();
        let verb = words.first()?;
        let base = verb.rsplit(['/', '\\']).next().unwrap_or(verb);
        let takes_last = matches!(base, "tee" | "touch" | "cp" | "mv");
        let sed_in_place = base.ends_with("sed") && words.contains(&"-i");
        if !(takes_last || sed_in_place) {
            continue;
        }
        // The last word that is not a flag: `cp -r a b`, `tee -a out`.
        if let Some(last) = words.iter().rev().find(|w| !w.starts_with('-')) {
            if *last != *verb {
                return Some((*last).to_string());
            }
        }
    }
    None
}

#[cfg(test)]
#[path = "allowlist_tests.rs"]
mod tests;
