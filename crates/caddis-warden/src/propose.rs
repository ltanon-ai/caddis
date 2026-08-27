//! propose.rs — laws discovered from history, scored before they bind
//! (CARD-0113, unit D).
//!
//! Mines the ledger for a signature nobody reads: a command ALLOWED and then
//! immediately UNDONE. That pair is the cheapest available evidence that a rule
//! is missing — the agent did something, and then it or its operator had to
//! take it back.
//!
//! ⛔ EVERY CANDIDATE SHIPS WITH ITS OWN FALSIFIER. A proposal states how many
//! rows across the WHOLE ledger a law on that signature would have denied, so
//! the reader sees the false-positive cost before adopting it rather than
//! after. A proposal without that number is an opinion.
//!
//! ⛔ NOTHING HERE INSTALLS A LAW. A conscience that writes its own rules
//! without a human reading them is a different and much larger decision than
//! this card, and it is not taken by omission.

use crate::rows::{first_line_capped, parse_row, split_body};
use std::collections::BTreeMap;

/// How many rows after an allow count as "immediately undone".
const LOOKAHEAD: usize = 8;

/// Command heads that TAKE SOMETHING BACK. Deliberately a short, literal list:
/// a broad heuristic would propose laws against ordinary work, and a proposal
/// nobody trusts is worse than none.
const UNDO_SHAPES: [&str; 6] = [
    "git reset",
    "git revert",
    "git checkout --",
    "git restore",
    "git stash drop",
    "git clean",
];

pub struct Candidate {
    pub signature: String,
    pub occurrences: u64,
    pub example_seq: u64,
    pub example: String,
    /// Rows across the WHOLE ledger a law on this signature would have denied.
    pub would_deny: u64,
}

pub struct Proposals {
    pub candidates: Vec<Candidate>,
    pub scanned: u64,
    pub unreadable: u64,
}

struct Cmd {
    seq: u64,
    from: String,
    tag: String,
    text: String,
    ts: u64,
}

fn is_undo(cmd: &str) -> bool {
    let c = cmd.trim();
    UNDO_SHAPES.iter().any(|u| c.starts_with(u))
}

/// The signature a candidate law would match: the first two words, which is
/// enough to separate `git commit` from `git push` without pinning a whole
/// command line that will never recur verbatim.
pub fn signature(cmd: &str) -> String {
    cmd.split_whitespace().take(2).collect::<Vec<_>>().join(" ")
}

/// Every command row, and how many lines could not be read. Split from `build`
/// when the gate measured CCN 15 against the cap of 10 — the fix is a split.
fn collect(text: &str) -> (Vec<Cmd>, u64) {
    let mut unreadable = 0u64;
    let mut cmds: Vec<Cmd> = Vec::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let Some(row) = parse_row(line) else {
            unreadable += 1;
            continue;
        };
        if let Some((tag, cmd)) = split_body(&row.body) {
            cmds.push(Cmd {
                seq: row.seq,
                from: row.from,
                tag,
                text: cmd,
                ts: row.ts,
            });
        }
    }
    (cmds, unreadable)
}

/// Is this row an allow, inside the window, that is not itself an undo?
fn minable(c: &Cmd, since_hours: Option<u64>, now: u64) -> bool {
    if c.tag != "allow" || c.text.trim().is_empty() {
        return false;
    }
    // An undo command is not itself evidence that IT needs a law; otherwise
    // every `git reset` after a `git reset` proposes a law against resetting.
    if is_undo(&c.text) || signature(&c.text).is_empty() {
        return false;
    }
    match since_hours {
        Some(h) => c.ts != 0 && now.saturating_sub(c.ts) <= h * 3600,
        None => true,
    }
}

pub fn build(text: &str, since_hours: Option<u64>, now: u64) -> Proposals {
    let (cmds, unreadable) = collect(text);
    // Scored over EVERY row, always — the whole point is what a candidate would
    // have cost across all of history, which a window cannot answer.
    let mut all_sig_counts: BTreeMap<String, u64> = BTreeMap::new();
    for c in &cmds {
        *all_sig_counts.entry(signature(&c.text)).or_default() += 1;
    }
    let mut found: BTreeMap<String, (u64, u64, String)> = BTreeMap::new();
    for (i, c) in cmds.iter().enumerate() {
        if !minable(c, since_hours, now) {
            continue;
        }
        let undone = cmds
            .iter()
            .skip(i + 1)
            .take(LOOKAHEAD)
            .any(|later| later.from == c.from && is_undo(&later.text));
        if !undone {
            continue;
        }
        let e = found
            .entry(signature(&c.text))
            .or_insert((0, c.seq, first_line_capped(&c.text)));
        e.0 += 1;
    }
    let mut candidates: Vec<Candidate> = found
        .into_iter()
        .map(
            |(signature, (occurrences, example_seq, example))| Candidate {
                would_deny: all_sig_counts.get(&signature).copied().unwrap_or(0),
                signature,
                occurrences,
                example_seq,
                example,
            },
        )
        .collect();
    // Strongest evidence first, then stable by name.
    candidates.sort_by(|a, b| {
        b.occurrences
            .cmp(&a.occurrences)
            .then_with(|| a.signature.cmp(&b.signature))
    });
    Proposals {
        candidates,
        scanned: cmds.len() as u64,
        unreadable,
    }
}

pub fn render_text(p: &Proposals) -> String {
    let mut s = format!("propose-laws: {} command row(s) scanned", p.scanned);
    if p.candidates.is_empty() {
        s.push_str("\nNO CANDIDATES — no allow-then-undo pattern in this window.");
        s.push_str("\nThat is a legitimate answer, not a failure to look.");
    }
    for c in &p.candidates {
        s.push_str(&format!(
            "\n\ncandidate: `{}`\n  seen undone {} time(s); example at seq {}: {}\n  \
             FALSIFIER: a law denying `{}` would have denied {} of {} recorded commands \
             ({:.1}%) — read that cost BEFORE adopting it",
            c.signature,
            c.occurrences,
            c.example_seq,
            c.example,
            c.signature,
            c.would_deny,
            p.scanned,
            if p.scanned == 0 {
                0.0
            } else {
                (c.would_deny as f64) * 100.0 / (p.scanned as f64)
            }
        ));
    }
    s.push_str(&format!(
        "\n\n{} ledger line(s) unreadable. Nothing here installs a law: these are \
         candidates for a human to rule on.",
        p.unreadable
    ));
    s
}

pub fn render_json(p: &Proposals) -> String {
    let body: Vec<String> = p
        .candidates
        .iter()
        .map(|c| {
            format!(
                "{{\"signature\":\"{}\",\"occurrences\":{},\"example_seq\":{},\
                 \"example\":\"{}\",\"would_deny\":{}}}",
                crate::wire::json_escape(&c.signature),
                c.occurrences,
                c.example_seq,
                crate::wire::json_escape(&c.example),
                c.would_deny
            )
        })
        .collect();
    format!(
        "{{\"scanned\":{},\"unreadable\":{},\"candidates\":[{}]}}",
        p.scanned,
        p.unreadable,
        body.join(",")
    )
}

/// The ledger text, or `None` after reporting why not. Shared with `laws` so
/// both subcommands fail the same way for the same reason.
pub fn read_ledger(who: &str) -> Option<String> {
    let path = crate::identity::ledger_path()
        .to_string_lossy()
        .into_owned();
    match std::fs::read_to_string(&path) {
        Ok(t) => Some(t),
        // AN ABSENT LEDGER IS AN EMPTY ONE. Nothing has been recorded yet, which
        // is a legitimate state on a fresh install and must not read as a
        // failure. `card status` has always treated it this way; these
        // subcommands did not, and a first-run user met an error instead of a
        // report.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Some(String::new()),
        Err(e) => {
            // A ledger that EXISTS and cannot be read is a different fact, and
            // it stays an error: "nothing happened" and "I could not look" must
            // never print the same.
            eprintln!("{who}: cannot read {path}: {e}");
            None
        }
    }
}

pub fn run(args: &[String]) -> i32 {
    let f = crate::receipt::parse_filters(&args[2.min(args.len())..]);
    let Some(text) = read_ledger("propose-laws") else {
        return 2;
    };
    let p = build(&text, f.since_hours, crate::identity::unix_seconds());
    if args.iter().any(|a| a == "--json") {
        println!("{}", render_json(&p));
    } else {
        println!("{}", render_text(&p));
    }
    0
}

#[cfg(test)]
#[path = "propose_tests.rs"]
mod tests;
