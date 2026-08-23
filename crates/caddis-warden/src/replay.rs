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

use caddis_warden::{decide, ToolCall, Verdict};
use std::path::Path;

struct Row {
    seq: u64,
    from: String,
    ts: u64,
    tool: String,
    body: String,
}

/// Scan a JSONL row for the three fields replay needs — no serde, this
/// crate carries zero dependencies by stated property. Returns None for
/// lines that are not ledger rows; the unescape is the minimal JSON set
/// the ledger writer produces (\", \\, \n, \t).
fn parse_row(line: &str) -> Option<Row> {
    let seq = extract(line, "\"seq\":")?.parse::<u64>().ok()?;
    let typ = extract(line, "\"type\":\"")?;
    let body = extract(line, "\"body\":\"")?;
    let from = unescape(&extract(line, "\"from\":\"").unwrap_or_default());
    let ts = extract(line, "\"ts\":")
        .and_then(|t| t.parse::<u64>().ok())
        .unwrap_or(0);
    Some(Row {
        seq,
        from,
        ts,
        tool: unescape(&typ),
        body: unescape(&body),
    })
}

/// The raw text after `needle` up to the closing quote or digit end.
fn extract(line: &str, needle: &str) -> Option<String> {
    let start = line.find(needle)? + needle.len();
    let rest = &line[start..];
    if needle.ends_with('"') {
        // string field: read to the unescaped closing quote
        let mut out = String::new();
        let mut chars = rest.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                out.push(c);
                out.push(chars.next()?);
            } else if c == '"' {
                return Some(out);
            } else {
                out.push(c);
            }
        }
        None
    } else {
        // numeric-or-quoted field: the real ledger quotes ts ("ts":"1787…"),
        // fixtures write it bare — accept both shapes.
        Some(
            rest.split(',')
                .next()?
                .trim_end_matches('}')
                .trim_matches('"')
                .to_string(),
        )
    }
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// Split `tag|command|path|why` — the command may contain pipes (it is
/// never re-derived from elsewhere), so the tail splits from the RIGHT.
fn split_body(body: &str) -> Option<(String, String)> {
    let (tag, rest) = body.split_once('|')?;
    // rest = "command|path|why": strip why, then path — from the right, so
    // pipes INSIDE the command survive.
    let without_why = rest.rsplit_once('|')?.0;
    let cmd = without_why.rsplit_once('|')?.0;
    Some((tag.to_string(), cmd.to_string()))
}

fn first_line_capped(s: &str) -> String {
    s.lines().next().unwrap_or("").chars().take(60).collect()
}

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
fn classify(row: &Row) -> Outcome {
    if row.tool != "tool.bash" && row.tool != "tool.powershell" {
        return Outcome::Skipped;
    }
    let Some((old_tag, cmd)) = split_body(&row.body) else {
        return Outcome::Skipped;
    };
    if cmd.contains("***redacted") || cmd.contains("bytes truncated]") {
        return Outcome::Skipped;
    }
    let call = ToolCall::new(&row.tool["tool.".len()..]).command(&cmd);
    let new_tag = match decide(&call) {
        Verdict::Allow => "allow",
        Verdict::Steer { .. } => "steer",
        Verdict::Deny { .. } => "deny",
    };
    if new_tag == old_tag {
        Outcome::Unchanged
    } else if new_tag == "deny" {
        Outcome::NewDeny
    } else if old_tag == "deny" {
        Outcome::Freed
    } else {
        Outcome::Note
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
        match classify(&row) {
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
    0
}
