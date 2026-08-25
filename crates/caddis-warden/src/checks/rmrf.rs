//! rmrf.rs — the destructive-filesystem law (DESTRUCTIVE-1, gemini ruling).
//!
//! DENY set: `rm -rf` whose target is an absolute root (`/`, `/home`, `/usr`,
//! `/var`, `/etc`, `/boot`, `/root`, `/opt`, `/bin`, `/lib`, `/Users`,
//! `C:\`, `%USERPROFILE%`, `%SystemDrive%`, `%HOMEPATH%`, `$HOME`, `$PWD`),
//! a `..` escape resolving above the workspace, a NULL VARIABLE EXPANSION
//! (undefined `$VAR` in `rm -rf $VAR/` is effectively `rm -rf /` — denied
//! before execution), `.*` (matches `.` and `..`), and bare `*` judged from
//! the workspace root. ALLOW: named relative subpaths — build dirs are
//! legitimate work. `*` after a `cd` STEERS (the wildcard check, Soft).
//!
//! ⚠ HONEST LIMITS: the null-expansion test reads THIS process's
//! environment; a shell-LOCAL variable (defined, unexported) reads as null
//! here while the agent's shell expands it fine. The ruling accepts that
//! cost — the null case is catastrophic and the defined case has a
//! variable-free spelling. Windows `Remove-Item -Recurse -Force` is OUTSIDE
//! this law (the ruling names `rm`); recorded here rather than papered over.
//!
//! No regex anywhere in this crate (zero-dependency law): hand matching.

use super::runners::skip_runner_prefix;
use crate::checks::cmdline::segments;
use crate::checks::Finding;

use super::rmrf_operand::{escapes_above, is_protected_root, null_expansion, rm_operands};

/// RED when an rm -rf target is provably destructive per the ruling.
pub fn protected_root(command: &str) -> Finding {
    let segs = segments(command);
    // A cd prefix moves the rm OUT of the workspace root: bare `*` there is
    // the SOFT wildcard's case, never this law's (the steer never stacks).
    let has_cd = segs.iter().any(|t| is_cd(t));
    for tokens in &segs {
        let Some(operands) = rm_operands(tokens) else {
            continue;
        };
        for operand in operands {
            if let Some(why) = judge(operand, has_cd) {
                return Some(why);
            }
        }
    }
    None
}

/// Is this segment a `cd` (after any wrapper descent)?
fn is_cd(tokens: &[String]) -> bool {
    let start = skip_runner_prefix(tokens);
    tokens.get(start).map(String::as_str) == Some("cd")
}

/// One operand's verdict text, or None when it is legitimate work. Bare
/// `*` is judged from the workspace root ONLY when no cd moved first.
fn judge(operand: &str, has_cd: bool) -> Option<String> {
    if is_protected_root(operand) {
        return Some(format!(
            "`rm -rf {operand}` targets a protected root. System and home roots are never \
             build output; nothing legitimate deletes them."
        ));
    }
    if escapes_above(operand) {
        return Some(format!(
            "`rm -rf {operand}` escapes above the workspace (`..`). The parent tree is not \
             yours to delete — clean a NAMED subpath instead."
        ));
    }
    if let Some(var) = null_expansion(operand) {
        return Some(format!(
            "`rm -rf {operand}` hinges on `{var}`, which is NULL here — the shell would expand \
             it to nothing and the delete lands on `/` or the suffix alone. Hard-stopped before \
             execution."
        ));
    }
    if operand == "*" && !has_cd {
        return Some(
            "`rm -rf *` judged from the workspace root wipes the workspace. Name the subpaths \
             you mean."
                .to_string(),
        );
    }
    if operand == ".*" {
        return Some(
            "`rm -rf .*` matches `.` and `..` — the same escape the `..` rule denies, spelled \
             as a glob."
                .to_string(),
        );
    }
    None
}

/// SOFT: bare `*` under a directory the command cd'd into. The bare
/// `rm -rf *` (judged at the workspace root) is the HARD law's; this steer
/// never stacks on it.
pub fn wildcard(command: &str) -> Finding {
    let segs = segments(command);
    if !segs.iter().any(|t| is_cd(t)) {
        return None;
    }
    for tokens in &segs {
        if is_cd(tokens) {
            continue;
        }
        let Some(operands) = rm_operands(tokens) else {
            continue;
        };
        if operands.contains(&"*") {
            return Some(
                "`rm -rf *` inside a cd'd directory deletes EVERYTHING there, including files \
                 you did not mean. Name the subpaths."
                    .to_string(),
            );
        }
    }
    None
}
