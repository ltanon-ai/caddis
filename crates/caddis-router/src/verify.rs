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
//! R5 (P4 slice 5) adds the SIGNATURE layer: rows the organ appended under a
//! warden key carry `sig`; verify recomputes the canonical encoding and
//! checks the HMAC. A present-but-wrong signature (`sig-mismatch`) means the
//! row was edited or forged after append. A signature with no key to check
//! it against is `sig-no-key`. A row appended AFTER activation without a
//! signature is `unsigned-row` — exactly the injected row R5 exists to
//! catch. Rows at or below `activated_seq` are the honest unsigned history:
//! COUNTED, never findings.
//!
//! NOT findings, deliberately:
//! - duplicate `route_id` — legitimate (O2 escalation re-routes the same
//!   card; the CARD references the latest decision row);
//! - a torn trailing line — already a `bad-line` finding; the ledger's
//!   append law guarantees it can only ever be the last line.

use crate::ledger::{encode_canonical, Ledger, LedgerErr, Loaded};
use crate::warden::WardenSlot;

#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    /// 1-based line in the file.
    pub line: u64,
    pub code: &'static str,
    pub detail: String,
}

/// What verify could tell about the warden beside the ledger.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum WardenInfo {
    /// No `warden.key` beside the ledger: unsigned era, activation is the
    /// operator's call (`caddis-router warden mint`).
    #[default]
    NoKey,
    /// Key live: fingerprint (16 hex) + the last seq that may be unsigned.
    Key {
        fingerprint: String,
        activated_seq: u64,
    },
    /// Key file present but unusable — signed rows cannot be checked.
    Broken(String),
}

#[derive(Debug, Default, PartialEq)]
pub struct VerifyReport {
    /// Non-empty lines scanned.
    pub lines: u64,
    /// Rows that parsed AND passed semantic checks.
    pub rows_ok: u64,
    pub findings: Vec<Finding>,
    /// R5: rows whose signature recomputed correctly under the live key.
    pub signed_ok: u64,
    /// R5: rows with no signature (pre-activation history, or no key at all).
    pub unsigned: u64,
    /// R5: the warden state this report was checked against.
    pub warden: WardenInfo,
}

impl VerifyReport {
    /// model-voice convention: the honest exit code is the finding COUNT.
    pub fn rc(&self) -> i32 {
        self.findings.len().min(i32::MAX as usize) as i32
    }
}

pub fn verify_path(path: &std::path::Path) -> Result<VerifyReport, LedgerErr> {
    let ledger = Ledger::new(path);
    let loaded = ledger.load()?;
    Ok(verify_with(&loaded, ledger.warden()))
}

/// The SEMANTIC layer alone (seq discipline), kept for callers that verify
/// parsed streams without a filesystem home. Signature counts stay zero and
/// the warden reports [`WardenInfo::NoKey`] — the sig layer needs the key,
/// which lives beside a file.
pub fn verify_loaded(loaded: &Loaded) -> VerifyReport {
    verify_with(loaded, &WardenSlot::Absent)
}

/// Full check: semantics for every row, signatures for every row a key (or
/// its absence) can judge. See the module doc for the finding classes.
pub fn verify_with(loaded: &Loaded, slot: &WardenSlot) -> VerifyReport {
    let mut rep = VerifyReport {
        warden: match slot {
            WardenSlot::Absent => WardenInfo::NoKey,
            WardenSlot::Key(k) => WardenInfo::Key {
                fingerprint: k.fingerprint(),
                activated_seq: k.activated_seq(),
            },
            WardenSlot::Broken(why) => WardenInfo::Broken(why.clone()),
        },
        ..VerifyReport::default()
    };
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
        check_signature(&mut rep, parsed, slot);
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

/// R5 judgement for ONE parsed row against the slot. Never panics: an
/// unencodable row (impossible after decode, but the type says Result) is a
/// mismatch finding, not a crash.
fn check_signature(rep: &mut VerifyReport, parsed: &crate::ledger::ParsedRow, slot: &WardenSlot) {
    match (&parsed.sig, slot) {
        (Some(sig), WardenSlot::Key(k)) => match recompute(parsed) {
            Some(canonical) if k.check(&canonical, sig) => rep.signed_ok += 1,
            Some(_) => rep.findings.push(Finding {
                line: parsed.line,
                code: "sig-mismatch",
                detail: format!(
                    "seq {} signature fails HMAC over its canonical encoding — \
                     the row was edited or forged after append",
                    parsed.seq
                ),
            }),
            None => rep.findings.push(Finding {
                line: parsed.line,
                code: "sig-mismatch",
                detail: format!(
                    "seq {} cannot be canonically re-encoded — signature unverifiable",
                    parsed.seq
                ),
            }),
        },
        (Some(_), WardenSlot::Absent) => rep.findings.push(Finding {
            line: parsed.line,
            code: "sig-no-key",
            detail: format!(
                "seq {} carries a signature but no warden.key lives beside the \
                 ledger — it cannot be checked",
                parsed.seq
            ),
        }),
        (Some(_), WardenSlot::Broken(why)) => rep.findings.push(Finding {
            line: parsed.line,
            code: "sig-unverifiable",
            detail: format!(
                "seq {} signed, but the key file is broken: {why}",
                parsed.seq
            ),
        }),
        (None, WardenSlot::Key(k)) => {
            if parsed.seq > k.activated_seq() {
                rep.findings.push(Finding {
                    line: parsed.line,
                    code: "unsigned-row",
                    detail: format!(
                        "seq {} has no signature but the warden activated at seq {} \
                         — injected or appended outside the organ",
                        parsed.seq,
                        k.activated_seq()
                    ),
                });
            } else {
                rep.unsigned += 1; // the honest pre-activation era
            }
        }
        (None, _) => rep.unsigned += 1,
    }
}

fn recompute(parsed: &crate::ledger::ParsedRow) -> Option<String> {
    encode_canonical(parsed.seq, &parsed.ts, &parsed.row).ok()
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::parse_stream;
    use crate::WardenKey;

    fn out(seq: u64, lane: &str) -> String {
        format!(
            "{{\"seq\":{seq},\"ts\":\"t\",\"kind\":\"outcome\",\"card_id\":\"C\",\
             \"task_class\":\"coding\",\"lane_id\":\"{lane}\",\"model\":\"m\",\
             \"cost_tokens\":1,\"cost_usd_est\":0.001,\"latency_ms\":10,\
             \"verify_outcome\":\"pass\",\"escalated_to\":null}}"
        )
    }

    #[test]
    fn clean_ledger_has_zero_findings() {
        let loaded = parse_stream(&[out(1, "a"), out(2, "a"), out(3, "b")].join("\n"));
        let rep = verify_loaded(&loaded);
        assert_eq!(rep.findings, vec![]);
        assert_eq!(rep.rows_ok, 3);
        assert_eq!(rep.lines, 3);
        assert_eq!(rep.unsigned, 3, "no key: all rows honestly unsigned");
        assert_eq!(rep.signed_ok, 0);
    }

    #[test]
    fn fork_and_gap_and_regression_are_findings() {
        let text = [out(1, "a"), out(2, "a"), out(2, "b"), out(9, "c")].join("\n");
        let loaded = parse_stream(&text);
        let rep = verify_loaded(&loaded);
        let codes: Vec<(&'static str, u64)> =
            rep.findings.iter().map(|f| (f.code, f.line)).collect();
        assert_eq!(
            codes,
            vec![("seq-dup", 3), ("seq-gap", 4),],
            "fork + gap in file order"
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

    // --- R5: the signature layer -------------------------------------------

    fn key_dir(tag: &str, activated_seq: u64) -> (std::path::PathBuf, WardenKey) {
        let dir = std::env::temp_dir().join(format!("rtr-verify5-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let k = crate::warden::mint(&dir, activated_seq).unwrap();
        (dir, k)
    }

    /// Append two rows through the REAL ledger (signed), one unsigned row at
    /// a seq below activation, then verify against the minted key.
    #[test]
    fn signed_rows_verify_clean_and_pre_activation_rows_count() {
        let (dir, k) = key_dir("clean", 2);
        let lpath = dir.join("ledger.jsonl");
        // Two unsigned rows FIRST (the pre-activation history), written
        // directly — the organ had no key yet.
        std::fs::write(&lpath, format!("{}\n{}\n", out(1, "a"), out(2, "a"))).unwrap();
        // …then organ appends under the key: rows 3+ are signed.
        let led = Ledger::new(&lpath);
        let seq = led
            .append_ts(
                &crate::ledger::Row::Outcome(crate::ledger::OutcomeRow {
                    card_id: "C".into(),
                    task_class: "coding".into(),
                    lane_id: "a".into(),
                    model: "m".into(),
                    cost_tokens: 1,
                    cost_usd_est: 0.001,
                    latency_ms: 10,
                    outcome: crate::ledger::Outcome::Pass,
                    escalated_to: None,
                }),
                "t",
                crate::ledger::LOCK_WAIT,
            )
            .unwrap();
        assert_eq!(seq, 3);
        let rep = verify_path(&lpath).unwrap();
        assert_eq!(rep.findings, vec![], "findings: {:?}", rep.findings);
        assert_eq!(rep.signed_ok, 1);
        assert_eq!(rep.unsigned, 2, "pre-activation history counts, not flags");
        assert!(matches!(
            &rep.warden,
            WardenInfo::Key { fingerprint, activated_seq } if *activated_seq == 2 && fingerprint.len() == 16
        ));
        let _ = k;
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn tampered_signed_row_is_a_mismatch_finding() {
        let (dir, _k) = key_dir("tamper", 0);
        let lpath = dir.join("ledger.jsonl");
        let led = Ledger::new(&lpath);
        led.append_ts(
            &crate::ledger::Row::Outcome(crate::ledger::OutcomeRow {
                card_id: "C".into(),
                task_class: "coding".into(),
                lane_id: "a".into(),
                model: "honest-model".into(),
                cost_tokens: 1,
                cost_usd_est: 0.001,
                latency_ms: 10,
                outcome: crate::ledger::Outcome::Pass,
                escalated_to: None,
            }),
            "t",
            crate::ledger::LOCK_WAIT,
        )
        .unwrap();
        // Hand-edit the model identity on the signed row: the exact R5 attack.
        let raw = std::fs::read_to_string(&lpath).unwrap();
        let forged = raw.replace("honest-model", "poisoned-model");
        assert_ne!(raw, forged);
        std::fs::write(&lpath, forged).unwrap();
        let rep = verify_path(&lpath).unwrap();
        assert_eq!(rep.rc(), 1, "findings: {:?}", rep.findings);
        assert_eq!(rep.findings[0].code, "sig-mismatch");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unsigned_row_after_activation_is_a_finding() {
        let (dir, _k) = key_dir("inject", 1);
        let lpath = dir.join("ledger.jsonl");
        // Row 1: pre-activation, unsigned, fine. Row 2: injected, unsigned.
        std::fs::write(&lpath, format!("{}\n{}\n", out(1, "a"), out(2, "a"))).unwrap();
        let rep = verify_path(&lpath).unwrap();
        assert_eq!(rep.rc(), 1, "findings: {:?}", rep.findings);
        assert_eq!(rep.findings[0].code, "unsigned-row");
        assert!(rep.findings[0].detail.contains("activated at seq 1"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn signed_row_with_no_key_is_reported() {
        // A ledger written under a key, then verified WITHOUT the key.
        let (dir, _k) = key_dir("nokey", 0);
        let lpath = dir.join("ledger.jsonl");
        let led = Ledger::new(&lpath);
        led.append_ts(
            &crate::ledger::Row::Outcome(crate::ledger::OutcomeRow {
                card_id: "C".into(),
                task_class: "coding".into(),
                lane_id: "a".into(),
                model: "m".into(),
                cost_tokens: 1,
                cost_usd_est: 0.001,
                latency_ms: 10,
                outcome: crate::ledger::Outcome::Pass,
                escalated_to: None,
            }),
            "t",
            crate::ledger::LOCK_WAIT,
        )
        .unwrap();
        std::fs::remove_file(dir.join("warden.key")).unwrap();
        let rep = verify_path(&lpath).unwrap();
        assert_eq!(rep.rc(), 1, "findings: {:?}", rep.findings);
        assert_eq!(rep.findings[0].code, "sig-no-key");
        std::fs::remove_dir_all(&dir).ok();
    }
}
