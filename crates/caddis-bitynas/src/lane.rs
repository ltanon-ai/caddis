//! lane.rs — the O2 lane guard, replicated from
//! `caddis-router/src/lane.rs LaneTier::parse` by CARD-BITYNAS-1 order
//! (do NOT depend on the router crate). Copies law: if the tier vocabulary
//! ever moves, the fix lands in BOTH copies.

/// Is `tier` an allowed lane tier?
///
/// Closed vocabulary `local | free | mid | premium` (case-insensitive,
/// surrounding whitespace trimmed — router parity). `"droid"` is refused
/// BY NAME with the O2 law in the message: no droid lanes, ever — O2 is
/// executable law, not a comment. Every other unknown tier is refused too,
/// so external lane data can never smuggle a forbidden or nonsense tier
/// past the guard.
pub fn lane_allowed(tier: &str) -> Result<(), String> {
    match tier.trim().to_ascii_lowercase().as_str() {
        "local" | "free" | "mid" | "premium" => Ok(()),
        "droid" => Err("lane tier 'droid' is forbidden (O2: no droid lanes)".to_string()),
        other => Err(format!(
            "unknown lane tier '{other}' (expected one of local|free|mid|premium)"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::lane_allowed;

    #[test]
    fn droid_is_refused_by_name_with_the_law() {
        for tier in ["droid", "DROID", " droid "] {
            let err = lane_allowed(tier).unwrap_err();
            assert!(err.contains("droid"), "must name droid: {err}");
            assert!(err.contains("O2"), "must cite O2: {err}");
        }
    }

    #[test]
    fn closed_vocabulary_case_and_whitespace_insensitive() {
        for tier in ["local", "free", "Mid", "PREMIUM", " mid "] {
            lane_allowed(tier).unwrap();
        }
        for tier in ["banana", "", "droidx"] {
            assert!(lane_allowed(tier).is_err());
        }
    }
}
