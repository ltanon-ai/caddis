//! report.rs — the ledger's reading end (REPORT-1).
//!
//! `caddis-warden report` aggregates the append-only ledger the warden
//! itself writes: counts by verdict and caller, first/last timestamps, and
//! deny reasons grouped by the law id the why field carries. The ledger
//! exists to answer "what did my agents do last night"; replay answers it
//! against the CURRENT law, report answers it as RECORDED — no
//! re-judgement, no second opinion, just the counts.
//!
//! The row scan is replay's (shared, never duplicated): one parser for one
//! file format, or the two rot apart exactly where the ledger's
//! credibility lives. JSON output is hand-rolled under the crate's
//! zero-dependency law, escaping via wire::json_escape.

use crate::rows::{parse_row, Row};
use crate::wire::json_escape;
use std::collections::BTreeMap;

/// Optional narrowing, mirroring replay's filters plus report's own:
/// verdict tag and a `--last N` tail slice OF THE FILTERED SET.
struct Filters {
    from: Option<String>,
    since_hours: Option<u64>,
    verdict: Option<String>,
    last: Option<usize>,
}

fn parse_filters(args: &[String]) -> Result<Filters, String> {
    let mut f = Filters {
        from: None,
        since_hours: None,
        verdict: None,
        last: None,
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--from" => f.from = it.next().cloned(),
            "--since" => f.since_hours = it.next().and_then(|v| v.parse().ok()),
            "--verdict" => {
                let v = it.next().cloned();
                if let Some(tag) = &v {
                    if !matches!(tag.as_str(), "allow" | "steer" | "deny") {
                        return Err(format!("unknown verdict `{tag}`"));
                    }
                }
                f.verdict = v;
            }
            "--last" => f.last = it.next().and_then(|v| v.parse().ok()),
            _ => {}
        }
    }
    Ok(f)
}

impl Filters {
    fn admits(&self, row: &Row, tag: &str) -> bool {
        if let Some(from) = &self.from {
            if !crate::rows::from_matches(&row.from, from) {
                return false;
            }
        }
        if let Some(want) = &self.verdict {
            if tag != want {
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

/// `tag|command|path|why` or `tag|command|path|why|fp` (CARD-0129).
/// Why is never the fingerprint.
fn tag_and_why(body: &str) -> (String, String) {
    (
        body.split('|').next().unwrap_or("").to_string(),
        crate::rows::body_why(body),
    )
}

/// The law id a deny why field names (`caddis-warden [id]: …`); rows whose
/// reason carries no bracket (sensitive-path denials predate the bracket
/// form) group under `(unattributed)` rather than being dropped.
fn law_id(why: &str) -> String {
    crate::rows::law_id_bracketed(why).unwrap_or("(unattributed)".to_string())
}

#[derive(Default)]
struct Agg {
    rows: u64,
    allow: u64,
    steer: u64,
    deny: u64,
    by_from: BTreeMap<String, u64>,
    first_ts: Option<u64>,
    last_ts: u64,
    deny_by_law: BTreeMap<String, Vec<u64>>,
}

fn fold(agg: &mut Agg, row: &Row) {
    let (tag, why) = tag_and_why(&row.body);
    agg.rows += 1;
    match tag.as_str() {
        "allow" => agg.allow += 1,
        "steer" => agg.steer += 1,
        "deny" => {
            agg.deny += 1;
            agg.deny_by_law
                .entry(law_id(&why))
                .or_default()
                .push(row.seq);
        }
        _ => {}
    }
    *agg.by_from.entry(row.from.clone()).or_default() += 1;
    if agg.first_ts.is_none() || row.ts < agg.first_ts.unwrap_or(0) {
        agg.first_ts = Some(row.ts);
    }
    if row.ts > agg.last_ts {
        agg.last_ts = row.ts;
    }
}

fn json_of(path: &str, a: &Agg) -> String {
    let from: Vec<String> = a
        .by_from
        .iter()
        .map(|(k, v)| format!("\"{}\":{}", json_escape(k), v))
        .collect();
    let laws: Vec<String> = a
        .deny_by_law
        .iter()
        .map(|(k, seqs)| {
            let s: Vec<String> = seqs.iter().map(|n| n.to_string()).collect();
            format!("\"{}\":[{}]", json_escape(k), s.join(","))
        })
        .collect();
    format!(
        "{{\"ledger\":\"{}\",\"rows\":{},\"verdicts\":{{\"allow\":{},\"steer\":{},\"deny\":{}}},\
         \"from\":{{{}}},\"first_ts\":{},\"last_ts\":{},\"deny_by_law\":{{{}}}}}",
        json_escape(path),
        a.rows,
        a.allow,
        a.steer,
        a.deny,
        from.join(","),
        a.first_ts.unwrap_or(0),
        a.last_ts,
        laws.join(",")
    )
}

fn digest(path: &str, a: &Agg) -> String {
    let mut s = format!(
        "report: {path}\nrows: {}  allow: {}  steer: {}  deny: {}",
        a.rows, a.allow, a.steer, a.deny
    );
    let from: Vec<String> = a.by_from.iter().map(|(k, v)| format!("{k}={v}")).collect();
    if !from.is_empty() {
        s.push_str(&format!("\nfrom: {}", from.join("  ")));
    }
    s.push_str(&format!(
        "\nfirst_ts: {}  last_ts: {}",
        a.first_ts.unwrap_or(0),
        a.last_ts
    ));
    if !a.deny_by_law.is_empty() {
        s.push_str("\ndeny by law:");
        for (law, seqs) in &a.deny_by_law {
            let listed: Vec<String> = seqs.iter().map(|n| n.to_string()).collect();
            s.push_str(&format!(
                "\n  {law} x{} (seq {})",
                seqs.len(),
                listed.join(",")
            ));
        }
    }
    s
}

/// The whole report: read-only aggregation of one ledger.
pub fn run(args: &[String]) -> i32 {
    let filters = match parse_filters(&args[2.min(args.len())..]) {
        Ok(f) => f,
        Err(why) => {
            eprintln!("report: {why}");
            eprintln!(
                "usage: caddis-warden report [--from NAME] [--since HOURS] \
                       [--verdict allow|steer|deny] [--last N] [--json]"
            );
            return 2;
        }
    };
    let path = crate::identity::ledger_path()
        .to_string_lossy()
        .into_owned();
    let kept = match load_rows(&path, &filters) {
        Ok(rows) => rows,
        Err(e) => {
            eprintln!("report: cannot read {path}: {e}");
            return 2;
        }
    };
    let mut agg = Agg::default();
    for row in &kept {
        fold(&mut agg, row);
    }
    if args.iter().any(|a| a == "--json") {
        println!("{}", json_of(&path, &agg));
    } else {
        println!("{}", digest(&path, &agg));
    }
    0
}

/// Scan, filter, and tail-slice the ledger — split from `run` when the
/// gate measured CCN 11 against the cap of 10 (the fix is a split, never
/// a trim; see wire.rs for the precedent).
fn load_rows(path: &str, filters: &Filters) -> Result<Vec<Row>, std::io::Error> {
    let text = std::fs::read_to_string(path)?;
    let mut kept: Vec<Row> = Vec::new();
    for line in text.lines() {
        if let Some(row) = parse_row(line) {
            let (tag, _) = tag_and_why(&row.body);
            if filters.admits(&row, &tag) {
                kept.push(row);
            }
        }
    }
    if let Some(n) = filters.last {
        let drop = kept.len().saturating_sub(n);
        kept.drain(0..drop);
    }
    Ok(kept)
}

#[cfg(test)]
#[path = "report_tests.rs"]
mod tests;
