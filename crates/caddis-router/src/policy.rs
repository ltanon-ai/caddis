//! Routing law as DATA (F5, F6, F2). The policy carries no behavior — P1
//! only obeys it; floor/tier changes reach it through operator-gated paths
//! (P4 warden policy wiring, P5 world settings propose->operator-confirm),
//! which is why every mutator here is plain and unconcealed.
//!
//! Defaults are PRIORS, not rulings: floors skeptic 0.85 / chair 0.70 (F6
//! converged), Secret/Pii local-only tier allowlists (F5 conservative prior —
//! the warden policy file may loosen them, the default never guesses loose),
//! min samples 5 (F2, both council lanes).

use std::collections::BTreeMap;

use crate::lane::{DataClass, LaneTier};

/// F2: measured runs before a lane may enter cheap-selection.
pub const DEFAULT_MIN_SAMPLES: u32 = 5;

#[derive(Debug, Clone, PartialEq)]
pub struct RoutePolicy {
    /// task class -> minimum quality (0 < q <= 1) for selection (F6/R9:
    /// floors are SELECTION thresholds). A class missing here fails closed —
    /// thresholds are never guessed.
    floors: BTreeMap<String, f64>,
    /// R1 (quorum, overriding QQ1b): task class -> STATIC per-chain budget
    /// ceiling in USD for escalation. NOT a multiple of a failing lane's
    /// baseline (2x ~zero makes premium mathematically unreachable). EMPTY
    /// by default: budgets are operator-set data, never guessed — a class
    /// without a ceiling fails escalation CLOSED.
    cost_ceilings: BTreeMap<String, f64>,
    /// data class -> tiers permitted to serve it (F5). Filter runs BEFORE
    /// cost selection; an empty result is fail-closed.
    tier_allow: BTreeMap<DataClass, Vec<LaneTier>>,
    /// F2 sample gate for cheap-pool entry.
    min_samples: u32,
}
impl Default for RoutePolicy {
    fn default() -> Self {
        let mut floors = BTreeMap::new();
        floors.insert("skeptic".to_string(), 0.85);
        floors.insert("chair".to_string(), 0.70);
        let mut tier_allow = BTreeMap::new();
        tier_allow.insert(DataClass::Secret, vec![LaneTier::Local]);
        tier_allow.insert(DataClass::Pii, vec![LaneTier::Local]);
        tier_allow.insert(
            DataClass::Internal,
            vec![LaneTier::Local, LaneTier::Free, LaneTier::Mid],
        );
        tier_allow.insert(
            DataClass::Public,
            vec![
                LaneTier::Local,
                LaneTier::Free,
                LaneTier::Mid,
                LaneTier::Premium,
            ],
        );
        RoutePolicy {
            floors,
            // R1: no guessed budgets — empty until the operator sets one.
            cost_ceilings: BTreeMap::new(),
            tier_allow,
            min_samples: DEFAULT_MIN_SAMPLES,
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum PolicyErr {
    /// Floor outside (0..=1] — malformed policy, never loaded, never routed.
    BadFloor(String),
    /// R1: a cost ceiling that is not a finite positive dollar amount.
    BadCeiling(String),
    /// A data class with an empty tier allowlist can only fail closed; an
    /// empty allowlist is a construction defect, not a lock.
    EmptyAllow(DataClass),
}

impl RoutePolicy {
    pub fn floor(&self, class: &str) -> Option<f64> {
        self.floors.get(class).copied()
    }

    /// F6: ALL floor adjustments require operator sign-off — enforced by the
    /// write path (P4/P5), not hidden here.
    pub fn set_floor(&mut self, class: &str, floor: f64) {
        self.floors.insert(class.to_string(), floor);
    }

    /// R1: the class's static per-chain escalation budget (USD). `None`
    /// means NO CEILING RULED — escalation for the class fails closed
    /// (budgets are never guessed; the operator sets them via the P5
    /// propose->confirm surface).
    pub fn ceiling(&self, class: &str) -> Option<f64> {
        self.cost_ceilings.get(class).copied()
    }

    /// R1/F6-adjacent: ALL ceiling adjustments require operator sign-off —
    /// enforced by the write path (P4/P5), not hidden here.
    pub fn set_ceiling(&mut self, class: &str, usd: f64) {
        self.cost_ceilings.insert(class.to_string(), usd);
    }

    pub fn permits(&self, data_class: DataClass, tier: LaneTier) -> bool {
        self.tier_allow
            .get(&data_class)
            .is_some_and(|v| v.contains(&tier))
    }

    /// F5: the data-class -> tier mapping is ruled in the warden policy file;
    /// the router obeys. An unknown (missing) data class permits NOTHING.
    pub fn set_tiers(&mut self, data_class: DataClass, tiers: Vec<LaneTier>) {
        self.tier_allow.insert(data_class, tiers);
    }

    pub fn min_samples(&self) -> u32 {
        self.min_samples
    }

    pub fn set_min_samples(&mut self, n: u32) {
        self.min_samples = n;
    }

    /// R7 seed: validate the policy data itself — a floor outside (0..=1] or
    /// an empty allowlist fails closed at LOAD time, before any routing runs.
    pub fn validate(&self) -> Result<(), PolicyErr> {
        for (class, floor) in &self.floors {
            if !floor.is_finite() || *floor <= 0.0 || *floor > 1.0 {
                return Err(PolicyErr::BadFloor(class.clone()));
            }
        }
        // R1: a ceiling must be a finite positive dollar amount — anything
        // else is an unruled budget, and unruled budgets never route.
        for (class, ceiling) in &self.cost_ceilings {
            if !ceiling.is_finite() || *ceiling <= 0.0 {
                return Err(PolicyErr::BadCeiling(class.clone()));
            }
        }
        for (data_class, tiers) in &self.tier_allow {
            if tiers.is_empty() {
                return Err(PolicyErr::EmptyAllow(*data_class));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_floors_are_the_f6_priors() {
        let p = RoutePolicy::default();
        assert_eq!(p.floor("skeptic"), Some(0.85));
        assert_eq!(p.floor("chair"), Some(0.70));
        assert_eq!(p.floor("unknown-class"), None);
        assert_eq!(p.min_samples(), 5);
        assert!(p.validate().is_ok());
    }

    #[test]
    fn default_tier_allowlists_are_conservative() {
        let p = RoutePolicy::default();
        assert!(p.permits(DataClass::Secret, LaneTier::Local));
        assert!(!p.permits(DataClass::Secret, LaneTier::Free));
        assert!(!p.permits(DataClass::Pii, LaneTier::Mid));
        assert!(p.permits(DataClass::Internal, LaneTier::Free));
        assert!(!p.permits(DataClass::Internal, LaneTier::Premium));
        assert!(p.permits(DataClass::Public, LaneTier::Premium));
    }

    #[test]
    fn validate_rejects_bad_floor_and_empty_allow() {
        let mut p = RoutePolicy::default();
        p.set_floor("skeptic", 1.5);
        assert_eq!(p.validate(), Err(PolicyErr::BadFloor("skeptic".into())));
        p.set_floor("skeptic", 0.9);
        p.set_tiers(DataClass::Public, vec![]);
        assert_eq!(p.validate(), Err(PolicyErr::EmptyAllow(DataClass::Public)));
    }

    #[test]
    fn ceilings_default_empty_and_validate_their_shape() {
        let mut p = RoutePolicy::default();
        assert_eq!(p.ceiling("chair"), None, "no guessed budgets by default");
        p.set_ceiling("chair", 2.5);
        assert_eq!(p.ceiling("chair"), Some(2.5));
        assert!(p.validate().is_ok());

        p.set_ceiling("chair", 0.0);
        assert_eq!(p.validate(), Err(PolicyErr::BadCeiling("chair".into())));
        p.set_ceiling("chair", f64::NAN);
        assert_eq!(p.validate(), Err(PolicyErr::BadCeiling("chair".into())));
    }
}
