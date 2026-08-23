//! runners.rs — walking a segment's prefix past everything that is provably
//! DECORATION rather than the command itself.
//!
//! Split out of `gitgrammar.rs` under the 280-line file law, later split
//! again when the registry table outgrew it (the table lives in
//! `registry.rs`). What remains here is the walk: assignments, runner
//! prefixes with their flags and values, leading operands, and the shell
//! grammar words that occupy the command position without being the command.

use super::registry::{runner_name, runner_spec};

/// Shell grammar words that occupy the COMMAND POSITION without being the
/// command themselves. `then git push …` runs git push; `if git push …; then`
/// runs it as the condition; `! git push …` runs it negated. They are skipped
/// like assignments — provable decoration — and only ever at a segment's
/// start, so `echo then git push` is untouched (there the word is echo's
/// argument). HIGH 4, audit 4: this class is NOT the runner list, and the
/// audit measured `( cd repo && git push )` denying while `then git push`
/// allowed — inconsistent rather than uniformly conservative.
const KEYWORDS: &[&str] = &[
    "if", "then", "else", "elif", "while", "until", "do", "{", "!",
];

/// Runners whose FIRST positional argument is their own operand, not the
/// command: `timeout 30 git ...` (the duration), `chrt -f 10 git ...` (the
/// priority), `flock /tmp/lk git ...` (the lock file).
///
/// `nice` is deliberately NOT here: no real nice (GNU, POSIX, BSD) accepts a
/// positional adjustment — `nice 10 git push` runs the command `10` — so the
/// old skip denied lines that never ran git at all.
fn takes_leading_operand(runner: &str) -> bool {
    matches!(runner, "timeout" | "chrt" | "flock")
}

/// Does this flag TOKEN consume the NEXT token as its value?
///
/// Yes when it is exactly a listed value flag (`-u root`), or a short CLUSTER
/// whose LAST character is a listed short value flag — getopt's rule, so
/// `-Eu root` reads `-E` + `-u` + value. A value flag EARLIER in the cluster
/// must carry its value inside the token (`-uE`, `-o0`, `-k5`), which
/// consumes nothing after: treating those as value-taking would eat the
/// command word and open the mirror hole. Measured by the CARD-WARDEN-11
/// reviewer after my "taken from each tool's real options" claim proved
/// narrower than the real tools.
fn consumes_next(raw: &str, value_flags: &[&str]) -> bool {
    if value_flags.contains(&raw) {
        return true;
    }
    if raw.starts_with("--") {
        return false; // long options never cluster
    }
    match raw.chars().skip(1).last() {
        Some(last) => value_flags.contains(&format!("-{last}").as_str()),
        None => false,
    }
}

/// Advance past ONE runner and report where the real command starts, plus the
/// values consumed for its STRING-CARRYING flags (a `-c`-style flag whose
/// value is an executed command line).
///
/// Flags are walked again after the leading operand, because util-linux
/// getopt PERMUTES: `flock /tmp/lk -c '…'` is legal and the string flag sits
/// after the lock file. The re-walk stops at the first non-flag word, which
/// is where the wrapped command begins.
fn skip_one_runner(
    tokens: &[String],
    start: usize,
    runner: &'static str,
    strings: &mut Vec<(&'static str, String)>,
) -> usize {
    let (value_flags, string_flags) = runner_spec(runner).unwrap_or((&[], &[]));
    let mut i = start + 1;
    i = walk_flags(tokens, i, runner, value_flags, string_flags, strings);
    // `timeout 30 git ...` — the duration is the runner's own operand, not
    // the command being wrapped.
    let operand_next = i < tokens.len() && !tokens[i].starts_with('-') && tokens[i] != "git";
    if takes_leading_operand(runner) && operand_next {
        i += 1;
        i = walk_flags(tokens, i, runner, value_flags, string_flags, strings);
    }
    i
}

/// One pass over a runner's flags: skip booleans, consume values, and collect
/// the values of string-carrying flags for re-lexing.
fn walk_flags(
    tokens: &[String],
    from: usize,
    runner: &'static str,
    value_flags: &[&'static str],
    string_flags: &[&'static str],
    strings: &mut Vec<(&'static str, String)>,
) -> usize {
    let mut i = from;
    while i < tokens.len() && tokens[i].starts_with('-') {
        let raw = tokens[i].clone();
        i += 1;
        // LONG options carry getopt's own rules: unambiguous abbreviations
        // resolve, inline `=` values collect, and the glued `--cmd'x'` form
        // is rejected (a long value needs `=`; attached is a SHORT rule).
        if let Some(long) = raw.strip_prefix("--") {
            i = long_flag(long, tokens, i, runner, value_flags, string_flags, strings);
            continue;
        }
        // Attached short value, with getopt's cluster rule: a BOOLEAN run
        // before the string flag is legal (`-iS'git push …'`).
        if let Some(value) = attached_value(&raw, value_flags, string_flags) {
            strings.push((runner, value));
            continue;
        }
        if !consumes_next(&raw, value_flags) || i >= tokens.len() {
            continue;
        }
        if string_flags.contains(&raw.as_str()) {
            strings.push((runner, tokens[i].clone()));
        }
        i += 1;
    }
    i
}

/// getopt_long's unambiguous-abbreviation rule: a prefix matches when
/// exactly ONE listed long option begins with it (`--comma` → `--command`
/// for flock — the reviewer measured the real tool running it). An empty or
/// ambiguous prefix matches nothing, exactly as getopt errors on it.
fn resolve_long(prefix: &str, flags: &[&'static str]) -> Option<&'static str> {
    if prefix.is_empty() {
        return None;
    }
    let mut matches = flags
        .iter()
        .filter_map(|f| f.strip_prefix("--").map(|long| (*f, long)))
        .filter(|(_, long)| long.starts_with(prefix))
        .map(|(f, _)| f);
    let first = matches.next()?;
    matches.next().is_none().then_some(first)
}

/// One long-option token (the `--` already stripped): inline `=` values
/// collect for string flags; a resolved value flag consumes the next token,
/// collected when it is a string flag. Unknown or ambiguous → nothing.
fn long_flag(
    long: &str,
    tokens: &[String],
    i: usize,
    runner: &'static str,
    value_flags: &[&'static str],
    string_flags: &[&'static str],
    strings: &mut Vec<(&'static str, String)>,
) -> usize {
    if let Some((prefix, value)) = long.split_once('=') {
        if resolve_long(prefix, string_flags).is_some() {
            strings.push((runner, value.to_string()));
        }
        return i;
    }
    let Some(flag) = resolve_long(long, value_flags) else {
        return i;
    };
    let mut next = i;
    if next < tokens.len() {
        if string_flags.contains(&flag) {
            strings.push((runner, tokens[next].clone()));
        }
        next += 1;
    }
    next
}

/// The value glued to a SHORT string flag, measured against real getopt
/// (the tool): a run of BOOLEAN letters before the string flag is legal —
/// `-iS'git push …'` is `-i` + `-S` + value, and env executes the string.
/// getopt's walk gives the FIRST value-taking letter the rest of the
/// cluster, so if a VALUE flag precedes the string flag, that flag owns the
/// remainder and nothing is collected (`flock -Ec'…'` → -E takes `c'…'`;
/// no -c exists, the line does nothing). Letters that are neither are
/// boolean for this purpose. Long flags are `long_flag`'s business — their
/// glued form is not a spelling.
fn attached_value(raw: &str, value_flags: &[&str], string_flags: &[&str]) -> Option<String> {
    let body = raw.strip_prefix('-')?;
    for (idx, letter) in body.char_indices() {
        let short = format!("-{letter}");
        if string_flags.contains(&short.as_str()) {
            let rest = &body[idx + letter.len_utf8()..];
            return (!rest.is_empty()).then(|| rest.to_string());
        }
        if value_flags.contains(&short.as_str()) {
            return None;
        }
    }
    None
}
/// Split a token list into (index of the first real command word, the command
/// strings its prefix carried — tagged with the runner that carried them).
/// ONE walk answers for every caller. The tag exists because string
/// SEMANTICS differ per runner: `flock -c` and `runuser -c` hand the string
/// to a shell; `env -S` execs the split string directly, no shell.
pub(super) fn split_prefix(tokens: &[String]) -> (usize, Vec<(&'static str, String)>) {
    let mut i = 0;
    let mut strings = Vec::new();
    while i < tokens.len() {
        let token = tokens[i].as_str();
        if token.contains('=') || KEYWORDS.contains(&token) {
            i += 1;
            continue;
        }
        if let Some(name) = runner_name(token) {
            i = skip_one_runner(tokens, i, name, &mut strings);
            continue;
        }
        break;
    }
    (i, strings)
}

/// Index of the first token that is not environment decoration.
///
/// Skips `VAR=value` assignments, shell grammar keywords, and runner prefixes
/// WITH their own flags. It deliberately does NOT scan forward for the word
/// `git`: that would make `sudo docker run --rm git push --force origin main`
/// read as a git push, when `git` there is an ARGUMENT to docker. The skip
/// advances only over things that are provably decoration — an assignment, a
/// keyword, a runner, a runner's flag, or that flag's value — and stops at
/// the first real command word.
pub(super) fn skip_runner_prefix(tokens: &[String]) -> usize {
    split_prefix(tokens).0
}
