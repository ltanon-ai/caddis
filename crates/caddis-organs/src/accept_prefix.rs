//! accept_prefix.rs — CARD-0248. The MTP longest-verified-prefix organ.
//!
//! MTP's deepest law (docs/design/MTP-DEEP-STUDY-2026-08-28.md): on
//! verification failure, keep every step up to the first divergence
//! and rewind ONLY the tail. Today the estate discards everything —
//! a card whose Done-When fails at check 7 of 10 re-runs the whole
//! card. [`longest_prefix`] is the pure organ that converts every
//! failed verification from a full restart into a tail-rewind: given
//! ordered checkpoint results, the accepted prefix is everything up
//! to (not including) the first Fail.
//!
//! The organ NEVER auto-accepts beyond a Pass: relaxed acceptance
//! (MTP's top-k tolerance) is REJECTED for work — work verifies
//! exactly or rewinds (our 0% false-pass law outranks throughput).
//! The prefix is CONTIGUOUS from the start; a Pass after a Fail is
//! never accepted.
//!
//! Pure, zero-dep, sync, std only. The organ computes the prefix;
//! the HOST re-arms from `prefix.step + 1` (not from 1) — that is the
//! tail-rewind, and the dispatch counter proves it.

/// One checkpoint result from a card's ordered Done-When checks.
/// The host records these per edit-hunk at each Done-When bullet
/// boundary; the organ reads them to find the accepted prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckResult {
    /// The check passed at `step` with checkpoint `hash`.
    /// `hash` is the host's build-stable checkpoint hash (the resume
    /// point for the tail-rewind).
    Pass { step: usize, hash: u64 },
    /// The check failed at `step` with `why` — the divergence point.
    Fail { step: usize, why: String },
}

/// The accepted prefix: how many steps passed before the first Fail,
/// and the checkpoint hash of the last accepted Pass (the resume
/// point). Empty prefix (first-step fail, or no results) is
/// `step == 0, checkpoint_hash == 0`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedPrefix {
    /// Number of contiguous Pass results from the start. 0 if the
    /// first result is Fail or the input is empty.
    pub step: usize,
    /// Checkpoint hash of the last accepted Pass (step's hash).
    /// 0 when the prefix is empty.
    pub checkpoint_hash: u64,
}

/// Given ordered checkpoint results, return the longest prefix of
/// contiguous Pass results from the start. The prefix is everything
/// up to (not including) the first Fail. Empty on first-step fail or
/// empty input.
///
/// Pure: no I/O, no allocation beyond the returned struct. The host
/// re-arms from `prefix.step + 1` — NOT from 1 — to achieve the
/// tail-rewind.
pub fn longest_prefix(results: &[CheckResult]) -> VerifiedPrefix {
    let mut step: usize = 0;
    let mut hash: u64 = 0;

    for r in results {
        match r {
            CheckResult::Pass { step: s, hash: h } => {
                step = *s;
                hash = *h;
            }
            CheckResult::Fail { .. } => break,
        }
    }

    VerifiedPrefix {
        step,
        checkpoint_hash: hash,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_is_empty_prefix() {
        let p = longest_prefix(&[]);
        assert_eq!(p.step, 0);
        assert_eq!(p.checkpoint_hash, 0);
    }

    #[test]
    fn first_fail_is_empty_prefix() {
        let p = longest_prefix(&[CheckResult::Fail {
            step: 1,
            why: "x".into(),
        }]);
        assert_eq!(p.step, 0);
        assert_eq!(p.checkpoint_hash, 0);
    }

    #[test]
    fn contiguous_pass_then_fail_stops_at_fail() {
        let p = longest_prefix(&[
            CheckResult::Pass { step: 1, hash: 10 },
            CheckResult::Pass { step: 2, hash: 20 },
            CheckResult::Fail {
                step: 3,
                why: "div".into(),
            },
            CheckResult::Pass { step: 4, hash: 40 },
        ]);
        assert_eq!(p.step, 2);
        assert_eq!(p.checkpoint_hash, 20);
    }
}
