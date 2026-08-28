//! Lane model (R4 taxonomy + F5 vocabulary). Tiers carry the free/local-first
//! ordering: `Local < Free < Mid < Premium`, so tier preference IS the Ord.
//! There is no Droid variant and [`LaneTier::parse`] refuses the string —
//! O2 is executable law, not a comment.

use std::collections::BTreeMap;

/// R4 tier taxonomy. Ord = preference order for equal cost (O3 free/local first).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LaneTier {
    Local,
    Free,
    Mid,
    Premium,
}

impl LaneTier {
    /// Registry-feed parser. Returns `None` for ANY unknown tier — including
    /// `"droid"` (O2: no droid lanes) — so external lane data can never smuggle
    /// a forbidden tier past the type system.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "local" => Some(LaneTier::Local),
            "free" => Some(LaneTier::Free),
            "mid" => Some(LaneTier::Mid),
            "premium" => Some(LaneTier::Premium),
            _ => None,
        }
    }
}

/// F5 data-class vocabulary. Secret is the strictest (local-only by default
/// policy); Public the loosest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DataClass {
    Secret,
    Pii,
    Internal,
    Public,
}

/// Measured capability of one lane for one task class (F2: the sample count
/// gates cheap-pool entry at N >= 5). Quality is a 0..=1 verify-outcome
/// score; how it is derived (EWMA over verify outcomes) is P2's job — P1
/// consumes the number as data.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Capability {
    pub quality: f64,
    pub samples: u32,
}

/// One dispatch lane as routing data. `alive` is the caller's liveness probe
/// result, `cost_per_task_usd` the measured/estimated dollars per task of
/// this class (O3: cost compares WITHIN a class, never across).
#[derive(Debug, Clone, PartialEq)]
pub struct Lane {
    pub id: String,
    pub family: String,
    pub tier: LaneTier,
    pub alive: bool,
    pub cost_per_task_usd: f64,
    /// class -> measured capability. Absent class = lane never ran it = not
    /// suitable (never guessed).
    pub caps: BTreeMap<String, Capability>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn o2_droid_is_unparseable_law() {
        assert!(LaneTier::parse("droid").is_none());
        assert!(LaneTier::parse("DROID").is_none());
        assert_eq!(LaneTier::parse(" local "), Some(LaneTier::Local));
        assert_eq!(LaneTier::parse("Free"), Some(LaneTier::Free));
        assert_eq!(LaneTier::parse("mid"), Some(LaneTier::Mid));
        assert_eq!(LaneTier::parse("premium"), Some(LaneTier::Premium));
    }

    #[test]
    fn tier_ord_is_free_local_first() {
        // O3 tiebreak order: Local < Free < Mid < Premium.
        assert!(LaneTier::Local < LaneTier::Free);
        assert!(LaneTier::Free < LaneTier::Mid);
        assert!(LaneTier::Mid < LaneTier::Premium);
    }
}
