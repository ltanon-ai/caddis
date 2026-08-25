//! rmrf_operand.rs — what an `rm -rf` operand IS, separately from what the law
//! makes of it.
//!
//! SPLIT FROM `rmrf.rs` (CARD-0104): that file reached 282 lines against a 280
//! cap and carried a function over the cognitive-complexity cap, and the two
//! concerns were already distinct — recognising an operand's SHAPE (flags,
//! normalisation, variable spellings, whether a path escapes upward) versus
//! DECIDING what to do about it. The doctrine's answer to an oversized file is
//! a split, never trimmed comments.
//!
//! No regex anywhere in this crate (zero-dependency law): hand matching.

use super::runners::skip_runner_prefix;

/// gemini's deny set, in NORMALIZED form (backslashes folded, trailing
/// slash trimmed, the root spelled `/`).
pub(crate) const PROTECTED_ROOTS: &[&str] = &[
    "/",
    "/home",
    "/usr",
    "/var",
    "/etc",
    "/boot",
    "/root",
    "/opt",
    "/bin",
    "/lib",
    "/Users",
    "C:",
    "%userprofile%",
    "%systemdrive%",
    "%homepath%",
];

/// Variables that are protected AS WHOLE TOKENS: `$HOME` deletes the
/// user's entire home when the shell expands it — the expansion IS the
/// root. `$HOME/cache` is an ordinary path after expansion, not this.
pub(crate) const PROTECTED_VARS: &[&str] = &["HOME", "PWD"];

/// Which of `-r`/`-R` and `-f` this invocation carries, in every spelling:
/// clustered (`-rf`), separate (`-r -f`), long (`--recursive --force`) and
/// long-with-value (`--force=...`).
///
/// SPLIT OUT ON PURPOSE: the three nested shapes — long form, short cluster,
/// per-character scan — are what drove `rm_operands` past the cognitive
/// complexity cap. The logic was never complicated; the nesting was.
pub(crate) fn recursive_and_force(tail: &[String]) -> (bool, bool) {
    let (mut recursive, mut force) = (false, false);
    for token in tail {
        if let Some(long) = token.strip_prefix("--") {
            match long.split('=').next().unwrap_or(long) {
                "recursive" => recursive = true,
                "force" => force = true,
                _ => {}
            }
        } else if let Some(cluster) = token.strip_prefix('-') {
            recursive |= cluster.contains('r') || cluster.contains('R');
            force |= cluster.contains('f');
        }
    }
    (recursive, force)
}

/// One rm invocation's operands, after wrapper descent.
pub(crate) fn rm_operands(tokens: &[String]) -> Option<Vec<&str>> {
    let start = skip_runner_prefix(tokens);
    let head = tokens.get(start)?;
    let head = head.strip_suffix(".exe").unwrap_or(head);
    if head != "rm" {
        return None;
    }
    let tail: &[String] = &tokens[start + 1..];
    let (recursive, force) = recursive_and_force(tail);
    if !recursive || !force {
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
pub(crate) fn norm(t: &str) -> String {
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
pub(crate) fn same_token(a: &str, b: &str) -> bool {
    let windows_shaped = |s: &str| s.contains('%') || (s.len() == 2 && s.ends_with(':'));
    if windows_shaped(a) || windows_shaped(b) {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

/// The variable name when the token is EXACTLY a variable (`$HOME`,
/// `${HOME}`) with no path suffix.
pub(crate) fn whole_var(operand: &str) -> Option<String> {
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
pub(crate) fn is_protected_root(operand: &str) -> bool {
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
pub(crate) fn escapes_above(operand: &str) -> bool {
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
pub(crate) fn split_var(operand: &str) -> Option<(String, String)> {
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
pub(crate) fn null_expansion(operand: &str) -> Option<String> {
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
