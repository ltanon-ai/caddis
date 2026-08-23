//! gitgrammar.rs — what a token run MEANS to git, as opposed to what its words
//! are.
//!
//! Split out of `cmdline.rs` under the repo's 280-line file law, along the seam
//! the module already had: `cmdline` answers "which commands is this line", and
//! this file answers "is that command a git push, and where is its refspec".
//! Both of the audit findings that hit this area — a `-C` argument read as the
//! subcommand, and a flagged `sudo` prefix defeating the scan — were bugs in the
//! GRAMMAR, not in the lexing, which is why they belong behind one door.
//! The prefix half of that grammar (which commands wrap another command) now
//! lives in `runners.rs`, split under the same law when its registry grew.

use super::runners::skip_runner_prefix;

/// Git's own global options that consume the NEXT token as their value.
///
/// ⭐ WITHOUT THIS THE SUBCOMMAND SCAN READS AN OPTION'S ARGUMENT AS THE
/// SUBCOMMAND, and every git check silently stops seeing `git -C <path> push`.
/// Found by a test, not by reading: the estate's Python takes the first
/// non-dash token after `git`, which for `git -C /repo push --force origin main`
/// is `/repo` — so that command evades the force-push gate entirely while
/// looking exactly like the case the gate exists for. This estate reaches repos
/// with `git -C` constantly; the warden's own reset check is spelled that way.
/// A deny-class gate with a hole this ordinary reads as protection and is not.
const VALUE_TAKING: &[&str] = &[
    "-C",
    "-c",
    "--git-dir",
    "--work-tree",
    "--namespace",
    "--config-env",
    "--exec-path",
    // Audit finding 2b: a real two-token git global that was missing, so
    // `git --super-prefix /x push --force origin main` read as no push at all.
    "--super-prefix",
];

/// True when this segment invokes `git <subcommand>`.
///
/// Leading `VAR=value` assignments and a `sudo`/`env` prefix are skipped so the
/// check reads the command that will actually run rather than its decoration.
pub fn is_git_subcommand(tokens: &[String], subcommand: &str) -> bool {
    match git_subcommand_index(tokens) {
        Some(at) => tokens[at] == subcommand,
        None => false,
    }
}

/// The index of the subcommand token in `git [globals] <subcommand> ...`.
///
/// ONE function answers this for every caller. An earlier draft had the
/// subcommand test here and the argument slicing in the force-push check
/// re-deriving the same layout by counting positionals — and they disagreed the
/// moment a `-C <path>` shifted everything by one, which is the two-bookkeepings
/// defect this whole card exists to remove.
pub fn git_subcommand_index(tokens: &[String]) -> Option<usize> {
    let mut i = skip_runner_prefix(tokens);
    if i >= tokens.len() || tokens[i] != "git" {
        return None;
    }
    i += 1;
    while i < tokens.len() {
        let token = &tokens[i];
        if !token.starts_with('-') {
            return Some(i);
        }
        // `--git-dir=x` carries its value inline and consumes nothing extra.
        if VALUE_TAKING.contains(&token.as_str()) {
            i += 2;
            continue;
        }
        i += 1;
    }
    None
}

/// The tokens AFTER `git [globals] <subcommand>`, or an empty slice.
pub fn git_subcommand_args(tokens: &[String]) -> &[String] {
    match git_subcommand_index(tokens) {
        Some(at) => &tokens[at + 1..],
        None => &[],
    }
}

/// A flag present either as a long form or bundled into a short cluster.
///
/// Shared by the force and blanket-stage checks because both were otherwise
/// re-deriving "is this letter inside a cluster like `-uf`", and two copies of a
/// bypass-detection rule drift apart precisely where it matters.
pub fn has_short_or_long(tokens: &[String], letter: char, long_forms: &[&str]) -> bool {
    let short = format!("-{letter}");
    tokens.iter().any(|token| {
        let base = token.split('=').next().unwrap_or(token);
        if long_forms.contains(&base) || base == short {
            return true;
        }
        token.len() > 1
            && token.starts_with('-')
            && !token.starts_with("--")
            && token[1..].contains(letter)
    })
}

/// The first non-flag token after `git <subcommand>`, if any.
pub fn positional(tokens: &[String]) -> Vec<&String> {
    tokens.iter().filter(|t| !t.starts_with('-')).collect()
}
