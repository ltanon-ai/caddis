//! replay.rs — the ledger as a simulator (CARD-REPLAY-1).
//!
//! `caddis-warden --replay <ledger>` re-judges every recorded command
//! against the CURRENT law and reports the diff. This is the cherry the
//! ledger was always shaped for: a law change you can preview against your
//! own history before it ever guards a live agent — "this new law would
//! have denied 3 of last night's 2142 calls; here they are". It kills both
//! adoption fears at once (false positives at install, regressions at
//! update) and it is only possible because the memory is attributed,
//! deterministic, and complete for command-bearing calls.
//!
//! HONEST LIMITS, stated where a reader meets them:
//! - Only what the ledger KEPT can be replayed. Masked (`***redacted`) and
//!   elided (`bytes truncated]`) commands are SKIPPED and counted, never
//!   guessed — the secrets/size doctrines outrank simulation fidelity.
//! - Non-command tools (write/edit) stored no content by design: skipped.
//! - Directory-sensitive laws (git state, incident history) are judged from
//!   the directory you run replay in, not from the agent's old cwd.
//! - The report never changes anything; replay is read-only by construction.

use crate::rows::{first_line_capped, law_id_bracketed, parse_row, split_body, Row};
use caddis_warden::{checks, decide, ToolCall, Verdict};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;


/// One row's re-judgement outcome.
enum Outcome {
    Unchanged,
    NewDeny,
    Freed,
    /// allow<->steer drift: counted, not itemized
    Note,
    Skipped,
}

/// Re-judge a single row against the current law. Skips what the ledger
/// deliberately never kept (masked, elided, non-command tools) rather
/// than guessing — the secrets and size doctrines outrank fidelity.
/// REPLAY-COUNTS-1: also returns the law ids the CURRENT verdict fired
/// (deny id, or every steer id) so the digest can count coverage — one
/// `decide`, both facts, never a second judgement.
fn classify(row: &Row) -> (Outcome, Vec<(&'static str, String)>) {
    if row.tool != "tool.bash" && row.tool != "tool.powershell" {
        return (Outcome::Skipped, Vec::new());
    }
    let Some((old_tag, cmd)) = split_body(&row.body) else {
        return (Outcome::Skipped, Vec::new());
    };
    if cmd.contains("***redacted") || cmd.contains("bytes truncated]") {
        return (Outcome::Skipped, Vec::new());
    }
    let call = ToolCall::new(&row.tool["tool.".len()..]).command(&cmd);
    let verdict = decide(&call);
    let fires = fired_ids(&verdict);
    let new_tag = match &verdict {
        Verdict::Allow => "allow",
        Verdict::Steer { .. } => "steer",
        Verdict::Deny { .. } => "deny",
    };
    let outcome = if new_tag == old_tag {
        Outcome::Unchanged
    } else if new_tag == "deny" {
        Outcome::NewDeny
    } else if old_tag == "deny" {
        Outcome::Freed
    } else {
        Outcome::Note
    };
    (outcome, fires)
}

/// The law ids one verdict fired: the bracketed id in a deny reason, or
/// every id the steer's why field carries.
fn fired_ids(verdict: &Verdict) -> Vec<(&'static str, String)> {
    match verdict {
        Verdict::Deny { reason } => law_id_bracketed(reason)
            .map(|id| vec![("deny", id)])
            .unwrap_or_default(),
        Verdict::Steer { why, .. } => why
            .split(", ")
            .map(|id| ("steer", id.to_string()))
            .collect(),
        Verdict::Allow => Vec::new(),
    }
}

/// Optional narrowing: one caller (`--from`), a recency window in hours
/// (`--since`). Users should not have to memorize slicing pipelines.
struct Filters {
    from: Option<String>,
    since_hours: Option<u64>,
}

fn parse_filters(args: &[String]) -> Filters {
    let mut f = Filters {
        from: None,
        since_hours: None,
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--from" => f.from = it.next().cloned(),
            "--since" => f.since_hours = it.next().and_then(|v| v.parse().ok()),
            _ => {}
        }
    }
    f
}

impl Filters {
    fn admits(&self, row: &Row) -> bool {
        if let Some(from) = &self.from {
            if &row.from != from {
                return false;
            }
        }
        if let Some(hours) = self.since_hours {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            if row.ts == 0 || now.saturating_sub(row.ts) > hours * 3600 {
                return false;
            }
        }
        true
    }
}

/// The whole replay: read-only re-judgement of one ledger.
pub fn run(args: &[String]) -> i32 {
    let Some(path) = args.get(2) else {
        eprintln!("usage: caddis-warden --replay <ledger.jsonl> [--from name] [--since hours]");
        return 2;
    };
    let text = match std::fs::read_to_string(Path::new(path)) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("replay: cannot read {path}: {e}");
            return 2;
        }
    };
    let (mut rows, mut judged, mut unchanged) = (0u64, 0u64, 0u64);
    let (mut denies, mut freed, mut skipped) = (0u64, 0u64, 0u64);
    let mut news: Vec<String> = Vec::new();
    let mut frees: Vec<String> = Vec::new();
    let mut deny_fires: BTreeMap<String, u64> = BTreeMap::new();
    let mut steer_fires: BTreeMap<String, u64> = BTreeMap::new();
    let filters = parse_filters(args);
    for line in text.lines() {
        let Some(row) = parse_row(line) else {
            continue;
        };
        if !filters.admits(&row) {
            continue;
        }
        rows += 1;
        let head = first_line_capped(&split_body(&row.body).map(|(_, c)| c).unwrap_or_default());
        let (outcome, fires) = classify(&row);
        for (kind, id) in fires {
            let slot = if kind == "deny" {
                &mut deny_fires
            } else {
                &mut steer_fires
            };
            *slot.entry(id).or_default() += 1;
        }
        match outcome {
            Outcome::Skipped => skipped += 1,
            Outcome::Unchanged | Outcome::Note => {
                judged += 1;
                unchanged += 1;
            }
            Outcome::NewDeny => {
                judged += 1;
                denies += 1;
                news.push(format!("NEW-DENY seq={} {}", row.seq, head));
            }
            Outcome::Freed => {
                judged += 1;
                freed += 1;
                frees.push(format!("FREED   seq={} {}", row.seq, head));
            }
        }
    }
    println!("replay: {path}");
    println!(
        "rows: {rows}  judged: {judged}  unchanged: {unchanged}  \
         new-denies: {denies}  freed: {freed}  skipped: {skipped}"
    );
    for n in &news {
        println!("{n}");
    }
    for f in &frees {
        println!("{f}");
    }
    print_law_fires(&deny_fires, &steer_fires);
    0
}

/// The REPLAY-COUNTS-1 summary: per law id, deny and steer fires over the
/// judged rows, then every REGISTERED law that never fired — coverage the
/// drift ratchet can read, never a claim that unfired means unnecessary.
fn print_law_fires(deny_fires: &BTreeMap<String, u64>, steer_fires: &BTreeMap<String, u64>) {
    let mut fired: BTreeSet<&str> = BTreeSet::new();
    for id in deny_fires.keys().chain(steer_fires.keys()) {
        fired.insert(id);
    }
    if fired.is_empty() {
        println!("law fires: none");
    } else {
        println!("law fires (current law over judged rows):");
        for id in &fired {
            println!(
                "  {id} deny={} steer={}",
                deny_fires.get(*id).copied().unwrap_or(0),
                steer_fires.get(*id).copied().unwrap_or(0)
            );
        }
    }
    let never: Vec<&str> = checks::registered_ids()
        .into_iter()
        .filter(|id| !fired.contains(id))
        .collect();
    println!("never fired: {}", never.join(", "));
}
