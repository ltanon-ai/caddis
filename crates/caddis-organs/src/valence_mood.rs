//! valence_mood.rs — CARD-0254. The mood aggregate, abort metrics, and
//! the pain/joy/patience/abort scorers, split out of valence.rs under
//! the 280-line law.
//!
//! Mood is a decaying aggregate of pain + joy with an attention pawl.
//! The pawl is the attention layer's ratchet: pain locks in until the
//! loop produces a clean turn, so the feedback loop (pain -> caution ->
//! failure -> pain) is VISIBLE across ticks, not smoothed away.
//!
//! The abort metrics are the A/B experiment's first-class signals
//! (K3's blocking conditions): denial and error deltas are +over-
//! baseline percentages; the two-sided abort trips when EITHER
//! crosses the +20% threshold. Both are first-class dash signals.

use crate::valence_senses::{
    Conation, EmpathySense, Interoception, Proprioception, SmellSense, TouchSense,
};

/// Mood: decaying aggregate of pain + joy with an attention pawl.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Mood {
    pub pain: u64,
    pub joy: u64,
    pub patience: u64,
    /// The attention pawl: pain that survives decay this tick.
    pub pawl: u64,
    pub abort: AbortMetrics,
}

/// The A/B experiment's first-class abort signals (K3's blocking
/// conditions). Both deltas are +over-baseline percentages; the
/// two-sided abort trips when EITHER crosses the +20% threshold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AbortMetrics {
    pub denial_delta: u64,
    pub error_delta: u64,
    pub aborted: bool,
}

/// Compose mood from the senses. Pain feeds from error rate, failing
/// bees, CCN at the cap, a stalled tree, and cache collapse. Joy feeds
/// from passing bees, tests, and a warm cache. The pawl RATCHETS pain:
/// it never decays below the worst single signal this turn. The abort
/// metrics are the A/B experiment's first-class signals.
pub fn mood(
    intero: &Interoception,
    touch: &TouchSense,
    empathy: &EmpathySense,
    conation: &Conation,
    smell: &SmellSense,
    proprio: &Proprioception,
) -> Mood {
    let pain = pain_score(intero, touch, empathy, conation, smell, proprio);
    let joy = joy_score(touch, empathy, smell, proprio);
    let patience = patience_score(intero, conation, proprio);
    let pawl = pain.min(100);
    let abort = abort_metrics(intero, empathy, touch, smell);
    Mood {
        pain,
        joy,
        patience,
        pawl,
        abort,
    }
}

/// Pain: sum of degradation signals, capped at 100.
fn pain_score(
    intero: &Interoception,
    touch: &TouchSense,
    empathy: &EmpathySense,
    conation: &Conation,
    smell: &SmellSense,
    proprio: &Proprioception,
) -> u64 {
    let err = intero.error_rate;
    let fail_bees = (empathy.fail_count as u64) * 20;
    let ccn = if touch.max_ccn >= 11 { 30 } else { 0 };
    let stalled = if conation.pace_verdict == "stalled" {
        (conation.goal_urgency as u64) * 5
    } else {
        0
    };
    let cache = if smell.cache_health == 0 && empathy.bee_count > 0 {
        20
    } else {
        let deficit = 100u64.saturating_sub(smell.cache_health as u64);
        deficit / 5
    };
    let overflow = if proprio.stored_pct >= 90 { 20 } else { 0 };
    (err + fail_bees + ccn + stalled + cache + overflow).min(100)
}

/// Joy: sum of health signals, capped at 100.
fn joy_score(
    touch: &TouchSense,
    empathy: &EmpathySense,
    smell: &SmellSense,
    proprio: &Proprioception,
) -> u64 {
    let tests = (touch.test_count as u64).min(50);
    let pass_bees = (empathy.bee_count.saturating_sub(empathy.fail_count) as u64) * 20;
    let cache = smell.cache_health as u64;
    let headroom = 100u64.saturating_sub(proprio.stored_pct).min(40);
    (tests + pass_bees + cache / 4 + headroom / 2).min(100)
}

/// Patience: how much runway remains before the loop should yield.
fn patience_score(intero: &Interoception, conation: &Conation, proprio: &Proprioception) -> u64 {
    let err_drain = intero.error_rate / 2;
    let urgency_drain = (conation.goal_urgency as u64) * 3;
    let fill_drain = proprio.stored_pct / 4;
    100u64.saturating_sub(err_drain + urgency_drain + fill_drain)
}

/// The A/B abort metrics. Denial delta rises with CCN at the cap (gate
/// denials); error delta rises with the error rate, failing bees, and
/// cache collapse. Two-sided abort trips at +20% on EITHER (K3).
pub fn abort_metrics(
    intero: &Interoception,
    empathy: &EmpathySense,
    touch: &TouchSense,
    smell: &SmellSense,
) -> AbortMetrics {
    let denial_delta = if touch.max_ccn >= 11 {
        40
    } else if touch.max_ccn >= 9 {
        10
    } else {
        0
    };
    let err_rate = intero.error_rate;
    let fail_bees = (empathy.fail_count as u64) * 20;
    let cache_penalty = 100u64.saturating_sub(smell.cache_health as u64) / 5;
    let error_delta = (err_rate + fail_bees + cache_penalty).min(100);
    let aborted = denial_delta >= 20 || error_delta >= 20;
    AbortMetrics {
        denial_delta,
        error_delta,
        aborted,
    }
}
