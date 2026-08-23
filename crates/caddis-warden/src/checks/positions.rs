//! positions.rs — commands hiding in positions a segment split cannot see.
//!
//! HIGH 4, audit 4 — a different class from the runner list. `$( … )` and
//! backticks run a command from inside a TOKEN; `bash -c '…'`, `eval …` and
//! the string-carrying runner flags (`flock -c`, `runuser -c`, `env -S`) run
//! one from inside a STRING. All are extracted here, bounded: one carrier
//! level composing with one substitution level. Deeper nesting is a parser's
//! job, not a vocabulary gate's. (The keyword half of HIGH 4 — `then`/`if`/`!`
//! at a command position — lives in `runners.rs` with the other decoration.)
//!
//! ⛔ Warden16to19Reviewer measured six false allows at fceb94e, every one a
//! real force-push in a scratch repo: unquoted `eval` (its argument list IS
//! the command line), the string flags above (the registry named them and
//! nothing consumed them), `/bin/bash -c` (path prefix), `bash -c $'…'`
//! (the `$` poisoned the re-lexed token), carrier × substitution, and a
//! fully escaped-space argument (`sh -c git\ push\ …`) that the pinned v4
//! escape ruling renders as space-free tokens. All six are closed here.

use super::cmdline::{strict_union, Segment};
use super::runners::split_prefix;

/// Shells and `eval`: commands whose argument is a whole command line.
const CARRIERS: &[&str] = &["bash", "sh", "dash", "zsh", "ksh", "eval"];

/// The carrier identity of a command word, if any.
///
/// An ABSOLUTE path is an identity claim: `/bin/bash` and
/// `C:/…/bash.exe` resolve to their basename, because that is the same
/// binary the bare name invokes. A RELATIVE path (`./bash`) is not a claim —
/// it could be any program — and stays unrecognised rather than guessed.
fn carrier_of(token: &str) -> Option<&'static str> {
    // The `.exe` suffix is Windows spelling, not a different program:
    // `bash.exe` is bash — bare (PATH resolution, the estate's native
    // spelling) or path-prefixed alike.
    let token = token.strip_suffix(".exe").unwrap_or(token);
    let absolute = token.starts_with('/') || token.starts_with('\\') || {
        let b = token.as_bytes();
        b.len() >= 3
            && b[0].is_ascii_alphabetic()
            && b[1] == b':'
            && (b[2] == b'/' || b[2] == b'\\')
    };
    if !absolute {
        return CARRIERS.iter().find(|c| **c == token).copied();
    }
    let base = token.rsplit(['/', '\\']).next().unwrap_or(token);
    CARRIERS.iter().find(|c| **c == base).copied()
}

/// Segments carried inside strings: shell/eval arguments, and the values of
/// string-carrying runner flags, for any base segment.
pub(super) fn carrier_segments(base: &[Segment]) -> Vec<Segment> {
    let mut out = Vec::new();
    for seg in base {
        let (at, carried) = split_prefix(&seg.tokens);
        // A runner flag whose value is an executed command string. env's -S
        // is the exception: it EXECS the split string directly, no shell —
        // separators are argv text there, not operators.
        for (runner, s) in &carried {
            if *runner == "env" {
                out.extend(argv_segments(s));
            } else {
                out.extend(string_segments(s));
            }
        }
        let Some(carrier) = seg.tokens.get(at).and_then(|t| carrier_of(t)) else {
            continue;
        };
        let args = &seg.tokens[at + 1..];
        if carrier == "eval" {
            // eval's argument list IS the command line, quoted or not: bash
            // joins the words and executes the result.
            let joined = args.join(" ");
            if !joined.trim().is_empty() {
                out.extend(string_segments(&joined));
            }
        } else if let Some(script) = script_argument(args) {
            out.extend(string_segments(&script));
        }
    }
    out
}

/// The argv form: `env -S` splits the string and EXECS it directly — no
/// shell, so separators are literal argv characters, not operators. Words
/// group by whitespace with one layer of quoting honoured. Judging the
/// string as a shell line denied `env -S 'echo a; git push …'` while
/// nothing pushy ran (Warden16to19Reviewer, re-measure).
fn argv_segments(s: &str) -> Vec<Segment> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for c in s.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => cur.push(c),
            None if c == '\'' || c == '"' => quote = Some(c),
            None if c.is_whitespace() => {
                if !cur.is_empty() {
                    words.push(std::mem::take(&mut cur));
                }
            }
            None => cur.push(c),
        }
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    if words.is_empty() {
        Vec::new()
    } else {
        vec![Segment {
            sep_before: None,
            tokens: words,
        }]
    }
}

/// The script argument of a shell carrier, if the `-c` form is present.
///
/// Without a `-c` (or a cluster containing it) the first argument is a SCRIPT
/// FILE, not a command line — `bash git push` runs the FILE `git`, and the
/// reviewer measured that no-push spelling as correctly allowed. With it, the
/// first non-flag argument is the script — including a fully escaped-space
/// run (`git\ push\ --force\ …`), re-joined into the one argument bash
/// assembles.
fn script_argument(args: &[String]) -> Option<String> {
    let mut idx = 0;
    let mut has_c = false;
    while idx < args.len() && args[idx].starts_with('-') && args[idx].len() > 1 {
        has_c = has_c || args[idx].contains('c');
        idx += 1;
    }
    if !has_c {
        return None;
    }
    let first = args.get(idx)?;
    if !first.ends_with('\\') {
        return (first.contains(' ')).then(|| first.clone());
    }
    // Each token ending in `\` glues to the next with a space, exactly as
    // bash assembles the escaped word. The run ends at the first bare token.
    let mut joined = String::new();
    for tok in &args[idx..] {
        match tok.strip_suffix('\\') {
            Some(head) => {
                joined.push_str(head);
                joined.push(' ');
            }
            None => {
                joined.push_str(tok);
                return Some(joined);
            }
        }
    }
    Some(joined.trim_end().to_string())
}

/// Segments hidden inside `$( … )` or backtick command substitution.
///
/// Bash runs substitution however it is quoted EXCEPT inside SINGLE quotes,
/// where `'$(…)'` is literal text — so the scan tracks single-quote state and
/// extracts everything else. Extracting from a single-quoted message would
/// fabricate a command out of prose (`git commit -m '$(git push …)'` commits
/// a MESSAGE).
pub(super) fn substitution_segments(command: &str) -> Vec<Segment> {
    let mut out = Vec::new();
    for inner in substitutions(command) {
        out.extend(strict_union(&inner));
    }
    out
}

/// A re-lexed command string judged together with its OWN substitutions —
/// one carrier level composing with one substitution level, the composition
/// the reviewer measured (`bash -c 'echo $(git push …)'`). A carrier inside
/// the substitution stays out: that is the documented depth bound.
fn string_segments(s: &str) -> Vec<Segment> {
    let mut out = strict_union(s);
    for sub in substitutions(s) {
        out.extend(strict_union(&sub));
    }
    out
}

/// The text of every command substitution span, single-quote aware.
fn substitutions(command: &str) -> Vec<String> {
    let chars: Vec<char> = command.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let mut in_single = false;
    while i < chars.len() {
        let c = chars[i];
        if in_single {
            if c == '\'' {
                in_single = false;
            }
            i += 1;
            continue;
        }
        match c {
            '\'' => in_single = true,
            '`' => {
                if let Some(end) = chars[i + 1..].iter().position(|&x| x == '`') {
                    let span = chars[i + 1..i + 1 + end].iter().collect::<String>();
                    out.push(span);
                    i = i + 1 + end + 1;
                    continue;
                }
            }
            '$' if chars.get(i + 1) == Some(&'(') => {
                if let Some(close) = matching_paren(&chars, i + 1) {
                    let span = chars[i + 2..close].iter().collect::<String>();
                    out.push(span);
                    i = close + 1;
                    continue;
                }
            }
            _ => {}
        }
        i += 1;
    }
    out
}

/// Index of the `)` matching the `(` at `open`, nesting-aware, or `None`.
fn matching_paren(chars: &[char], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (i, &c) in chars.iter().enumerate().skip(open) {
        if c == '(' {
            depth += 1;
        } else if c == ')' {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}
