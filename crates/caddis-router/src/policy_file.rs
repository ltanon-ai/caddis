//! P4 slice 2 — the WARDEN POLICY FILE (F5: "mapping ruled in the warden
//! policy file; router obeys"). The file is the ruling HOME; the builtin
//! [`RoutePolicy::default`] is only the conservative prior the router runs
//! on UNTIL a file exists.
//!
//! File law:
//! - FLAT JSON object — the crate's proven no-nesting subset (the ledger
//!   parser rejects nesting by design; the policy file inherits that law,
//!   so a hand-edited nested file fails closed instead of half-loading).
//! - Key vocabulary, nothing else: `floor.<task_class>` (number),
//!   `ceiling.<task_class>` (USD number), `tier.<data_class>`
//!   (comma-separated tier words), `min_samples` (integer >= 1).
//!   An unknown key is a MALFORMED file — a typo must never silently drop
//!   the ruling it was trying to change.
//! - THE FILE IS THE WHOLE POLICY: whatever it does not rule, the router
//!   REFUSES — floors it omits are absent (every such class fails
//!   [`RouteErr::NoFloorForClass`]), data classes it omits permit NOTHING.
//!   Defaults are never silently mixed into an authored file. The one
//!   exception is `min_samples`: F2's 5 is a converged constant, not an
//!   operator ruling, so an omitted `min_samples` keeps
//!   [`DEFAULT_MIN_SAMPLES`].
//! - A file that exists and does not parse+validate is an ERROR — routing
//!   must refuse (fail closed past an authored file); falling back to the
//!   builtin defaults would route around the warden's ruling.
//! - The router NEVER writes this file. It is authored by the
//!   operator/warden path (P5 propose->confirm); `caddis-router policy`
//!   only audits what would be obeyed.

use std::collections::BTreeMap;
use std::path::Path;

use crate::lane::{DataClass, LaneTier};
use crate::ledger::{parse_object, Val};
use crate::policy::{RoutePolicy, DEFAULT_MIN_SAMPLES};

#[derive(Debug, PartialEq)]
pub enum PolicyFileErr {
    /// The file exists but cannot be read (permissions, lock). The message
    /// is the io error's own string — kept as text because io::Error is not
    /// PartialEq (same law as AlertErr).
    Read(String),
    /// The file exists but is not a loadable ruling. One honest string; the
    /// CLI prints it as the single finding and exits non-zero.
    Malformed(String),
}

/// Load the policy file. `Ok(None)` = no file at `path` — the caller runs
/// on the builtin conservative defaults (visible via `caddis-router
/// policy`). `Err` = the file exists and does not rule cleanly — the
/// caller must refuse to route.
pub fn load_policy(path: &Path) -> Result<Option<RoutePolicy>, PolicyFileErr> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(PolicyFileErr::Read(e.to_string())),
    };
    parse_policy(&text).map(Some)
}

/// Pure text -> policy (the file is only a container; every parse law is
/// testable without I/O).
pub fn parse_policy(text: &str) -> Result<RoutePolicy, PolicyFileErr> {
    let map = parse_object(text)
        .map_err(|e| PolicyFileErr::Malformed(format!("not a flat JSON object: {e}")))?;

    let mut floors: BTreeMap<String, f64> = BTreeMap::new();
    let mut ceilings: BTreeMap<String, f64> = BTreeMap::new();
    let mut tier_allow: BTreeMap<DataClass, Vec<LaneTier>> = BTreeMap::new();
    let mut min_samples: Option<u32> = None;

    for (k, v) in &map {
        if let Some(class) = k.strip_prefix("floor.") {
            floors.insert(class.to_string(), num(v, k)?);
        } else if let Some(class) = k.strip_prefix("ceiling.") {
            ceilings.insert(class.to_string(), num(v, k)?);
        } else if let Some(word) = k.strip_prefix("tier.") {
            let dc = DataClass::parse(word).ok_or_else(|| {
                PolicyFileErr::Malformed(format!(
                    "field {k:?}: unknown data class {word:?} (vocabulary: secret|pii|internal|public)"
                ))
            })?;
            let list = match v {
                Val::Str(s) => s.as_str(),
                _ => {
                    return Err(PolicyFileErr::Malformed(format!(
                        "field {k:?}: tier list must be a string (\"local,free\")"
                    )))
                }
            };
            let mut tiers = Vec::new();
            for word in list.split(',') {
                let w = word.trim();
                if w.is_empty() {
                    return Err(PolicyFileErr::Malformed(format!(
                        "field {k:?}: empty tier word in {list:?}"
                    )));
                }
                let t = LaneTier::parse(w).ok_or_else(|| {
                    PolicyFileErr::Malformed(format!(
                        "field {k:?}: unknown tier {w:?} (taxonomy: local|free|mid|premium; droid is refused — O2)"
                    ))
                })?;
                tiers.push(t);
            }
            tier_allow.insert(dc, tiers);
        } else if k == "min_samples" {
            let n = match v {
                Val::Num(n) if *n >= 1.0 && n.fract() == 0.0 && *n <= u32::MAX as f64 => {
                    *n as u32
                }
                _ => {
                    return Err(PolicyFileErr::Malformed(format!(
                        "field 'min_samples': must be an integer >= 1 (F2 constant is {DEFAULT_MIN_SAMPLES})"
                    )))
                }
            };
            min_samples = Some(n);
        } else {
            return Err(PolicyFileErr::Malformed(format!(
                "unknown field {k:?} (vocabulary: floor.<class> | ceiling.<class> | tier.<data_class> | min_samples)"
            )));
        }
    }

    // A file that rules no data class rules nothing routable — that is a
    // construction defect, not a lock (same law as PolicyErr::EmptyAllow).
    if tier_allow.is_empty() {
        return Err(PolicyFileErr::Malformed(
            "no tier.<data_class> ruled — a policy file must rule at least one data class".into(),
        ));
    }

    let policy = RoutePolicy::from_parts(
        floors,
        ceilings,
        tier_allow,
        min_samples.unwrap_or(DEFAULT_MIN_SAMPLES),
    );
    policy
        .validate()
        .map_err(|e| PolicyFileErr::Malformed(format!("policy data invalid: {e}")))?;
    Ok(policy)
}

/// Deterministic wire form: sorted keys, tiers in taxonomy order (Local,
/// Free, Mid, Premium — canonical, never alphabetical). This is the exact
/// text `caddis-router policy --print` shows and [`parse_policy`] accepts
/// back, so what the operator AUDITS is what the router OBEYS.
pub fn encode_policy(p: &RoutePolicy) -> String {
    let mut fields: Vec<(String, String)> = Vec::new();
    for (class, usd) in p.ceilings() {
        fields.push((format!("ceiling.{class}"), usd.to_string()));
    }
    for (class, floor) in p.floors() {
        fields.push((format!("floor.{class}"), floor.to_string()));
    }
    fields.push(("min_samples".to_string(), p.min_samples().to_string()));
    let taxonomy = [
        LaneTier::Local,
        LaneTier::Free,
        LaneTier::Mid,
        LaneTier::Premium,
    ];
    for (dc, tiers) in p.tier_allow() {
        let words: Vec<&str> = taxonomy
            .iter()
            .filter(|t| tiers.contains(t))
            .map(|t| t.as_str())
            .collect();
        fields.push((
            format!("tier.{}", dc.as_str()),
            format!("\"{}\"", words.join(",")),
        ));
    }
    fields.sort_by(|a, b| a.0.cmp(&b.0));
    let body: String = fields
        .iter()
        .map(|(k, v)| format!("\"{}\":{}", crate::ledger::esc(k), v))
        .collect::<Vec<_>>()
        .join(",");
    format!("{{{}}}", body)
}

/// A numeric field — anything else is malformed.
fn num(v: &Val, k: &str) -> Result<f64, PolicyFileErr> {
    match v {
        Val::Num(n) => Ok(*n),
        _ => Err(PolicyFileErr::Malformed(format!(
            "field {k:?}: expected a number"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_round_trip_through_the_wire() {
        let text = encode_policy(&RoutePolicy::default());
        assert_eq!(parse_policy(&text), Ok(RoutePolicy::default()));
    }

    #[test]
    fn encode_is_the_documented_snapshot() {
        // What the operator audits is what the router obeys — pin the exact
        // bytes so any encoding drift is a deliberate act of man.
        assert_eq!(
            encode_policy(&RoutePolicy::default()),
            "{\"floor.chair\":0.7,\"floor.skeptic\":0.85,\"min_samples\":5,\
             \"tier.internal\":\"local,free,mid\",\"tier.pii\":\"local\",\
             \"tier.public\":\"local,free,mid,premium\",\"tier.secret\":\"local\"}"
        );
    }

    #[test]
    fn absent_file_is_none_not_an_error() {
        let missing = std::env::temp_dir().join(format!(
            "caddis-router-policy-absent-{}-{}",
            std::process::id(),
            line!()
        ));
        assert_eq!(load_policy(&missing), Ok(None));
    }

    #[test]
    fn the_file_is_the_whole_policy_never_mixed_with_defaults() {
        // Rules ONE data class and ONE floor: everything else the defaults
        // carry is GONE — floors empty, other classes permit nothing.
        let p = parse_policy("{\"tier.secret\":\"local\",\"floor.skeptic\":0.9,\"min_samples\":6}")
            .expect("loads");
        assert_eq!(p.floor("skeptic"), Some(0.9));
        assert_eq!(p.min_samples(), 6);
        assert_eq!(p.floor("chair"), None, "default floor must not leak in");
        assert!(!p.permits(DataClass::Public, LaneTier::Local));
        assert!(p.permits(DataClass::Secret, LaneTier::Local));
    }

    #[test]
    fn min_samples_omitted_keeps_the_converged_constant() {
        let p = parse_policy("{\"tier.public\":\"local\"}").expect("loads");
        assert_eq!(p.min_samples(), DEFAULT_MIN_SAMPLES);
    }

    #[test]
    fn unknown_field_fails_closed() {
        let e = parse_policy("{\"floors.skeptic\":0.9}").unwrap_err();
        assert!(format!("{e:?}").contains("unknown field"), "{e:?}");
    }

    #[test]
    fn garbage_fails_closed() {
        assert!(parse_policy("not json at all").is_err());
        assert!(parse_policy("{}").is_err(), "no data class ruled");
    }

    #[test]
    fn nesting_is_rejected_by_the_flat_subset() {
        // The ledger parser refuses arrays/objects as VALUES — the policy
        // file inherits that law instead of half-supporting nesting.
        let e = parse_policy("{\"tier.secret\":[\"local\"]}").unwrap_err();
        assert!(format!("{e:?}").contains("flat JSON object"), "{e:?}");
    }

    #[test]
    fn o2_droid_is_refused_through_the_file() {
        let e = parse_policy("{\"tier.secret\":\"droid\"}").unwrap_err();
        assert!(format!("{e:?}").contains("droid is refused"), "{e:?}");
    }

    #[test]
    fn unknown_tier_word_fails_closed() {
        let e = parse_policy("{\"tier.secret\":\"local,cloud\"}").unwrap_err();
        assert!(format!("{e:?}").contains("unknown tier"), "{e:?}");
    }

    #[test]
    fn unknown_data_class_fails_closed() {
        let e = parse_policy("{\"tier.top_secret\":\"local\"}").unwrap_err();
        assert!(format!("{e:?}").contains("unknown data class"), "{e:?}");
    }

    #[test]
    fn empty_tier_words_fail_closed() {
        assert!(parse_policy("{\"tier.secret\":\"\"}").is_err());
        assert!(parse_policy("{\"tier.secret\":\"local,\"}").is_err());
        assert!(parse_policy("{\"tier.secret\":\"local,,free\"}").is_err());
    }

    #[test]
    fn tier_words_tolerate_spacing_and_case() {
        let p = parse_policy("{\"tier.internal\": \" Local, FREE , mid \"}").expect("loads");
        assert!(p.permits(DataClass::Internal, LaneTier::Free));
        assert!(!p.permits(DataClass::Internal, LaneTier::Premium));
    }

    #[test]
    fn bad_numbers_fail_closed_via_validate() {
        // floor > 1 / <= 0 and non-positive ceilings are caught by the SAME
        // validate() the in-memory mutators go through — one law, two doors.
        assert!(parse_policy("{\"tier.public\":\"local\",\"floor.x\":1.5}").is_err());
        assert!(parse_policy("{\"tier.public\":\"local\",\"floor.x\":0}").is_err());
        assert!(parse_policy("{\"tier.public\":\"local\",\"floor.x\":\"high\"}").is_err());
        assert!(parse_policy("{\"tier.public\":\"local\",\"ceiling.y\":-1}").is_err());
    }

    #[test]
    fn min_samples_must_be_one_or_more() {
        assert!(parse_policy("{\"tier.public\":\"local\",\"min_samples\":0}").is_err());
        assert!(parse_policy("{\"tier.public\":\"local\",\"min_samples\":1.5}").is_err());
        let p = parse_policy("{\"tier.public\":\"local\",\"min_samples\":12}").expect("loads");
        assert_eq!(p.min_samples(), 12);
    }

    #[test]
    fn empty_floors_is_legal_and_fail_closed() {
        // Ruling "no class has a floor" is legal DATA — every class then
        // fails NoFloorForClass at route time. Visible, never guessed.
        let p = parse_policy("{\"tier.public\":\"local\"}").expect("loads");
        assert_eq!(p.floor("skeptic"), None);
        assert!(p.validate().is_ok());
    }

    #[test]
    fn ceilings_survive_the_round_trip() {
        let text = "{\"ceiling.coding\":1.5,\"floor.skeptic\":0.8,\
                    \"tier.public\":\"local,free,mid,premium\"}";
        let p = parse_policy(text).expect("loads");
        assert_eq!(p.ceiling("coding"), Some(1.5));
        assert_eq!(parse_policy(&encode_policy(&p)), Ok(p));
    }
}
