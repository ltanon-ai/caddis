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

/// gemini's deny set, in NORMALIZED form (backslashes folded, trailing
/// slash trimmed, the root spelled `/`).
const PROTECTED_ROOTS: &[&str] = &[
    "/", "/home", "/usr", "/var", "/etc", "/boot", "/root", "/opt", "/bin", "/lib", "/Users",
    "C:", "%userprofile%", "%systemdrive%", "%homepath%",
];

/// Variables that are protected AS WHOLE TOKENS: `$HOME` deletes the
/// user's entire home when the shell expands it — the expansion IS the
/// root. `$HOME/cache` is an ordinary path after expansion, not this.
const PROTECTED_VARS: &[&str] = &["HOME", "PWD"];

/// One rm invocation's operands, after wrapper descent.
fn rm_operands(tokens: &[String]) -> Option<Vec<&str>> {
    let start = skip_runner_prefix(tokens);
    let head = tokens.get(start)?;
    let head = head.strip_suffix(".exe").unwrap_or(head);
    if head != "rm" {
        return None;
    }
    let tail: &[String] = &tokens[start + 1..];
    let (mut r, mut f) = (false, false);
    for t in tail {
        if let Some(long) = t.strip_prefix("--") {
            let base = long.split('=').next().unwrap_or(long);
            if base == "recursive" {
                r = true;
            } else if base == "force" {
                f = true;
            }
        } else if let Some(short) = t.strip_prefix('-') {
            for c in short.chars() {
                if c == 'r' || c == 'R' {
                    r = true;
                } else if c == 'f' {
                    f = true;
                }
            }
        }
    }
    if !r || !f {
        return None;
    }
    Some(
        tail.iter()
            .filter(|t| !t.starts_with('-'))
            .map(String::as_str)
            .collect(),
    )
}

/// Normalize for deny-set comparison: fold separators, trim trailing
/// slashes; a token that is ALL slashes is the root itself.
fn norm(t: &str) -> String {
    let folded = t.replace('\\', "/");
    let trimmed = folded.trim_end_matches('/');
    if trimmed.is_empty() {
        "/".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Windows-shaped tokens (%VAR%, drive roots) compare case-insensitively —
/// that is how the expanding shell treats them.
fn same_token(a: &str, b: &str) -> bool {
    let windows_shaped = |s: &str| s.contains('%') || (s.len() == 2 && s.ends_with(':'));
    if windows_shaped(a) || windows_shaped(b) {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

/// The variable name when the token is EXACTLY a variable (`$HOME`,
/// `${HOME}`) with no path suffix.
fn whole_var(operand: &str) -> Option<String> {
    let valid = |name: &str| {
        (!name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
            .then(|| name.to_string())
    };
    if let Some(inner) = operand.strip_prefix("${").and_then(|r| r.strip_suffix('}')) {
        return valid(inner);
    }
    operand.strip_prefix('$').and_then(valid)
}

/// Is this operand an exact protected root, literally or as a whole-token
/// variable?
fn is_protected_root(operand: &str) -> bool {
    let n = norm(operand);
    if PROTECTED_ROOTS.iter().any(|p| same_token(&n, p)) {
        return true;
    }
    whole_var(operand)
        .map(|name| PROTECTED_VARS.contains(&name.as_str()))
        .unwrap_or(false)
}

/// Does the path walk OUT of the workspace (above cwd)? Lexical, no
/// filesystem touch: a `..` before enough named parts drops below start.
fn escapes_above(operand: &str) -> bool {
    let mut depth: i32 = 0;
    for part in operand.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                depth -= 1;
                if depth < 0 {
                    return true;
                }
            }
            _ => depth += 1,
        }
    }
    false
}


/// A `$`-token's (name, suffix) in both spellings: `$VAR/x` and
/// `${VAR}/x` both yield ("VAR", "/x").
fn split_var(operand: &str) -> Option<(String, String)> {
    if let Some(inner) = operand.strip_prefix("${") {
        let end = inner.find('}')?;
        let (name, rest) = inner.split_at(end);
        return Some((name.to_string(), rest[1..].to_string()));
    }
    let body = operand.strip_prefix('$')?;
    let end = body
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(body.len());
    let (name, rest) = body.split_at(end);
    Some((name.to_string(), rest.to_string()))
}

/// A `$`-token whose variable is NULL in this environment: the shell will
/// expand it to nothing, and `rm -rf $VAR/build` becomes `rm -rf /build`
/// — the ruling hard-denies it before execution. A DEFINED variable is
/// judged as its VALUE (one level; the value is data, not a nested var).
fn null_expansion(operand: &str) -> Option<String> {
    let (var, suffix) = split_var(operand)?;
    if var.is_empty() {
        return None;
    }
    match std::env::var(&var) {
        Ok(value) => {
            let expanded = format!("{value}{suffix}");
            if is_protected_root(&expanded) || escapes_above(&expanded) {
                Some(expanded)
            } else {
                None
            }
        }
        Err(_) => Some(var),
    }
}

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
