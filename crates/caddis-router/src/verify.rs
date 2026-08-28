//! `verify` — honest findings over a decision ledger (P2, the model-voice
//! law: a ledger tool reports what IS, including its operator's own history;
//! it never silently repairs).
//!
//! Structural findings (unparseable lines) come from [`crate::ledger::load`];
//! this module adds the SEMANTIC layer: seq must be exactly 1, 2, 3, … in
//! file order. A gap, a duplicate, or a regression is reported with the
//! line number and both seq values — the historical model-voice fork taught
//! that per-line linkage checks alone are blind to exactly these.
//!
//! NOT findings, deliberately:
//! - duplicate `route_id` — legitimate (O2 escalation re-routes the same
//!   card; the CARD references the latest decision row);
//! - a torn trailing line — already a `bad-line` finding; the ledger's
//!   append law guarantees it can only ever be the last line.

use crate::ledger::{Ledger, LedgerErr, Loaded};

#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    /// 1-based line in the file.
    pub line: u64,
    pub code: &'static str,
    pub detail: String,
}

#[derive(Debug, Default, PartialEq)]
pub struct VerifyReport {
    /// Non-empty lines scanned.
    pub lines: u64,
    /// Rows that parsed AND passed semantic checks.
    pub rows_ok: u64,
    pub findings: Vec<Finding>,
}

impl VerifyReport {
    /// model-voice convention: the honest exit code is the finding COUNT.
    pub fn rc(&self) -> i32 {
        self.findings.len().min(i32::MAX as usize) as i32
    }
}

pub fn verify_path(path: &std::path::Path) -> Result<VerifyReport, LedgerErr> {
    Ok(verify_loaded(&Ledger::new(path).load()?))
}

pub fn verify_loaded(loaded: &Loaded) -> VerifyReport {
    let mut rep = VerifyReport::default();
    let mut prev: Option<u64> = None; // last row's seq in FILE order
    let mut bad_iter = loaded.bad.iter().peekable();
    for parsed in &loaded.rows {
        // Report bad lines IN FILE ORDER, interleaved with seq findings.
        while let Some(&(line, _)) = bad_iter.peek() {
            if *line < parsed.line {
                let (line, why) = bad_iter.next().unwrap();
                rep.findings.push(Finding {
                    line: *line,
                    code: "bad-line",
                    detail: why.clone(),
                });
            } else {
                break;
            }
        }
        let finding = match prev {
            None if parsed.seq == 1 => None,
            None => Some((
                "seq-start",
                format!("first row has seq {}, expected 1", parsed.seq),
            )),
            Some(p) if parsed.seq == p => Some((
                "seq-dup",
                format!(
                    "seq {} repeats the previous row's seq (fork signature)",
                    parsed.seq
                ),
            )),
            Some(p) if parsed.seq > p + 1 => Some((
                "seq-gap",
                format!("seq {} jumps past {} ({} missing)", parsed.seq, p, p + 1),
            )),
            Some(p) if parsed.seq < p => Some((
                "seq-regression",
                format!("seq {} goes backward after {}", parsed.seq, p),
            )),
            Some(_) => None, // exactly prev+1: the only clean step
        };
        match finding {
            Some((code, detail)) => rep.findings.push(Finding {
                line: parsed.line,
                code,
                detail,
            }),
            None => rep.rows_ok += 1,
        }
        prev = Some(parsed.seq);
    }
    for (line, why) in bad_iter {
        rep.findings.push(Finding {
            line: *line,
            code: "bad-line",
            detail: why.clone(),
        });
    }
    rep.lines = (loaded.rows.len() + loaded.bad.len()) as u64;
    rep
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::parse_stream;

    fn out(seq: u64, lane: &str) -> String {
        format!(
            "{{\"seq\":{seq},\"ts\":\"t\",\"kind\":\"outcome\",\"card_id\":\"c\",\"task_class\":\"k\",\
             \"lane_id\":\"{lane}\",\"model\":\"m\",\"cost_tokens\":1,\"cost_usd_est\":0.0,\
             \"latency_ms\":1,\"verify_outcome\":\"pass\",\"escalated_to\":null}}"
        )
    }

    #[test]
    fn clean_ledger_has_zero_findings() {
        let loaded = parse_stream(&[out(1, "a"), out(2, "a"), out(3, "b")].join("\n"));
        let rep = verify_loaded(&loaded);
        assert_eq!(rep.findings, vec![]);
        assert_eq!(rep.rows_ok, 3);
        assert_eq!(rep.lines, 3);
        assert_eq!(rep.rc(), 0);
    }

    #[test]
    fn findings_carry_line_numbers_in_file_order() {
        let text = [
            out(1, "a"),
            "garbage {".to_string(),
            out(4, "a"), // gap: expected 2
            out(4, "a"), // dup: expected 5 now? seq 4 again where 5 expected
            out(3, "b"), // regression
        ]
        .join("\n");
        let loaded = parse_stream(&text);
        let rep = verify_loaded(&loaded);
        let codes: Vec<(&'static str, u64)> =
            rep.findings.iter().map(|f| (f.code, f.line)).collect();
        assert_eq!(
            codes,
            vec![
                ("bad-line", 2),
                ("seq-gap", 3),
                ("seq-dup", 4),
                ("seq-regression", 5),
            ]
        );
    }

    #[test]
    fn empty_and_missing_files_are_clean_zero() {
        let loaded = parse_stream("");
        assert_eq!(verify_loaded(&loaded).rc(), 0);
        let dir = std::env::temp_dir().join(format!("rtr-verify-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let missing = dir.join("nope.jsonl");
        let rep = verify_path(&missing).unwrap();
        assert_eq!(rep.rc(), 0);
        std::fs::remove_dir_all(dir).ok();
    }
}
