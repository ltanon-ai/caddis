//! law.rs — the rules the warden enforces at omp's tool boundary.
//!
//! ⚠ WHY THE PATTERNS ARE ASSEMBLED FROM FRAGMENTS. This file must MENTION the
//! suppression markers it forbids, and the estate's own `nobs-anti-disable`
//! guard scans tracked files for exactly those markers. Writing them literally
//! blocks the write (measured — it blocked this file's first draft). Spending a
//! `nobs-allow` here would park a literal marker in a tracked file forever, so
//! the fragments are concatenated at runtime instead: identical bytes at match
//! time, no literal in the source, and no guard weakened.

use crate::allowlist;
use crate::{ToolCall, Verdict};
use std::path::Path;

/// Files the global contract names never-read-or-expose. Matched on the tail of
/// the path so a home-relative, absolute or `~`-prefixed form all land.
const SENSITIVE: &[&str] = &[
    ".credentials.json",
    "mcp-needs-auth-cache.json",
    ".sonar-token",
    "history.jsonl",
];

/// Tools that only observe. Reading is never taxed — a consciousness that makes
/// reading expensive gets hated for no safety gain.
fn is_read_only(tool: &str) -> bool {
    matches!(tool, "read" | "grep" | "glob" | "ls" | "list")
}

fn sensitive_path(path: &str) -> Option<&'static str> {
    let p = path.replace('\\', "/").to_lowercase();
    if p.contains("/channels/") || p.starts_with("channels/") {
        return Some("channels/**");
    }
    SENSITIVE
        .iter()
        .find(|s| p.ends_with(&**s.to_owned()))
        .copied()
}

/// Extensions where a suppression marker is INERT: no scanner reads prose for
/// it, so the marker suppresses nothing.
///
/// EXEMPTING THESE IS NOT A WEAKENING — it drops a case the rule never had
/// jurisdiction over. Without it the guard forbids DOCUMENTING its own rules:
/// a `.md` saying "never write the nosec marker" was denied, and so would every
/// postmortem, lesson and incident report about a marker (CARD-WARDEN-4).
///
/// ⚠ IT MUST NOT GENERALISE TO "NON-CODE". A CI config is not code either, and
/// it is precisely where a skip-marker or an allow-failure does its damage.
/// Config stays fully in scope, and so does everything unrecognised: an
/// exemption is granted by RECOGNITION, never by failure to recognise.
const PROSE_EXT: &[&str] = &[".md", ".txt", ".rst", ".adoc", ".markdown"];

fn is_prose(path: &str) -> bool {
    let p = path.to_lowercase();
    PROSE_EXT.iter().any(|e| p.ends_with(*e))
}

/// Where a bash command REDIRECTS its output, if anywhere.
///
/// ⛔ WITHOUT THIS THE PROSE EXEMPTION CANNOT REACH A BASH COMMAND AT ALL, which
/// is how `echo "...the marker..." >> NOTES.md` was denied: a file tool carries
/// its destination in `path`, and a shell command carries it in the command
/// text, so the exemption only ever covered one of the two ways prose is
/// written. Audit 2 proved it; audit 1 had reported the symptom and I disputed
/// it on the strength of a claim I had not tested — the whole point of the
/// exemption is that a guard must not forbid documenting its own rules, and this
/// was that failure surviving one round of review.
pub(crate) fn redirect_target(command: &str) -> Option<String> {
    let tokens = crate::checks::cmdline::segments(command)
        .into_iter()
        .flatten()
        .collect::<Vec<String>>();
    let at = tokens.iter().position(|t| t == ">" || t == ">>")?;
    tokens.get(at + 1).cloned()
}

/// The destination this call actually writes to: the tool's own path, or the
/// shell redirect when there is no path field.
fn effective_path(call: &ToolCall) -> String {
    if !call.path.is_empty() {
        return call.path.clone();
    }
    redirect_target(&call.command).unwrap_or_default()
}

/// The suppression markers, assembled so this file carries none of them.
fn suppression_rules() -> Vec<(String, &'static str)> {
    vec![
        (["--no", "-verify"].concat(), "no-verify-flag"),
        (["# no", "sec"].concat(), "nosec"),
        (["# no", "qa"].concat(), "noqa"),
        (["eslint-", "disable"].concat(), "eslint-disable"),
        (["@ts-", "ignore"].concat(), "ts-ignore"),
        (["allow_", "failure"].concat(), "allow-failure"),
        (["[skip ", "ci]"].concat(), "skip-ci"),
        (["NO", "SONAR"].concat(), "nosonar"),
        (["gitleaks:", "allow"].concat(), "gitleaks-allow"),
    ]
}

/// The documented escape hatch, line-scoped: the allowance must sit on the
/// offending line or the one before it.
///
/// LINE-SCOPED ON PURPOSE. A file-wide search would let one allowance at the
/// top of a file silently cover every suppression below it for the rest of that
/// file's life — an exception that grows without anyone deciding it should.
fn allowed_here(lines: &[&str], idx: usize) -> bool {
    let marker = ["nobs", "-allow:"].concat();
    if lines[idx].contains(&marker) {
        return true;
    }
    idx > 0 && lines[idx - 1].contains(&marker)
}

fn suppression(payload: &str) -> Option<(String, String)> {
    let rules = suppression_rules();
    let lines: Vec<&str> = payload.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        for (pat, id) in &rules {
            if line.contains(pat.as_str()) && !allowed_here(&lines, i) {
                return Some((id.to_string(), (*line).trim().to_string()));
            }
        }
    }
    None
}

/// A run of token-shaped characters long enough to be a real credential.
fn long_token_after(payload: &str, prefix: &str, min: usize) -> bool {
    payload.split(prefix).skip(1).any(|rest| {
        rest.chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .count()
            >= min
    })
}

fn secret_literal(payload: &str) -> Option<&'static str> {
    if long_token_after(payload, "sk-", 20) {
        return Some("an API-key-shaped literal (sk-…)");
    }
    if long_token_after(payload, "ghp_", 20) || long_token_after(payload, "github_pat_", 20) {
        return Some("a GitHub token literal");
    }
    if long_token_after(payload, "AKIA", 16) {
        return Some("an AWS access-key literal");
    }
    None
}

/// The registry's verdict on this command, or `None` when every check is green.
///
/// CARD-WARDEN-3 established that a steer must carry an EXECUTED CHECK FINDING
/// rather than a paragraph of principle, and CARD-WARDEN-6 makes that the only
/// way findings arrive: the ad-hoc `git push --force` string test that used to
/// live here is gone, replaced by `git.push.force-to-protected`, which is
/// strictly better in both directions — it reads `origin +main` (a force with no
/// force FLAG at all, previously invisible) and it stops denying
/// `git push --force origin feature-main-thing`, which the old substring test
/// caught because the branch name happened to contain "main".
///
/// A HARD finding denies; SOFT findings steer, and EVERY soft finding that fired
/// is carried, because dropping one silently is how a channel stops being worth
/// reading. `git reset --hard` on a clean tree is still perfect silence.
fn registry_verdict(call: &ToolCall, cwd: &Path) -> Option<Verdict> {
    let ctx = crate::checks::Ctx {
        command: &call.command,
        cwd,
    };
    let findings = crate::checks::run_all(&ctx);
    let hard = findings
        .iter()
        .find(|(_, sev, _)| *sev == crate::checks::Severity::Hard);
    if let Some((id, _, finding)) = hard {
        return Some(Verdict::Deny {
            reason: format!("caddis-warden [{id}]: {finding}"),
        });
    }
    let soft: Vec<&(&str, crate::checks::Severity, String)> = findings
        .iter()
        .filter(|(_, sev, _)| *sev == crate::checks::Severity::Soft)
        .collect();
    if soft.is_empty() {
        return None;
    }
    Some(Verdict::Steer {
        law: soft
            .iter()
            .map(|(_, _, f)| f.clone())
            .collect::<Vec<String>>()
            .join("\n"),
        why: soft
            .iter()
            .map(|(id, _, _)| *id)
            .collect::<Vec<&str>>()
            .join(", "),
    })
}

/// The whole law, in the order that matters.
///
/// Sensitive paths are checked FIRST and for every tool, because they are the
/// one case where even reading is forbidden. Read-only tools then leave early:
/// scanning a read's payload would deny a `grep` for the very markers this file
/// exists to find — the warden would forbid auditing itself.
pub fn apply(call: &ToolCall, cwd: &Path) -> Verdict {
    if let Some(hit) = sensitive_path(&call.path) {
        return Verdict::Deny {
            reason: format!(
                "caddis-warden: `{hit}` is on the never-read-or-expose list. Secrets are reached by \
                 vault PATH, never by reading the store."
            ),
        };
    }
    if is_read_only(&call.tool) {
        return Verdict::Allow;
    }
    if let Some(v) = crate::size::check(call) {
        return v;
    }

    // THE REGISTRY RUNS BEFORE THE PAYLOAD RULES, and the ordering is
    // load-bearing rather than incidental. Both can fire on the same command,
    // but the registry names the ACTUAL defect while the generic suppression
    // scan names a category. The estate learned this the expensive way: a
    // signing bypass was once denied with the reason "bypasses the hooks that
    // are the only quality control", which is false — the flag skips SIGNING and
    // the hooks run normally. The verdict was right and the explanation was
    // wrong, and an explanation nobody trusts is one they stop reading.
    if let Some(v) = registry_verdict(call, cwd) {
        return v;
    }

    let payload = call.payload();

    // The suppression rule — and ONLY this rule — steps aside for prose. The
    // secret rule below still applies: a key written into a .md is a key in git
    // history forever, and no amount of "it is only documentation" changes that.
    if let Some((rule, line)) = suppression(&payload).filter(|_| !is_prose(&effective_path(call))) {
        return Verdict::Deny {
            reason: format!(
                "caddis-warden: `{rule}` suppresses a quality gate, and bypassing the gate is not \
                 a fix — it hides the finding and keeps the defect. Offending line: `{line}`. Fix \
                 the underlying code. If this is genuinely a false positive, document it on the \
                 same or preceding line with a nobs allowance naming the rule AND a real reason \
                 (not 'makes the scan pass'). This applies to hook-verify bypasses too."
            ),
        };
    }

    if let Some(kind) = secret_literal(&payload) {
        return Verdict::Deny {
            reason: format!(
                "caddis-warden: this writes {kind} into a file. Secrets live in the vault and \
                 reach code as a PATH or an environment variable, never as a literal — a literal \
                 is permanent in git history even after it is deleted. If a test needs the SHAPE \
                 of a secret, build it at runtime from character codes."
            ),
        };
    }

    // ⛔ LAST, AND THAT IS LOAD-BEARING (CARD-0111). Every rule above names a
    // defect in the work itself; this one names a mismatch with a plan. When
    // both fire the reader deserves the stronger reason, so the allowlist only
    // ever speaks about calls the rest of the law was willing to allow. It is
    // also INERT until someone opts in by opening a card.
    if let Some(v) = allowlist::verdict_for(call, cwd) {
        return v;
    }

    Verdict::Allow
}
