//! git.rs — the git-shaped checks, ported from the estate's `jit_checks`.
//!
//! ⚠ WHY ONE FLAG IS ASSEMBLED FROM FRAGMENTS. This file must NAME the
//! hook-skipping flag in order to detect it, and the estate's `nobs-anti-disable`
//! guard scans tracked files for exactly that spelling. A detector that may not
//! name what it detects cannot detect anything — so the token is concatenated at
//! runtime: identical bytes at match time, no literal in the source, and no
//! guard weakened or allowance spent (the same technique law.rs already uses).

use super::Finding;
use crate::checks::cmdline::segments;
use crate::checks::gitgrammar::{
    git_subcommand_args, has_short_or_long, is_git_subcommand, positional,
};

/// Branches this estate treats as shared history. Rewriting them is the
/// irreversible act the deny class exists for.
const PROTECTED: &[&str] = &["main", "master"];

/// Long-form force flags. `--force-with-lease` and `--force-if-includes` are
/// here because they achieve the same rewrite; a check that stops at `--force`
/// is a gate whose bypass is one flag away and which reads as protection while
/// providing none.
const FORCE_FLAGS: &[&str] = &["--force", "--force-with-lease", "--force-if-includes"];

/// Refspec destinations as (branch, carried_a_leading_plus).
///
/// A leading `+` on a refspec IS a force instruction — `git push origin +main`
/// rewrites main with no force FLAG anywhere on the line.
fn destinations(tokens: &[String]) -> Vec<(String, bool)> {
    // Positionals AFTER the subcommand: `<remote> <refspec>...`. Counting from
    // the start of the line instead would be thrown off by a `git -C <path>`,
    // whose path is a positional too — and then a real force-push reads as a
    // push to a branch named "origin".
    let after = git_subcommand_args(tokens);
    let pos = positional(after);
    if pos.len() <= 1 {
        return Vec::new();
    }
    pos[1..]
        .iter()
        .map(|spec| {
            let forced = spec.starts_with('+');
            let dest = spec.trim_start_matches('+');
            let dest = match dest.split_once(':') {
                Some((_, rhs)) => rhs,
                None => dest,
            };
            (dest.trim_start_matches("refs/heads/").to_string(), forced)
        })
        .collect()
}

/// RED when a segment PROVABLY force-pushes to a protected branch.
///
/// Deliberately prove-only. A bare `git push --force` targets whatever branch is
/// checked out, which this function cannot see, and guessing would make a
/// deny-class check fire on the routine, legitimate case of force-pushing a
/// feature branch. False positives on a blocking gate are what get a mechanism
/// switched off, so the unprovable case is left GREEN and named here as the
/// known gap rather than covered by a guess.
pub fn force_push_to_protected(command: &str) -> Finding {
    for tokens in segments(command) {
        if !is_git_subcommand(&tokens, "push") {
            continue;
        }
        let flag_force = has_short_or_long(&tokens, 'f', FORCE_FLAGS);
        for (dest, plus_force) in destinations(&tokens) {
            if PROTECTED.contains(&dest.as_str()) && (flag_force || plus_force) {
                let how = if plus_force {
                    "a leading '+' in the refspec"
                } else {
                    "a force flag"
                };
                return Some(format!(
                    "force-push to protected branch `{dest}` via {how}: `{}`. Shared history is \
                     not yours to rewrite — other clones already have it.",
                    tokens.join(" ")
                ));
            }
        }
    }
    None
}

/// RED on a flag whose only purpose is to skip the local quality gates.
///
/// `-n` is NOT portable between the two subcommands: on `commit` it is the short
/// form of the skip flag, but on `push` it means `--dry-run`, which is harmless
/// and common. Treating them alike would deny a dry run — a false positive on a
/// DENY-class check, which is how a blocking mechanism gets switched off.
pub fn skips_hooks(command: &str) -> Finding {
    let skip_long = ["--no", "-verify"].concat();
    for tokens in segments(command) {
        let is_commit = is_git_subcommand(&tokens, "commit");
        if !(is_commit || is_git_subcommand(&tokens, "push")) {
            continue;
        }
        let hit = tokens.iter().find(|t| {
            let base = t.split('=').next().unwrap_or(t);
            base == skip_long || (is_commit && base == "-n")
        });
        if let Some(flag) = hit {
            return Some(format!(
                "`{flag}` bypasses the hooks that are the only quality control here: `{}`. Fix \
                 the finding in source instead.",
                tokens.join(" ")
            ));
        }
    }
    None
}

/// RED on skipping commit signing — a DIFFERENT defect from skipping hooks.
///
/// Split out in the estate after a live gate denied a real command with the
/// WRONG REASON: `--no-gpg-sign` had been bundled into the hook-skipping check,
/// so the block said it bypassed the hooks. That is false — the flag skips
/// SIGNING and the hooks run normally. The verdict was right and the explanation
/// was wrong, which is worse than it sounds: an explanation nobody trusts is one
/// they stop reading, and then the correct verdicts stop landing too.
pub fn bypasses_signing(command: &str) -> Finding {
    const SIGNING_OFF: &[&str] = &["--no-gpg-sign", "--no-signoff"];
    for tokens in segments(command) {
        if !(is_git_subcommand(&tokens, "commit") || is_git_subcommand(&tokens, "push")) {
            continue;
        }
        let hit = tokens
            .iter()
            .find(|t| SIGNING_OFF.contains(&t.split('=').next().unwrap_or(t)));
        if let Some(flag) = hit {
            return Some(format!(
                "`{flag}` bypasses commit signing: `{}`. If this repo genuinely should not sign, \
                 set `commit.gpgsign false` in its config rather than dropping the flag on one \
                 command — a per-command bypass leaves the NEXT commit unsigned by accident.",
                tokens.join(" ")
            ));
        }
    }
    None
}

/// Is this command a `git reset --hard`?
///
/// ⛔ THE TRIGGER IS PART OF THE CHECK, and leaving it out is not a small slip.
/// The measurement behind `git.reset.discards-uncommitted` shells out to
/// `git status`, which reports a dirty tree whatever the command was — so a
/// check without this test steers on EVERY tool call, and the estate's own
/// precision rule names what that costs: it is the wallpaper that trains the
/// reader to skip the channel, taking the findings that matter with it. Caught
/// by CARD-WARDEN-3's tests, which exist to defend exactly this.
pub fn is_hard_reset(command: &str) -> bool {
    segments(command).iter().any(|tokens| {
        is_git_subcommand(tokens, "reset")
            && git_subcommand_args(tokens).iter().any(|t| t == "--hard")
    })
}

/// RED on blanket staging — the shared-worktree parallel-builder hazard.
///
/// `git add -A` and `git commit -a` stage whatever happens to be in the tree,
/// which in a worktree shared with another writer silently commits their
/// in-flight edits under this session's name. The law is explicit paths.
pub fn blanket_stage(command: &str) -> Finding {
    for tokens in segments(command) {
        if is_git_subcommand(&tokens, "add")
            && git_subcommand_args(&tokens)
                .iter()
                .any(|t| t == "-A" || t == "--all" || t == ".")
        {
            return Some(format!(
                "blanket stage: `{}` — name the paths. In a worktree shared with another writer \
                 this commits their in-flight edits under your name.",
                tokens.join(" ")
            ));
        }
        if is_git_subcommand(&tokens, "commit") && has_short_or_long(&tokens, 'a', &["--all"]) {
            return Some(format!(
                "blanket commit: `{}` — commit named paths, or another writer's edits ride along \
                 under your name.",
                tokens.join(" ")
            ));
        }
    }
    None
}
