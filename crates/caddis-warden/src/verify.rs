//! verify.rs — the ledger's integrity reading end (WARDEN-LEDGER-INTEGRITY
//! 2026-08-26, recommendation 3).
//!
//! `caddis-warden verify` scans one ledger READ-ONLY and counts what the
//! integrity survey measured by hand: unparseable lines (defect L1 — two
//! writers' rows fused by an unlocked concurrent append) and duplicate `seq`
//! values (defect L2 — `seq` is per-writer-instance, so every restarted
//! writer re-seeds from 1). Same findings-engine shape as the model-voice
//! organ v0.2.0: the tool REPORTS the ledger's honest state, it never
//! rewrites history — the append-only law outranks tidiness, and the L1/L2
//! write-path fix shape belongs to the warden owner, not to this reader.
//!
//! The row scan is replay's and report's (shared, never duplicated): one
//! parser for one file format. JSON output is hand-rolled under the crate's
//! zero-dependency law, escaping via wire::json_escape.

use crate::rows::{parse_row, Row};
use crate::wire::json_escape;
use std::collections::BTreeMap;

/// Exit code when findings exist. Distinct from 2 (usage/read failure) so a
/// caller can tell "the ledger has defects" from "the tool could not look".
pub const FINDINGS: i32 = 3;

/// The most examples/worst-offenders the output lists — enough to act on,
/// short enough that a badly corrupt ledger cannot flood a terminal.
const EXAMPLES: usize = 5;

/// One scan's honest state. `unparseable` keeps (line number, capped head)
/// pairs so a finding can be located by eye, not just counted.
#[derive(Default)]
pub(crate) struct Scan {
    scanned: usize,
    blank: usize,
    rows: usize,
    unparseable: Vec<(usize, String)>,
    unparseable_total: usize,
    seq_counts: BTreeMap<u64, usize>,
    by_from: BTreeMap<String, usize>,
    first_ts: Option<u64>,
    last_ts: u64,
}

/// The single pass over the file. Blank lines are counted, never treated as
/// corruption: a trailing newline artifact is not a lost record, and calling
/// it one would cry wolf on every healthy ledger.
pub(crate) fn scan(text: &str) -> Scan {
    let mut s = Scan::default();
    for (idx, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            s.blank += 1;
            continue;
        }
        s.scanned += 1;
        match parse_row(line) {
            Some(row) => fold(&mut s, row),
            None => {
                s.unparseable_total += 1;
                if s.unparseable.len() < EXAMPLES {
                    let head = crate::rows::first_line_capped(line);
                    s.unparseable.push((idx + 1, head));
                }
            }
        }
    }
    s
}

fn fold(s: &mut Scan, row: Row) {
    s.rows += 1;
    *s.seq_counts.entry(row.seq).or_insert(0) += 1;
    *s.by_from.entry(row.from).or_insert(0) += 1;
    if row.ts > 0 && s.first_ts.is_none_or(|f| row.ts < f) {
        s.first_ts = Some(row.ts);
    }
    if row.ts > s.last_ts {
        s.last_ts = row.ts;
    }
}

/// Seq values seen more than once, worst first (count desc, then seq asc so
/// the order is deterministic between runs on the same ledger).
pub(crate) fn dup_seqs(s: &Scan) -> Vec<(u64, usize)> {
    let mut dups: Vec<(u64, usize)> = s
        .seq_counts
        .iter()
        .filter(|(_, n)| **n > 1)
        .map(|(seq, n)| (*seq, *n))
        .collect();
    dups.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    dups
}

/// Parsed coverage as hundredths of a percent, integer math so the JSON never
/// carries a float artifact. An empty ledger is vacuously complete.
fn coverage_hundredths(s: &Scan) -> u64 {
    if s.scanned == 0 {
        10000
    } else {
        (s.rows as u64 * 10000) / s.scanned as u64
    }
}

fn json_of(path: &str, s: &Scan) -> String {
    let dups = dup_seqs(s);
    let dup_rows: usize = dups.iter().map(|(_, n)| *n).sum();
    let examples: Vec<String> = s
        .unparseable
        .iter()
        .map(|(line, head)| {
            format!(
                "{{\"line\":{},\"head\":\"{}\"}}",
                line,
                json_escape(head)
            )
        })
        .collect();
    let worst: Vec<String> = dups
        .iter()
        .take(EXAMPLES)
        .map(|(seq, n)| format!("{{\"seq\":{seq},\"count\":{n}}}"))
        .collect();
    let from: Vec<String> = s
        .by_from
        .iter()
        .map(|(k, v)| format!("\"{}\":{}", json_escape(k), v))
        .collect();
    let cov = coverage_hundredths(s);
    let findings = s.unparseable_total + dups.len();
    format!(
        "{{\"ledger\":\"{}\",\"lines_scanned\":{},\"blank_lines\":{},\"rows\":{},\
         \"coverage_pct\":{}.{:02},\"from\":{{{}}},\"first_ts\":{},\"last_ts\":{},\
         \"findings\":{{\"unparseable\":{},\"unparseable_examples\":[{}],\
         \"dup_seq_values\":{},\"dup_seq_rows\":{},\"dup_seq_worst\":[{}]}},\
         \"status\":\"{}\"}}",
        json_escape(path),
        s.scanned,
        s.blank,
        s.rows,
        cov / 100,
        cov % 100,
        from.join(","),
        s.first_ts.unwrap_or(0),
        s.last_ts,
        s.unparseable_total,
        examples.join(","),
        dups.len(),
        dup_rows,
        worst.join(","),
        if findings == 0 { "clean" } else { "findings" }
    )
}

fn digest(path: &str, s: &Scan) -> String {
    let dups = dup_seqs(s);
    let dup_rows: usize = dups.iter().map(|(_, n)| *n).sum();
    let cov = coverage_hundredths(s);
    let mut out = format!(
        "verify: {path}\nlines: {} (blank {})  rows: {}  coverage: {}.{:02}%  from: {} labels",
        s.scanned, s.blank, s.rows, cov / 100, cov % 100, s.by_from.len()
    );
    if let Some(first) = s.first_ts {
        out.push_str(&format!("\nfirst_ts: {first}  last_ts: {}", s.last_ts));
    }
    out.push_str(&format!("\nunparseable: {}", s.unparseable_total));
    for (line, head) in &s.unparseable {
        out.push_str(&format!("\n  line {line}: {head}"));
    }
    out.push_str(&format!(
        "\ndup seq: {} values across {} rows",
        dups.len(),
        dup_rows
    ));
    if !dups.is_empty() {
        let worst: Vec<String> = dups
            .iter()
            .take(EXAMPLES)
            .map(|(seq, n)| format!("seq={seq} x{n}"))
            .collect();
        out.push_str(&format!("\n  worst: {}", worst.join("  ")));
    }
    out.push_str(&format!(
        "\nstatus: {}",
        if s.unparseable_total + dups.len() == 0 {
            "CLEAN"
        } else {
            "FINDINGS"
        }
    ));
    out
}

/// The whole verify: read-only integrity scan of one ledger. The path is the
/// first positional argument if given (replay's convention for pointing a
/// diagnostic at an explicit artifact), else `CADDIS_WARDEN_LEDGER` (report's
/// convention for the shared live ledger) — composing the two the crate
/// already has rather than inventing a third.
pub fn run(args: &[String]) -> i32 {
    let rest = &args[2.min(args.len())..];
    let path = match rest.iter().find(|a| !a.starts_with("--")) {
        Some(p) => p.clone(),
        None => match std::env::var("CADDIS_WARDEN_LEDGER") {
            Ok(p) if !p.is_empty() => p,
            _ => {
                eprintln!("verify: no ledger path and CADDIS_WARDEN_LEDGER is not set");
                eprintln!(
                    "usage: caddis-warden verify [LEDGER] [--json]   \
                       (exit 0 clean, {FINDINGS} findings, 2 unreadable)"
                );
                return 2;
            }
        },
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("verify: cannot read {path}: {e}");
            return 2;
        }
    };
    let s = scan(&text);
    if rest.iter().any(|a| a == "--json") {
        println!("{}", json_of(&path, &s));
    } else {
        println!("{}", digest(&path, &s));
    }
    if s.unparseable_total > 0 || !dup_seqs(&s).is_empty() {
        FINDINGS
    } else {
        0
    }
}

#[cfg(test)]
#[path = "verify_tests.rs"]
mod tests;
