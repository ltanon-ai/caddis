//! valence.rs — CARD-0254. The seven-sense body-state organ, measured
//! by A/B abort metrics.
//!
//! The council rejected front-injection (S6 in disguise); the quorum
//! approved the revised architecture: valence data lives in the
//! ORIENTATION PACKET TAIL (volatile zone), soul in the HEAD
//! (session-stable). Seven senses map to existing caddis telemetry.
//! A/B experiment is the PRIMARY measurement; déjà-vu citations are
//! secondary diagnostics.
//!
//! The organ COMPUTES; the host RENDERS. [`body_state`] is PURE (no
//! I/O): it takes raw telemetry snapshots and composes the seven
//! senses into [`BodyState`]. [`render_tail`] formats ~60 tokens of
//! volatile content for the packet tail — never the head.
//!
//! The mood feedback loop (pain -> caution -> failure -> pain) is
//! detectable via the [`AbortMetrics`] — denial and error deltas are
//! first-class dash signals, and the two-sided abort trips when
//! either crosses +20% over baseline (K3's blocking conditions).
//!
//! Splits: [`valence_senses`] (the seven sense structs + composers),
//! [`valence_mood`] (mood + abort metrics). This file is the public
//! API: [`body_state`] + [`render_tail`] + re-exports.

use crate::eddy::Tick;
use crate::valence_mood::mood;
pub use crate::valence_mood::{AbortMetrics, Mood};
use crate::valence_senses::{empathy_sense, interoception, smell_sense, time_sense};
pub use crate::valence_senses::{
    BoardTick, Conation, EmpathySense, GateCensus, Interoception, PagerObserve, Proprioception,
    SmellSense, TimeSense, TouchSense, TreePulse,
};

/// The seven-sense body state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodyState {
    pub proprio: Proprioception,
    pub time_: TimeSense,
    pub intero: Interoception,
    pub touch: TouchSense,
    pub smell: SmellSense,
    pub empathy: EmpathySense,
    pub conation: Conation,
    pub mood: Mood,
}

/// PURE, no I/O. Takes the raw telemetry snapshots and composes the
/// seven-sense [`BodyState`]. The host fills the snapshots from the
/// existing nerves; this function never reads a file.
pub fn body_state(
    ticks: &[Tick],
    gates: &GateCensus,
    pager: &PagerObserve,
    board: &[BoardTick],
    tree: &TreePulse,
) -> BodyState {
    let proprio = Proprioception {
        stored_pct: pager.stored_pct,
        stored_tokens: pager.stored_tokens,
    };
    let time_ = time_sense(ticks);
    let intero = interoception(ticks, gates);
    let touch = TouchSense {
        file_size: gates.file_size,
        max_ccn: gates.max_ccn,
        test_count: gates.test_count,
    };
    let smell = smell_sense(ticks, pager);
    let empathy = empathy_sense(board);
    let conation = Conation {
        goal_urgency: tree.goal_urgency,
        pace_verdict: tree.pace_verdict.clone(),
    };
    let composed_mood = mood(&intero, &touch, &empathy, &conation, &smell, &proprio);
    BodyState {
        proprio,
        time_,
        intero,
        touch,
        smell,
        empathy,
        conation,
        mood: composed_mood,
    }
}

/// Render ~60 tokens of volatile content for the orientation packet
/// TAIL. Mood, patience, and the abort metrics are volatile — they
/// change every tick and NEVER appear at the packet head (the soul is
/// session-stable and lives in the HEAD). This is a REPLACEMENT, not
/// an addition: the tail lives INSIDE the fixed byte budget.
pub fn render_tail(state: &BodyState) -> String {
    let bar = |v: u64| -> String {
        let filled = (v / 10).min(10) as usize;
        format!("[{}{}]", "#".repeat(filled), "-".repeat(10 - filled))
    };
    format!(
        "tail | mood {}{} p{} pat{} | smell:{} bees:{}{} | deny:{} err:{} {}",
        bar(state.mood.pain),
        bar(state.mood.joy),
        state.mood.pawl,
        bar(state.mood.patience),
        state.smell.cache_health,
        state.empathy.bee_count,
        state.empathy.fail_count,
        state.mood.abort.denial_delta,
        state.mood.abort.error_delta,
        if state.mood.abort.aborted {
            "ABORT"
        } else {
            "ok"
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eddy::{StatusClass, Tick};

    fn ok_tick(seq: u64) -> Tick {
        Tick {
            run_id: "r".into(),
            seq,
            payload_hash: 0,
            status_class: StatusClass::Ok,
            outcome_hash: 0,
            artifact_hash: 0,
            cache_read: 100,
            cache_write: 10,
            latency_ms: 50,
            ts_ms: 10_000 + seq * 1_000,
            resume_after: None,
            page: 0,
        }
    }

    #[test]
    fn empty_inputs_do_not_panic() {
        let state = body_state(
            &[],
            &GateCensus::default(),
            &PagerObserve::default(),
            &[],
            &TreePulse::default(),
        );
        assert_eq!(state.proprio.stored_pct, 0);
        assert_eq!(state.empathy.bee_count, 0);
    }

    #[test]
    fn render_tail_is_compact() {
        let state = body_state(
            &[ok_tick(1)],
            &GateCensus {
                file_size: 100,
                max_ccn: 5,
                test_count: 3,
            },
            &PagerObserve {
                stored_pct: 20,
                stored_tokens: 0,
                evicted: 0,
            },
            &[BoardTick {
                card: "c".into(),
                exit: 0,
            }],
            &TreePulse {
                goal_urgency: 1,
                pace_verdict: "run".into(),
            },
        );
        let t = render_tail(&state);
        assert!(t.len() < 400, "compact: {} chars", t.len());
        assert!(!t.contains("SOUL"), "no soul in the tail");
    }
}
