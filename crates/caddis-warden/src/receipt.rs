//! receipt.rs — the ledger becomes evidence (CARD-0112, unit A).
//!
//! `caddis-warden receipt` reconstructs from the ledger ALONE what one caller
//! did in one window. The ledger has been write-mostly since it was born — 15k
//! rows in, almost nothing out — which is the "writer with no reader" failure
//! this estate treats as dead machinery.
//!
//! THE CONSUMER IS NAMED, because a reader that nobody uses is the same defect
//! one layer up: every handoff, report and MR body carries one. The handoff
//! auditor stops checking prose against the writer's MEMORY and starts diffing
//! prose against the receipt — a mechanical check where a judgement call used
//! to be.
//!
//! ⚠ HONEST SCOPE. A receipt says WHAT happened. It cannot say why, what was
//! decided, or what comes next, and it cannot say whether anything SUCCEEDED:
//! the warden fires before a tool runs and no exit code exists in a row. It
//! removes fabrication, not the need to write.

use crate::receipt_report::{render_json, render_text};
use crate::rows::{
    body_path, body_why, first_line_capped, from_matches, law_id_bracketed, parse_row, split_body,
    Row,
};
use std::collections::BTreeMap;

/// Everything one window of the ledger says about one caller.
#[derive(Default)]
pub struct Receipt {
    pub ledger: String,
    pub from: Option<String>,
    pub since_hours: Option<u64>,
    pub rows: u64,
    pub allow: u64,
    pub steer: u64,
    pub deny: u64,
    pub first_ts: Option<u64>,
    pub last_ts: u64,
    pub by_tool: BTreeMap<String, u64>,
    /// Distinct paths written, with how many times each. DISTINCT, because
    /// "how many files did it touch" and "how many writes did it make" are
    /// different questions and conflating them overstates the blast radius.
    pub files: BTreeMap<String, u64>,
    pub deny_by_law: BTreeMap<String, Vec<u64>>,
    pub law_fires: BTreeMap<String, u64>,
    pub cards_opened: Vec<String>,
    pub cards_closed: Vec<String>,
    /// Rows that no reader can attribute — structural damage in the file.
    pub unreadable: u64,
    /// Rows whose command the ledger deliberately never kept. Counted, because
    /// a withheld command is not a command that did not happen.
    pub withheld: u64,
}

/// Optional narrowing, mirroring `report` so one question has one answer.
pub struct Filters {
    pub from: Option<String>,
    pub since_hours: Option<u64>,
}

pub fn parse_filters(args: &[String]) -> Filters {
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
    fn admits(&self, row: &Row, now: u64) -> bool {
        if let Some(from) = &self.from {
            if !from_matches(&row.from, from) {
                return false;
            }
        }
        if let Some(hours) = self.since_hours {
            // ts == 0 means UNKNOWN, and treating unknown as recent would
            // quietly widen every window.
            if row.ts == 0 || now.saturating_sub(row.ts) > hours * 3600 {
                return false;
            }
        }
        true
    }
}

/// Fold one ledger into one receipt.
pub fn build(ledger_path: &str, text: &str, f: &Filters, now: u64) -> Receipt {
    let mut r = Receipt {
        ledger: ledger_path.to_string(),
        from: f.from.clone(),
        since_hours: f.since_hours,
        ..Default::default()
    };
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let Some(row) = parse_row(line) else {
            r.unreadable += 1;
            continue;
        };
        if !f.admits(&row, now) {
            continue;
        }
        fold(&mut r, &row);
    }
    r
}

fn fold(r: &mut Receipt, row: &Row) {
    r.rows += 1;
    *r.by_tool.entry(row.tool.clone()).or_default() += 1;
    if r.first_ts.is_none_or(|t| row.ts < t) && row.ts > 0 {
        r.first_ts = Some(row.ts);
    }
    r.last_ts = r.last_ts.max(row.ts);
    if fold_card(r, row) {
        return;
    }
    let Some((tag, cmd)) = split_body(&row.body) else {
        return;
    };
    fold_verdict(r, row, &tag);
    if cmd.contains("***redacted") || cmd.contains("bytes truncated]") {
        r.withheld += 1;
    }
    // The row's `path` field lives between the command and the why; a write
    // with an empty path contributes nothing rather than an empty entry.
    let path = body_path(&row.body);
    if !path.is_empty() {
        *r.files.entry(path).or_default() += 1;
    }
}

fn fold_verdict(r: &mut Receipt, row: &Row, tag: &str) {
    let why = body_why(&row.body);
    match tag {
        "allow" => r.allow += 1,
        "steer" => {
            r.steer += 1;
            for id in why.split(", ").filter(|s| !s.is_empty()) {
                *r.law_fires.entry(id.to_string()).or_default() += 1;
            }
        }
        "deny" => {
            r.deny += 1;
            let id = law_id_bracketed(&why).unwrap_or_else(|| "(unattributed)".to_string());
            r.deny_by_law.entry(id.clone()).or_default().push(row.seq);
            *r.law_fires.entry(id).or_default() += 1;
        }
        _ => {}
    }
}

/// Card rows are not verdicts and must not be counted as any.
fn fold_card(r: &mut Receipt, row: &Row) -> bool {
    let id = || row.body.split('|').nth(1).unwrap_or_default().to_string();
    match row.tool.as_str() {
        crate::card_state::OPEN_TYPE => {
            r.cards_opened.push(id());
            true
        }
        crate::card_state::CLOSE_TYPE => {
            r.cards_closed.push(id());
            true
        }
        _ => false,
    }
}

/// One command's head, for the digest. Kept here so the renderer never has to
/// know the body grammar.
pub fn command_head(body: &str) -> String {
    first_line_capped(&split_body(body).map(|(_, c)| c).unwrap_or_default())
}

pub fn run(args: &[String]) -> i32 {
    let path = crate::identity::ledger_path()
        .to_string_lossy()
        .into_owned();
    // An unreadable ledger is an ERROR, never an empty receipt: "you did
    // nothing" and "I could not look" must never print the same.
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("receipt: cannot read {path}: {e}");
            return 2;
        }
    };
    let filters = parse_filters(&args[2.min(args.len())..]);
    let r = build(&path, &text, &filters, crate::identity::unix_seconds());
    if args.iter().any(|a| a == "--json") {
        println!("{}", render_json(&r));
    } else {
        println!("{}", render_text(&r));
    }
    0
}

#[cfg(test)]
#[path = "receipt_tests.rs"]
mod tests;
