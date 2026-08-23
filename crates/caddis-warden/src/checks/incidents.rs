//! incidents.rs — the ONE stateful check, held apart on purpose.
//!
//! Every other check in this crate is a total function of the command string:
//! same input, same verdict, microseconds, no filesystem. That promise is
//! load-bearing — it is why they run on the hot path without thought — so this
//! module lives beside them rather than quietly eroding the rule next door.
//!
//! THE CONTRACT HERE: one bounded read of a small local file. No network, no
//! subprocess, no repo walk. That is the minimum needed to answer "has this repo
//! an unresolved history-rewrite incident?", which cannot be answered from the
//! command text alone — the whole reason an outcome latch exists.
//!
//! WHY IT BLOCKS RATHER THAN WARNS: detection after the act cannot undo it. Once
//! shared history in a repo has been rewritten, pushing MORE work on top makes
//! the recovery harder, so further pushes are refused until a human has looked.
//! The refusal is made cheap on purpose — the message names the exact command
//! that clears it, because a gate nobody can get past gets disabled instead of
//! satisfied.
//!
//! ⚠ NO JSON LIBRARY. This crate carries zero third-party dependencies, so the
//! few fields needed are lifted out by hand. A line that does not yield them is
//! SKIPPED, exactly as the estate's Python skips a line that will not parse:
//! that under-blocks a real incident rather than inventing one, and the incident
//! stays loud in the session banner regardless. Stated as the known gap it is.

use super::Finding;
use crate::checks::cmdline::segments;
use crate::checks::gitgrammar::is_git_subcommand;
use std::path::Path;

/// The log reader lives next door; re-exported so callers and tests keep one
/// import path for "incidents".
pub use super::incident_log::{open_incidents, open_incidents_from, Incident};

/// A leaf name occurring as a whole PATH SEGMENT, not as a loose substring.
///
/// ⚠ THIS DELIBERATELY DIVERGES FROM THE PYTHON IT MIRRORS, and the divergence
/// is a fix rather than a preference. The estate's version asks
/// `leaf in command`, which for a real incident repo ending in `.../wt` makes
/// the leaf the two letters `wt` — matching `git push origin newt`, or any
/// command that merely contains those letters anywhere. On a SOFT check that
/// would be noise; this is a HARD check that DENIES a push, and a deny-class
/// gate firing on an unrelated command is exactly how a mechanism gets switched
/// off. So the leaf must sit on a boundary: start/end of string, or bounded by
/// a character that cannot be part of a path segment.
fn leaf_as_segment(haystack: &str, leaf: &str) -> bool {
    if leaf.is_empty() {
        return false;
    }
    let bound = |c: char| !(c.is_alphanumeric() || c == '-' || c == '_' || c == '.');
    haystack.match_indices(leaf).any(|(at, _)| {
        let before_ok = at == 0 || haystack[..at].chars().next_back().is_some_and(bound);
        let after = at + leaf.len();
        let after_ok =
            after >= haystack.len() || haystack[after..].chars().next().is_some_and(bound);
        before_ok && after_ok
    })
}

/// Does this command plausibly act on the incident repo?
///
/// Matched on the repo's LEAF name as well as its full path, because commands
/// reach a repo by `cd`, by `git -C`, or simply by already being there. Coarse
/// on purpose: it decides only WHICH repo an existing incident applies to, never
/// whether an incident exists — so a miss under-blocks rather than inventing.
fn mentions_repo(command: &str, repo: &str) -> bool {
    let haystack = command.replace('\\', "/").to_lowercase();
    let normalised = repo.replace('\\', "/").to_lowercase();
    if !normalised.is_empty() && haystack.contains(&normalised) {
        return true;
    }
    match normalised.rsplit('/').find(|s| !s.is_empty()) {
        Some(leaf) => leaf_as_segment(&haystack, leaf),
        None => false,
    }
}

/// Is the process ALREADY INSIDE the incident repo?
///
/// ⛔ AUDIT FINDING 3: the doc below promises coverage for reaching a repo "by
/// `cd`, by `git -C`, or simply by ALREADY BEING THERE" — and the check never
/// read the cwd at all, so the most ordinary case of the three was the one it
/// missed. A promise in a comment that the code does not keep is worse than a
/// missing feature: it is a documented guarantee that silently is not one.
///
/// Prefix match on the normalised path, so a subdirectory of the incident repo
/// counts — you are still in that repo.
fn cwd_inside_repo(cwd: &Path, repo: &str) -> bool {
    let here = cwd.to_string_lossy().replace('\\', "/").to_lowercase();
    let normalised = repo.replace('\\', "/").to_lowercase();
    let normalised = normalised.trim_end_matches('/');
    if normalised.is_empty() || here.is_empty() {
        return false;
    }
    here == normalised || here.starts_with(&format!("{normalised}/"))
}

/// What the denial shows for the rewrite's new side. `"new": null` means the
/// ref VANISHED — there is no new commit to name. Rendering it as an empty
/// string printed *"655f64d2 is not an ancestor of ."* — a measurement-shaped
/// sentence with nothing measured in it (LOW 7).
fn new_side(incident: &Incident) -> String {
    if incident.new.is_empty() {
        "nothing (the ref vanished)".to_string()
    } else {
        short(&incident.new)
    }
}

fn short(sha: &str) -> String {
    sha.chars().take(8).collect()
}

/// RED when pushing into a repo whose protected history was rewritten.
///
/// Scoped to pushes rather than to every command in the repo: reading, testing
/// and committing locally are all safe and are often exactly what the recovery
/// needs. It is PUBLISHING more history on top of a rewrite that deepens the
/// damage.
pub fn push_into_rewritten_repo(command: &str, cwd: &Path) -> Finding {
    push_into_rewritten_repo_in(command, cwd, &open_incidents())
}

/// Kept for callers that genuinely have no directory to offer. Prefer
/// `push_into_rewritten_repo_in` — a latch that cannot see where it is standing
/// misses the commonest way of being in a repo.
pub fn push_into_rewritten_repo_with(command: &str, incidents: &[Incident]) -> Finding {
    push_into_rewritten_repo_in(command, Path::new(""), incidents)
}

/// The PURE half, so the latch can be driven against a known incident set.
pub fn push_into_rewritten_repo_in(command: &str, cwd: &Path, incidents: &[Incident]) -> Finding {
    if incidents.is_empty() {
        return None;
    }
    for tokens in segments(command) {
        if !is_git_subcommand(&tokens, "push") {
            continue;
        }
        for incident in incidents {
            if mentions_repo(command, &incident.repo) || cwd_inside_repo(cwd, &incident.repo) {
                return Some(format!(
                    "UNRESOLVED HISTORY REWRITE in {} ({}: {} is not an ancestor of {}). Pushing \
                     more work on top makes recovery harder. Recover the lost commits from a \
                     clone that still has them, then clear this with:\n  python \
                     ~/.claude/hooks/outcome-watch.py --resolve",
                    incident.repo,
                    incident.reference,
                    short(&incident.old),
                    new_side(incident)
                ));
            }
        }
    }
    None
}
