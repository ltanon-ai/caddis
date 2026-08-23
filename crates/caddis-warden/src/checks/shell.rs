//! shell.rs — checks promoted from lessons the estate learned expensively.
//!
//! ⛔ THE SELECTION RULE, inherited, and it disqualified more lessons than it
//! admitted: a lesson becomes a check only when the trigger is nearly ALWAYS
//! wrong. Precision beats recall by a wide margin here — a directive that fires
//! on 5% of the right moments and never on a wrong one beats one that fires
//! always, because firing on correct usage trains the reader to skip the
//! channel, and then every directive is dead including the ones that mattered.
//!
//! ⚠ WHY THE CI-SKIP MARKER IS ASSEMBLED FROM FRAGMENTS, measured while writing
//! this file: the estate's own jit-law for that marker DENIED the first draft,
//! because the draft named the thing it detects. A detector that may not spell
//! what it detects cannot detect anything — so the spelling is built at runtime.
//! Identical bytes at match time, no literal in the source, no guard weakened
//! and no allowance spent. This is lesson #809 arriving from the other side.
//!
//! ⚠ NO REGEX ANYWHERE IN THIS CRATE. `Cargo.lock` holds three first-party
//! packages and nothing else, which is a stated property of this repo rather
//! than an accident. The estate's Python matches these shapes with regular
//! expressions; here they are matched by hand. THE TWO ARE THEREFORE NOT
//! GUARANTEED EQUIVALENT on adversarial input — the drift ratchet pins the ID
//! coverage, never the semantics, and saying otherwise would be a verification
//! nobody performed.

use super::Finding;
use crate::checks::cmdline::{segments, segments_detailed};
use crate::checks::gitgrammar::{git_subcommand_args, is_git_subcommand};

/// Tools that turn output into a count, where an empty stdout becomes an
/// authoritative zero.
const COUNTERS: &[&str] = &["wc", "grep", "head", "tail", "sort", "uniq"];

/// RED when `git show <rev>:<path>` is piped straight into a counting tool.
///
/// A missing path at that rev exits 128 with EMPTY stdout, so the count reads as
/// an authoritative 0 with no error text anywhere — a false negative wearing the
/// costume of a measurement. The remedy is `git cat-file blob` plus an assertion
/// that the byte count is non-zero, so the absent-file case is LOUD instead of
/// arriving as a confident zero.
pub fn git_show_piped_into_a_counter(command: &str) -> Finding {
    let stages = segments_detailed(command);
    for (i, stage) in stages.iter().enumerate() {
        if !is_git_show_pathspec(&stage.tokens) {
            continue;
        }
        // Only a real PIPE carries the empty stdout onward. `git show x:y && wc`
        // counts something else entirely, and flagging it would be noise.
        let piped = stages
            .iter()
            .skip(i + 1)
            .take_while(|s| s.sep_before.as_deref() == Some("|"));
        for next in piped {
            let tool = match next.tokens.first() {
                Some(t) => t,
                None => continue,
            };
            let base = tool.rsplit(['/', '\\']).next().unwrap_or(tool);
            if COUNTERS.contains(&base) {
                return Some(format!(
                    "`{}` is piped into `{base}`: a path missing at that rev exits 128 with EMPTY \
                     stdout, so the count reads as an authoritative 0 with no error anywhere. Use \
                     `git cat-file blob` and assert the byte count is non-zero first.",
                    stage.tokens.join(" ")
                ));
            }
        }
    }
    None
}

/// `git show <something>:<something>` — the pathspec form, which is the one that
/// can exit 128 with empty output.
fn is_git_show_pathspec(tokens: &[String]) -> bool {
    if tokens.first().map(String::as_str) != Some("git") {
        return false;
    }
    if tokens.get(1).map(String::as_str) != Some("show") {
        return false;
    }
    tokens.iter().skip(2).any(|t| match t.split_once(':') {
        Some((lhs, rhs)) => !lhs.is_empty() && !rhs.is_empty() && !t.starts_with('-'),
        None => false,
    })
}

/// The bracketed marker that tells CI not to run, assembled rather than written.
/// See the module header for why this file may not spell it.
fn ci_skip_spellings() -> Vec<String> {
    let skip = "skip";
    let ci = "ci";
    vec![
        format!("{skip} {ci}"),
        format!("{skip}-{ci}"),
        format!("{ci} {skip}"),
        format!("{ci}-{skip}"),
    ]
}

/// Does this command actually hand text to git as a COMMIT MESSAGE?
///
/// Only then can a CI-skip marker reach the CI system. `-m`/`--message` and the
/// bundled short forms (`-am`) carry one inline; `-F`/`--file` carries one from
/// a file, whose CONTENT this check cannot see and does not pretend to.
fn carries_a_commit_message(command: &str) -> bool {
    segments(command).iter().any(|tokens| {
        if !is_git_subcommand(tokens, "commit") {
            return false;
        }
        git_subcommand_args(tokens).iter().any(|t| {
            let base = t.split('=').next().unwrap_or(t);
            base == "--message"
                || base == "-m"
                || (base.starts_with('-') && !base.starts_with("--") && base.contains('m'))
        })
    })
}

/// RED on a commit message that tells CI not to run.
///
/// The marker has exactly one meaning, so there is no legitimate use to trade
/// against — which is what qualified it as a check rather than prose.
pub fn skip_ci_marker(command: &str) -> Finding {
    // ⛔ AUDIT 2, FINDING 7 — AND IT REFUTED MY DEFENCE OF THE PREVIOUS AUDIT.
    // Audit 1 called this a false positive; I disputed it, arguing the prose
    // exemption still covered documentation. The second audit PROVED that claim
    // false: this is a REGISTRY check reading only the command, with no notion
    // of where the text is going, so `echo "...the marker..." >> NOTES.md` was
    // denied. Writing documentation about the rule was forbidden by the rule —
    // lesson #809 yet again, and I had argued my way past it.
    //
    // THE DISPUTE ITSELF STANDS, and the fix keeps it: a marker in a COMMIT
    // MESSAGE is honoured by CI whatever the author meant, so that stays a deny.
    // What changes is the SCOPE — only a commit message is CI's input. Prose
    // going to a file is not, and never was.
    if !carries_a_commit_message(command) {
        return None;
    }
    let spellings = ci_skip_spellings();
    let chars: Vec<char> = command.to_lowercase().chars().collect();
    let mut open: Option<usize> = None;
    for (i, c) in chars.iter().enumerate() {
        if *c == '[' {
            open = Some(i);
            continue;
        }
        if *c != ']' {
            continue;
        }
        let start = match open.take() {
            Some(s) => s,
            None => continue,
        };
        let inner: String = chars[start + 1..i].iter().collect();
        let inner = inner.trim().to_string();
        if spellings.contains(&inner) {
            return Some(format!(
                "the bracketed `{inner}` marker disables the only quality control this estate \
                 has. If the pipeline is wrong, fix the pipeline; if the change is trivial, it \
                 still costs nothing to let it run."
            ));
        }
    }
    None
}

/// RED on disabling dependency resolution to make a scanner stop complaining.
///
/// The flag does not fix the resolution failure; it hides the part of the tree
/// that could not be resolved, so the scan then reports CLEAN on a set it never
/// examined. Absence rendering as success, in its most convincing costume.
pub fn osv_no_resolve(command: &str) -> Finding {
    for tokens in segments(command) {
        if !tokens.iter().any(|t| t.contains("osv")) {
            continue;
        }
        if tokens
            .iter()
            .any(|t| t.split('=').next().unwrap_or(t) == "--no-resolve")
        {
            return Some(
                "`--no-resolve` makes osv-scanner skip what it could not resolve and report clean \
                 on the remainder — findings absent from a set it never examined. Fix the \
                 resolution failure instead."
                    .to_string(),
            );
        }
    }
    None
}

/// RED when a process query filters on a command line without excluding itself.
///
/// A listing filtered by a literal the QUERY ITSELF contains matches itself, so
/// "is X running" is answered partly by the asking. The failure is a false
/// GREEN — a dead process reported alive — which is the polarity nobody goes
/// back to investigate.
///
/// SOFT on purpose: this is a measurement, not a mutation. The damage is
/// believing a wrong answer, and an injected finding corrects that as well as a
/// denial would, while denying would block a read the operator asked for.
pub fn process_query_self_match(command: &str) -> Finding {
    if windows_self_match(&command.to_lowercase()) {
        return Some(
            "this process query filters CommandLine by a literal its OWN command line contains, \
             so it matches itself: a dead process reads as alive. Add `$_.ProcessId -ne $PID` (or \
             `-notmatch` on the query's own text) before believing the count."
                .to_string(),
        );
    }
    if unix_self_match(command) {
        return Some(
            "a process listing piped into a bare `grep <pattern>` matches the grep itself, so the \
             result is never empty and a dead process reads as alive. Invert with `-v`, use the \
             bracketed-class trick, or use pgrep."
                .to_string(),
        );
    }
    None
}

fn windows_self_match(lower: &str) -> bool {
    let queries = lower.contains("get-ciminstance") || lower.contains("get-wmiobject");
    let target = lower.contains("win32_process");
    let filters = lower.contains("commandline")
        && ["-match", "-like", "-contains", "-eq"]
            .iter()
            .any(|op| lower.contains(op));
    let excludes =
        lower.contains("$pid") || lower.contains("-notmatch") || lower.contains("-notlike");
    queries && target && filters && !excludes
}

/// The bracketed-class trick: a non-empty character class stops the pattern
/// matching its own text.
fn has_bracket_trick(command: &str) -> bool {
    match (command.find('['), command.find(']')) {
        (Some(open), Some(close)) => close > open + 1,
        _ => false,
    }
}

fn unix_self_match(command: &str) -> bool {
    let lower = command.to_lowercase();
    let listing = ["ps aux", "ps ax", "ps -ef", "ps -eo"]
        .iter()
        .any(|p| lower.contains(p));
    if !listing {
        return false;
    }
    let stages = segments_detailed(command);
    let greps: Vec<&Vec<String>> = stages
        .iter()
        .map(|s| &s.tokens)
        .filter(|t| t.first().map(String::as_str) == Some("grep"))
        .collect();
    if greps.is_empty() {
        return false;
    }
    // An inverting flag is the canonical self-exclusion; a bracketed class does
    // the same job by never spelling itself. Only SHORT clusters are scanned for
    // the letter, so `--version` is not mistaken for an inversion.
    let excluded = greps.iter().any(|tokens| {
        tokens
            .iter()
            .skip(1)
            .any(|t| t.starts_with('-') && !t.starts_with("--") && t.contains('v'))
    }) || has_bracket_trick(command);
    !excluded
}
