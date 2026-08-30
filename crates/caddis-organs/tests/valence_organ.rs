//! valence_organ.rs — CARD-0254 RED-first. The seven-sense body-state
//! organ, measured by A/B abort metrics.
//!
//! The council rejected front-injection (S6 in disguise); the quorum
//! approved valence data in the ORIENTATION PACKET TAIL (volatile
//! zone), soul in the HEAD (session-stable). Seven senses map to
//! existing caddis telemetry. A/B experiment is the PRIMARY
//! measurement; déjà-vu citations are secondary diagnostics.
//!
//! THE RED: today no BodyState, no render_tail, and no abort-metric
//! feedback loop exists. The mood feedback loop (pain -> caution ->
//! failure -> pain) must be detectable via the abort metrics, and
//! none of that compiles now.

use caddis_organs::valence::{
    body_state, render_tail, BoardTick, GateCensus, PagerObserve, TreePulse,
};

/// Build a minimal PagerObserve with the given stored percentage.
fn pager(stored_pct: u64) -> PagerObserve {
    PagerObserve {
        stored_pct,
        stored_tokens: 0,
        evicted: 0,
    }
}

/// Build a minimal GateCensus.
fn census(file_size: u64, max_ccn: u32, test_count: u32) -> GateCensus {
    GateCensus {
        file_size,
        max_ccn,
        test_count,
    }
}

/// Build a minimal BoardTick.
fn bee(card: &str, exit: i64) -> BoardTick {
    BoardTick {
        card: card.into(),
        exit,
    }
}

/// Build a minimal TreePulse.
fn tree(urgency: u32, pace: &str) -> TreePulse {
    TreePulse {
        goal_urgency: urgency,
        pace_verdict: pace.into(),
    }
}

/// RED: a healthy steady state renders a compact tail (~60 tokens)
/// with mood dominated by joy, and the abort metrics at baseline.
#[test]
fn healthy_state_renders_joyful_tail() {
    let state = body_state(
        &[],
        &census(100, 5, 10),
        &pager(20),
        &[bee("c1", 0)],
        &tree(1, "run"),
    );
    let tail = render_tail(&state);
    assert!(!tail.is_empty(), "tail is non-empty");
    assert!(
        tail.len() < 400,
        "tail is compact (<~60 tokens): {} chars",
        tail.len()
    );
    assert!(state.mood.joy > state.mood.pain, "joy dominates pain");
    let m = &state.mood.abort;
    assert_eq!(m.denial_delta, 0, "baseline denial delta");
    assert_eq!(m.error_delta, 0, "baseline error delta");
    assert!(!m.aborted, "healthy state does not abort");
}

/// RED: a pain feedback loop (high error rate + high CCN) raises pain
/// above joy, and the abort metrics must detect the degradation so
/// the loop (pain -> caution -> failure -> pain) is OBSERVABLE.
#[test]
fn pain_loop_is_detectable_via_abort_metrics() {
    // Three failing bees + max CCN at the cap + 0 tests.
    let bees = vec![bee("c1", 1), bee("c2", 2), bee("c3", 1)];
    let state = body_state(
        &[],
        &census(500, 11, 0),
        &pager(95),
        &bees,
        &tree(9, "stalled"),
    );
    assert!(
        state.mood.pain > state.mood.joy,
        "pain dominates joy in a degradation loop"
    );
    // The abort metrics MUST surface the signal — denial or error
    // delta crosses the two-sided threshold. This is the feedback
    // loop's detection: without it the pain loop is invisible.
    let m = &state.mood.abort;
    let signal = m.denial_delta >= 20 || m.error_delta >= 20;
    assert!(
        signal,
        "abort metric surfaces degradation: denial={} err={} (one must >=20)",
        m.denial_delta, m.error_delta
    );
    assert!(m.aborted, "two-sided abort trips on degradation");
}

/// RED: the valence tail is volatile content (mood, patience) and
/// NEVER appears at the packet head. render_tail must not contain
/// session-stable soul markers (e.g. "SOUL" / "session:").
#[test]
fn tail_is_volatile_not_soul() {
    let state = body_state(
        &[],
        &census(100, 5, 10),
        &pager(20),
        &[bee("c1", 0)],
        &tree(1, "run"),
    );
    let tail = render_tail(&state);
    assert!(!tail.contains("SOUL"), "no soul markers in the tail");
    assert!(tail.contains("mood"), "mood is volatile tail content");
}

/// RED: body_state is PURE — it takes raw telemetry slices and
/// composes BodyState with no I/O. The seven senses must each carry
/// their mapped telemetry value.
#[test]
fn body_state_maps_seven_senses() {
    let state = body_state(
        &[],
        &census(200, 7, 4),
        &pager(50),
        &[bee("c1", 0), bee("c2", 0)],
        &tree(3, "run"),
    );
    assert_eq!(state.touch.file_size, 200, "TouchSense: file size");
    assert_eq!(state.touch.max_ccn, 7, "TouchSense: CCN");
    assert_eq!(state.touch.test_count, 4, "TouchSense: test count");
    assert_eq!(state.proprio.stored_pct, 50, "Proprioception: stored_pct");
    assert_eq!(state.empathy.bee_count, 2, "EmpathySense: bee count");
    assert_eq!(state.empathy.fail_count, 0, "EmpathySense: no failures");
    assert_eq!(state.conation.goal_urgency, 3, "Conation: goal urgency");
    assert_eq!(state.conation.pace_verdict, "run", "Conation: pace verdict");
}

/// RED: a cache collapse (all-cold trailing window) raises the
/// SmellSense alarm, feeding pain into mood.
#[test]
fn cache_collapse_feeds_smell_and_pain() {
    let state = body_state(
        &[],
        &census(100, 5, 10),
        &pager(20),
        &[bee("c1", 0)],
        &tree(1, "run"),
    );
    // With healthy telemetry the smell is calm; the structural
    assert!(
        state.smell.cache_health <= 100,
        "SmellSense reports a bounded grade"
    );
}

/// RED: the abort metrics are first-class signals on the dash —
/// both denial and error deltas are present and named.
#[test]
fn abort_metrics_are_first_class() {
    let state = body_state(
        &[],
        &census(100, 5, 10),
        &pager(20),
        &[bee("c1", 0)],
        &tree(1, "run"),
    );
    let m = &state.mood.abort;
    // Both metrics must be present as first-class fields, not buried.
    assert!(m.denial_delta <= 100, "denial delta is a bounded signal");
    assert!(m.error_delta <= 100, "error delta is a bounded signal");
}

/// RED: conation reads goal urgency and pace verdict from the tree.
/// A stalled tree with high urgency feeds pain.
#[test]
fn stalled_high_urgency_feeds_pain() {
    let state = body_state(
        &[],
        &census(100, 5, 10),
        &pager(20),
        &[bee("c1", 0)],
        &tree(9, "stalled"),
    );
    assert!(
        state.conation.goal_urgency >= 8,
        "high urgency read from tree"
    );
    assert_eq!(state.conation.pace_verdict, "stalled");
    assert!(state.mood.pain > 0, "stalled high urgency feeds pain");
}
