//! P4 slice 4 — the LANE REGISTRY (council 2026-08-28, seats mistral +
//! cartographer, both real answers; premise-prover transport-failed and is
//! recorded as such). Rulings folded in:
//!
//! - **Q1 (unanimous)** `lanes.jsonl` is OPERATOR-AUTHORED — the policy.json
//!   law transposed to lanes: identity is fact but tier/cost are taste and
//!   budget, i.e. RULINGS; two writers on one file is the hazard the policy
//!   file already avoided. Collectors may PRINT a proposal; they never
//!   write here.
//! - **Q2 (unanimous)** STATIC until the operator re-rules. Same inputs ⇒
//!   same decision is the auditability law (F1); a registry that
//!   auto-refreshes mid-stream is a hidden clock that invalidates replay.
//! - **Q4 (unanimous)** an entry is exactly `{id, family, tier,
//!   cost_per_task_usd}` — NO `alive` (the caller's probe result, F1: no
//!   probing lives in this crate) and NO `caps` (ledger-derived through
//!   [`CapsReport::p1_caps`], F2: measurements are never authored).
//!
//! File law (inherits the organ's discipline):
//! - JSONL: ONE flat JSON object per line, parsed by the same no-nesting
//!   [`crate::ledger::parse_object`] the policy file uses — a hand-edited
//!   nested line fails closed instead of half-loading.
//! - Unknown field is MALFORMED (a typo must never silently drop the entry
//!   it was trying to change — policy-file law). Unknown tier — including
//!   `"droid"` — is refused by [`LaneTier::parse`] (O2 executable law).
//! - Duplicate lane id, negative/non-finite cost, or a registry with zero
//!   entries: malformed. The router never repairs, never guesses.
//! - The router NEVER writes this file. Missing file is `Ok(None)`: an
//!   honest "not yet ruled" the CLI surfaces as a fail-closed usage stop,
//!   never as routing on an empty universe.

use std::collections::BTreeSet;
use std::path::Path;

use crate::lane::{Lane, LaneTier};
use crate::ledger::{parse_object, Val};
use crate::stats::CapsReport;

/// One authored lane entry (Q4 shape — nothing else).
#[derive(Debug, Clone, PartialEq)]
pub struct LaneEntry {
    pub id: String,
    pub family: String,
    pub tier: LaneTier,
    pub cost_per_task_usd: f64,
}

/// The authored lane universe, sorted by id (deterministic wire order).
#[derive(Debug, Clone, PartialEq)]
pub struct LaneRegistry {
    lanes: Vec<LaneEntry>,
}

#[derive(Debug, PartialEq)]
pub enum RegistryErr {
    /// The file exists but cannot be read. Text, not io::Error — io::Error
    /// is not PartialEq (same law as PolicyFileErr/AlertErr).
    Read(String),
    /// The file exists but is not a loadable registry. One honest string;
    /// the CLI prints it and fails closed (exit 2 — construction defect,
    /// never a routing decision).
    Malformed(String),
}

/// Load the registry file. `Ok(None)` = no file at `path` — the operator
/// has not ruled lanes yet; the caller must fail closed with that exact
/// message, never route on an empty universe. `Err` = the file exists and
/// does not rule cleanly.
pub fn load_registry(path: &Path) -> Result<Option<LaneRegistry>, RegistryErr> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(RegistryErr::Read(e.to_string())),
    };
    parse_registry(&text).map(Some)
}

/// Pure text -> registry (the file is only a container; every parse law is
/// testable without I/O). One flat object per non-empty line.
pub fn parse_registry(text: &str) -> Result<LaneRegistry, RegistryErr> {
    let mut lanes: Vec<LaneEntry> = Vec::new();
    for (idx, raw) in text.lines().enumerate() {
        let line_no = idx + 1;
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let map = parse_object(line).map_err(|e| {
            RegistryErr::Malformed(format!("line {line_no}: not a flat JSON object: {e}"))
        })?;
        let entry = entry_from_map(&map)
            .map_err(|why| RegistryErr::Malformed(format!("line {line_no}: {why}")))?;
        if lanes.iter().any(|l| l.id == entry.id) {
            return Err(RegistryErr::Malformed(format!(
                "line {line_no}: duplicate lane id {:?}",
                entry.id
            )));
        }
        lanes.push(entry);
    }
    if lanes.is_empty() {
        return Err(RegistryErr::Malformed(
            "no lane entries — a registry that exists must name at least one lane".into(),
        ));
    }
    lanes.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(LaneRegistry { lanes })
}

/// Field law: exactly id (non-empty string), family (non-empty string),
/// tier (vocabulary word), cost_per_task_usd (finite number >= 0).
/// Everything else — including a nested value or an unknown key — is
/// malformed.
fn entry_from_map(map: &std::collections::BTreeMap<String, Val>) -> Result<LaneEntry, String> {
    let mut id: Option<String> = None;
    let mut family: Option<String> = None;
    let mut tier: Option<LaneTier> = None;
    let mut cost: Option<f64> = None;
    for (k, v) in map {
        match k.as_str() {
            "id" => {
                id = Some(string_field(v, k)?);
            }
            "family" => {
                family = Some(string_field(v, k)?);
            }
            "tier" => {
                let word = string_field(v, k)?;
                tier = Some(LaneTier::parse(&word).ok_or_else(|| {
                    format!(
                        "unknown tier {word:?} (taxonomy: local|free|mid|premium; droid is refused — O2)"
                    )
                })?);
            }
            "cost_per_task_usd" => {
                let n = match v {
                    Val::Num(n) => *n,
                    _ => return Err(format!("field {k:?}: cost must be a number")),
                };
                if !n.is_finite() || n < 0.0 {
                    return Err(format!(
                        "field {k:?}: cost must be finite and >= 0 (got {n})"
                    ));
                }
                cost = Some(n);
            }
            other => {
                return Err(format!(
                    "unknown field {other:?} (vocabulary: id | family | tier | cost_per_task_usd)"
                ))
            }
        }
    }
    Ok(LaneEntry {
        id: id.ok_or("missing field \"id\"")?,
        family: family.ok_or("missing field \"family\"")?,
        tier: tier.ok_or("missing field \"tier\"")?,
        cost_per_task_usd: cost.ok_or("missing field \"cost_per_task_usd\"")?,
    })
}

fn string_field(v: &Val, k: &str) -> Result<String, String> {
    match v {
        Val::Str(s) if !s.is_empty() => Ok(s.clone()),
        Val::Str(_) => Err(format!("field {k:?}: must be a non-empty string")),
        _ => Err(format!("field {k:?}: must be a string")),
    }
}

impl LaneRegistry {
    /// The authored universe, id-sorted.
    pub fn entries(&self) -> &[LaneEntry] {
        &self.lanes
    }

    /// Is this id in the authored universe? (The CLI refuses an `--alive`
    /// id the registry does not know — a lane outside the ruling is a
    /// caller typo, not a routing input.)
    pub fn knows(&self, id: &str) -> bool {
        self.lanes.iter().any(|l| l.id == id)
    }

    /// Merge the authored entries with MEASURED capability (F2: own
    /// measurements only, folded from outcome rows) and the caller's
    /// liveness set (F1: alive is the caller's probe result — `--alive` or
    /// the named `--assume-alive` assumption; never probed here).
    pub fn lanes(&self, caps: &CapsReport, alive: &BTreeSet<String>) -> Vec<Lane> {
        self.lanes
            .iter()
            .map(|e| Lane {
                id: e.id.clone(),
                family: e.family.clone(),
                tier: e.tier,
                alive: alive.contains(&e.id),
                cost_per_task_usd: e.cost_per_task_usd,
                caps: caps.p1_caps(&e.id),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::{Loaded, Outcome, OutcomeRow, ParsedRow, Row};

    const GOOD: &str = concat!(
        "{\"id\":\"kamane\",\"family\":\"pi\",\"tier\":\"free\",\"cost_per_task_usd\":0}\n",
        "{\"id\":\"ollama-local\",\"family\":\"ollama\",\"tier\":\"local\",\"cost_per_task_usd\":0.0004}\n",
        "\n",
        "{\"id\":\"gemini\",\"family\":\"google\",\"tier\":\"mid\",\"cost_per_task_usd\":0.011}\n",
    );

    #[test]
    fn parses_and_sorts_by_id() {
        let r = parse_registry(GOOD).unwrap();
        let ids: Vec<&str> = r.entries().iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["gemini", "kamane", "ollama-local"]);
        assert_eq!(r.entries()[1].tier, LaneTier::Free);
    }

    #[test]
    fn empty_text_is_malformed() {
        assert!(parse_registry("").is_err());
        assert!(parse_registry("\n \n").is_err());
    }

    #[test]
    fn unknown_field_is_malformed() {
        let t = "{\"id\":\"a\",\"family\":\"f\",\"tier\":\"free\",\"cost_per_task_usd\":0,\"alive\":true}";
        let e = parse_registry(t).unwrap_err();
        match e {
            RegistryErr::Malformed(m) => assert!(m.contains("unknown field \"alive\""), "{m}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn droid_tier_is_refused_o2() {
        let t = "{\"id\":\"a\",\"family\":\"f\",\"tier\":\"droid\",\"cost_per_task_usd\":0}";
        let e = parse_registry(t).unwrap_err();
        match e {
            RegistryErr::Malformed(m) => assert!(m.contains("droid is refused"), "{m}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn negative_cost_is_malformed() {
        let t = "{\"id\":\"a\",\"family\":\"f\",\"tier\":\"free\",\"cost_per_task_usd\":-1}";
        assert!(
            parse_registry(t).unwrap_err()
                == RegistryErr::Malformed(
                    "line 1: field \"cost_per_task_usd\": cost must be finite and >= 0 (got -1)"
                        .into()
                )
        );
    }

    #[test]
    fn duplicate_id_is_malformed() {
        let t = concat!(
            "{\"id\":\"a\",\"family\":\"f\",\"tier\":\"free\",\"cost_per_task_usd\":0}\n",
            "{\"id\":\"a\",\"family\":\"g\",\"tier\":\"mid\",\"cost_per_task_usd\":1}\n",
        );
        let e = parse_registry(t).unwrap_err();
        match e {
            RegistryErr::Malformed(m) => assert!(m.contains("duplicate lane id"), "{m}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn missing_field_is_malformed() {
        let t = "{\"id\":\"a\",\"family\":\"f\",\"tier\":\"free\"}";
        let e = parse_registry(t).unwrap_err();
        match e {
            RegistryErr::Malformed(m) => assert!(m.contains("cost_per_task_usd"), "{m}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn missing_file_is_none() {
        let p = std::env::temp_dir().join("rtr-registry-no-such-file.jsonl");
        let _ = std::fs::remove_file(&p);
        assert_eq!(load_registry(&p).unwrap(), None);
    }

    /// lanes() merges authored data with measured caps and caller liveness:
    /// measured lane gets its capability, unmeasured stays empty (never
    /// guessed), alive only for ids in the caller's set.
    #[test]
    fn lanes_merge_caps_and_liveness() {
        let r = parse_registry(GOOD).unwrap();
        let rows = Loaded {
            rows: vec![
                ParsedRow {
                    line: 1,
                    seq: 1,
                    row: Row::Outcome(OutcomeRow {
                        card_id: "C1".into(),
                        task_class: "chair".into(),
                        lane_id: "kamane".into(),
                        model: "glm-5.2".into(),
                        cost_tokens: 10,
                        cost_usd_est: 0.0,
                        latency_ms: 500,
                        outcome: Outcome::Pass,
                        escalated_to: None,
                    }),
                },
                ParsedRow {
                    line: 2,
                    seq: 2,
                    row: Row::Outcome(OutcomeRow {
                        card_id: "C2".into(),
                        task_class: "chair".into(),
                        lane_id: "kamane".into(),
                        model: "glm-5.2".into(),
                        cost_tokens: 10,
                        cost_usd_est: 0.0,
                        latency_ms: 500,
                        outcome: Outcome::Pass,
                        escalated_to: None,
                    }),
                },
            ],
            bad: vec![],
        };
        let caps = CapsReport::from_rows(&rows);
        let mut alive = BTreeSet::new();
        alive.insert("kamane".to_string());
        let lanes = r.lanes(&caps, &alive);
        assert_eq!(lanes.len(), 3);
        let kamane = lanes.iter().find(|l| l.id == "kamane").unwrap();
        assert!(kamane.alive);
        assert_eq!(kamane.caps.get("chair").map(|c| c.samples), Some(2));
        let gemini = lanes.iter().find(|l| l.id == "gemini").unwrap();
        assert!(!gemini.alive);
        assert!(gemini.caps.is_empty(), "never guessed");
        // Registry universe gate: an id outside the ruling is unknown.
        assert!(!r.knows("bitute"));
    }
}
