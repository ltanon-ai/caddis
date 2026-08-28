//! Capability statistics over the outcome stream (P2): quality per
//! (lane, class) as EWMA over RED-TEST outcomes, cold-start as FAMILY MEDIAN
//! — never as a guess (F2 council fold: "cold-start provisional = family
//! median, N≥5 holdout runs before cheap-pool").
//!
//! Purity law (F1): everything here is a function of (rows, lane registry) —
//! no I/O, no clock. The EWMA constants are the brief's (α ≈ 0.3); the
//! hysteresis counter (2-consecutive-failure) is COMPUTED here and consumed
//! by P3's escalation state machine.
//!
//! Two hard laws encoded:
//! - **F2 selection vs provisional:** [`CapsReport::p1_caps`] returns ONLY a
//!   lane's OWN measurements — a family median can never leak into
//!   [`crate::route`] selection, which independently enforces N ≥
//!   [`MIN_SAMPLES`]. The median is advisory (P3 escalation candidates,
//!   operator-facing reports), never a selection input.
//! - **QQ1a:** warden denies are not rows at all (see `ledger.rs`), so
//!   nothing here can decay a lane for being correctly blocked.

use std::collections::BTreeMap;

use crate::lane::{Capability, Lane};
use crate::ledger::{Loaded, Outcome, Row};

/// Brief ruling: EWMA α ≈ 0.3.
pub const EWMA_ALPHA: f64 = 0.3;
/// F2: minimum OWN successful samples before the cheap-selection pool.
pub const MIN_SAMPLES: u32 = 5;

/// Measured capability of one lane for one class, folded from outcome rows.
#[derive(Debug, Clone, PartialEq)]
pub struct LaneCap {
    pub lane_id: String,
    pub class: String,
    /// EWMA over pass(1.0)/fail(0.0) observations, in seq order. The FIRST
    /// observation initializes it (no hidden 0.5 prior — a lane that passed
    /// once reads 1.0, honestly optimistic, not 0.65).
    pub ewma: f64,
    pub samples: u32,
    /// Consecutive trailing fails (hysteresis input for P3; QQ2 recovery
    /// clears it with one pass).
    pub consecutive_failures: u32,
    /// Total pass count (QQ3's numerator: first-pass successes per euro).
    pub passes: u32,
}

impl LaneCap {
    /// True when the lane has cleared the F2 holdout for this class.
    pub fn established(&self) -> bool {
        self.samples >= MIN_SAMPLES
    }
}

#[derive(Debug, Default, PartialEq)]
pub struct CapsReport {
    /// Sorted by (lane_id, class). One entry per (lane, class) with ≥1 row.
    caps: Vec<LaneCap>,
}

impl CapsReport {
    /// Fold every OUTCOME row (seq order; decision rows are not capability
    /// evidence) into per-(lane, class) EWMA. Family joins are NOT done here
    /// — a family is a registry fact, so [`CapsReport::family_median`] and
    /// [`CapsReport::cold_start`] take the registry where they need it.
    pub fn from_rows(rows: &Loaded) -> Self {
        let mut fold: BTreeMap<(String, String), LaneCap> = BTreeMap::new();
        let mut sorted: Vec<&crate::ledger::ParsedRow> = rows.rows.iter().collect();
        sorted.sort_by_key(|p| p.seq);
        for parsed in sorted {
            let Row::Outcome(o) = &parsed.row else {
                continue;
            };
            let key = (o.lane_id.clone(), o.task_class.clone());
            let entry = fold.entry(key.clone()).or_insert_with(|| LaneCap {
                lane_id: key.0.clone(),
                class: key.1.clone(),
                ewma: 0.0,
                samples: 0,
                consecutive_failures: 0,
                passes: 0,
            });
            let obs = match o.outcome {
                Outcome::Pass => 1.0,
                Outcome::Fail => 0.0,
            };
            entry.ewma = if entry.samples == 0 {
                obs
            } else {
                EWMA_ALPHA * obs + (1.0 - EWMA_ALPHA) * entry.ewma
            };
            entry.samples += 1;
            match o.outcome {
                Outcome::Pass => {
                    entry.consecutive_failures = 0;
                    entry.passes += 1;
                }
                Outcome::Fail => entry.consecutive_failures += 1,
            }
        }
        let mut caps: Vec<LaneCap> = fold.into_values().collect();
        caps.sort_by(|a, b| (&a.lane_id, &a.class).cmp(&(&b.lane_id, &b.class)));
        CapsReport { caps }
    }

    pub fn lane_cap(&self, lane_id: &str, class: &str) -> Option<&LaneCap> {
        self.caps
            .iter()
            .find(|c| c.lane_id == lane_id && c.class == class)
    }

    /// Median EWMA of the family's ESTABLISHED lanes for `class` (samples ≥
    /// MIN_SAMPLES — the median of holdout lanes would just launder noise).
    /// Even count → mean of the two middles. None when the family has no
    /// established lane for the class: honest, never a guess.
    pub fn family_median(&self, lanes: &[Lane], family: &str, class: &str) -> Option<f64> {
        let members: Vec<&str> = lanes
            .iter()
            .filter(|l| l.family == family)
            .map(|l| l.id.as_str())
            .collect();
        let mut qs: Vec<f64> = self
            .caps
            .iter()
            .filter(|c| c.class == class && members.contains(&c.lane_id.as_str()))
            .filter(|c| c.established())
            .map(|c| c.ewma)
            .collect();
        if qs.is_empty() {
            return None;
        }
        qs.sort_by(|a, b| a.partial_cmp(b).expect("EWMA is always finite"));
        let n = qs.len();
        Some(if n % 2 == 1 {
            qs[n / 2]
        } else {
            (qs[n / 2 - 1] + qs[n / 2]) / 2.0
        })
    }

    /// Cold-start provisional quality for a lane that has NOT cleared the F2
    /// holdout on its own runs: the family median. `None` means "not
    /// provisional" — either established, never ran the class (no evidence,
    /// no guess), or the family has no established median to borrow.
    pub fn cold_start(&self, lanes: &[Lane], lane_id: &str, class: &str) -> Option<f64> {
        let own = self.lane_cap(lane_id, class)?;
        if own.established() {
            return None;
        }
        let family = lanes
            .iter()
            .find(|l| l.id == lane_id)
            .map(|l| l.family.as_str())?;
        // Never borrow from yourself: a lane with samples but < MIN_SAMPLES
        // must not inflate its own family median.
        if self.family_median(lanes, family, class).is_some() {
            let without: Vec<Lane> = lanes.iter().filter(|l| l.id != lane_id).cloned().collect();
            return self.family_median(&without, family, class);
        }
        None
    }

    /// OWN measurements only, as P1 [`Capability`] values — the feed into
    /// [`crate::route`]. Family medians are structurally absent: selection
    /// quality is always the lane's own verified history (F2 + R9).
    pub fn p1_caps(&self, lane_id: &str) -> BTreeMap<String, Capability> {
        self.caps
            .iter()
            .filter(|c| c.lane_id == lane_id)
            .map(|c| {
                (
                    c.class.clone(),
                    Capability {
                        quality: c.ewma,
                        samples: c.samples,
                    },
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::parse_stream;

    fn outcome_row(seq: u64, lane: &str, class: &str, pass: bool) -> String {
        format!(
            "{{\"seq\":{seq},\"ts\":\"t\",\"kind\":\"outcome\",\"card_id\":\"c\",\"task_class\":\"{class}\",\
             \"lane_id\":\"{lane}\",\"model\":\"m\",\"cost_tokens\":1,\"cost_usd_est\":0.001,\
             \"latency_ms\":10,\"verify_outcome\":\"{}\",\"escalated_to\":null}}",
            if pass { "pass" } else { "fail" }
        )
    }

    fn stream(lines: &[String]) -> Loaded {
        let text = lines.join("\n");
        parse_stream(&text)
    }

    fn lane(id: &str, family: &str) -> Lane {
        Lane {
            id: id.into(),
            family: family.into(),
            tier: crate::lane::LaneTier::Free,
            alive: true,
            cost_per_task_usd: 0.0,
            caps: BTreeMap::new(),
        }
    }

    #[test]
    fn ewma_folds_in_seq_order_with_first_obs_init() {
        let rows = stream(&[
            outcome_row(1, "gemini-a", "coding", true),
            outcome_row(2, "gemini-a", "coding", true),
            outcome_row(3, "gemini-a", "coding", false),
        ]);
        let rep = CapsReport::from_rows(&rows);
        let cap = rep.lane_cap("gemini-a", "coding").unwrap();
        // init 1.0; pass keeps 1.0; fail -> 0.3*0 + 0.7*1.0 = 0.7
        assert!((cap.ewma - 0.7).abs() < 1e-12, "got {}", cap.ewma);
        assert_eq!(cap.samples, 3);
        assert_eq!(cap.consecutive_failures, 1);
        assert_eq!(cap.passes, 2);
    }

    #[test]
    fn ewma_fail_first_is_honest_zero_then_recovers() {
        let rows = stream(&[
            outcome_row(1, "l", "coding", false),
            outcome_row(2, "l", "coding", true),
        ]);
        let rep = CapsReport::from_rows(&rows);
        let cap = rep.lane_cap("l", "coding").unwrap();
        // init 0.0 (no fake prior); pass -> 0.3*1 + 0.7*0 = 0.3 (QQ2 auto-heal)
        assert!((cap.ewma - 0.3).abs() < 1e-12);
        assert_eq!(cap.consecutive_failures, 0, "one pass clears hysteresis");
    }

    #[test]
    fn classes_are_independent_and_decision_rows_ignored() {
        let rows = stream(&[
            outcome_row(1, "l", "coding", true),
            outcome_row(2, "l", "writing", false),
            // A decision row between outcomes must not disturb any fold.
            "{\"seq\":3,\"ts\":\"t\",\"kind\":\"decision\",\"route_id\":\"r\",\"card_id\":\"c\",\
             \"task_class\":\"writing\",\"lane_id\":\"l\",\"tier\":\"free\",\
             \"cost_per_task_usd\":0,\"degraded\":false}"
                .to_string(),
            outcome_row(4, "l", "writing", true),
        ]);
        let rep = CapsReport::from_rows(&rows);
        let w = rep.lane_cap("l", "writing").unwrap();
        assert_eq!(w.samples, 2);
        assert!((w.ewma - 0.3).abs() < 1e-12, "fail then pass = 0.3");
        assert_eq!(rep.lane_cap("l", "coding").unwrap().samples, 1);
    }

    #[test]
    fn family_median_over_established_only_and_cold_start_borrows_it() {
        let mut lines = vec![];
        let mut seq = 0;
        // gemini-a: established (5 samples, all pass -> 1.0)
        for _ in 0..5 {
            seq += 1;
            lines.push(outcome_row(seq, "gemini-a", "coding", true));
        }
        // gemini-b: established, mixed -> 1,1,1,1,0 -> 0.7
        for i in 0..5 {
            seq += 1;
            lines.push(outcome_row(seq, "gemini-b", "coding", i < 4));
        }
        // gemini-c: holdout lane, 2 own samples
        for _ in 0..2 {
            seq += 1;
            lines.push(outcome_row(seq, "gemini-c", "coding", true));
        }
        let rows = stream(&lines);
        let lanes = vec![
            lane("gemini-a", "gemini"),
            lane("gemini-b", "gemini"),
            lane("gemini-c", "gemini"),
            lane("groq-x", "groq"),
        ];
        let rep = CapsReport::from_rows(&rows);

        // Median over {1.0, 0.7} = 0.85; groq-x (no established lanes) -> None.
        let m = rep.family_median(&lanes, "gemini", "coding").unwrap();
        assert!((m - 0.85).abs() < 1e-12);
        assert_eq!(rep.family_median(&lanes, "groq", "coding"), None);

        // Cold start: gemini-c borrows 0.85 WITHOUT contributing to it.
        let cs = rep.cold_start(&lanes, "gemini-c", "coding").unwrap();
        assert!((cs - 0.85).abs() < 1e-12);
        // Established lane is not provisional.
        assert_eq!(rep.cold_start(&lanes, "gemini-a", "coding"), None);
        // Unknown lane / unknown family: honest None.
        assert_eq!(rep.cold_start(&lanes, "ghost", "coding"), None);
    }

    #[test]
    fn p1_caps_never_leak_family_medians() {
        let mut lines = vec![];
        for i in 0..3 {
            lines.push(outcome_row(i + 1, "gemini-a", "coding", true));
        }
        let rows = stream(&lines);
        let rep = CapsReport::from_rows(&rows);
        let lanes = vec![lane("gemini-a", "gemini"), lane("gemini-b", "gemini")];
        // gemini-a has only 3 samples (holdout): no established family
        // member -> cold-start is honest None, and gemini-b's empty p1_caps
        // map carries no borrowed median either.
        assert_eq!(rep.cold_start(&lanes, "gemini-b", "coding"), None);
        let caps = rep.p1_caps("gemini-a");
        assert_eq!(caps.get("coding").unwrap().samples, 3);
        // A lane with zero own rows gets an EMPTY map — not a borrowed median.
        assert!(rep.p1_caps("gemini-b").is_empty());
    }

    #[test]
    fn hysteresis_counts_consecutive_fails() {
        let rows = stream(&[
            outcome_row(1, "l", "c", false),
            outcome_row(2, "l", "c", false),
            outcome_row(3, "l", "c", true),
            outcome_row(4, "l", "c", false),
        ]);
        let rep = CapsReport::from_rows(&rows);
        let cap = rep.lane_cap("l", "c").unwrap();
        assert_eq!(cap.consecutive_failures, 1);
        assert_eq!(cap.passes, 1);
        assert_eq!(cap.samples, 4);
    }
}
