//! eddy_payload_freeze.rs — CARD-0230 RED-first.
//!
//! The operator-hurt: every message typed during a live loop silently
//! BECAME the loop body — a correction aimed at the agent was re-fired
//! for 2.5 hours. Adopted unanimously by council and quorum:
//!   - the armed payload is IMMUTABLE without explicit re-arm;
//!   - typed text during a live loop is a ledgered ONE-SHOT steer for
//!     the next tick only, and PayloadDrift is reported;
//!   - Esc must not hot-rearm on the next keystroke — the 800ms timer
//!     firing the OLD payload between an off/on toggle is the trap;
//!   - the re-arm command ships WITH the freeze, never the freeze alone.

use caddis_organs::eddy_arm::{ArmSpec, Armed, Bound, TypedOutcome};

fn live(payload: &str) -> Armed {
    // Payload-law tests: any bounded arm will do; the freeze is the law
    // under test here, not the bound (that is CARD-0231's eddy_bound_class).
    Armed::arm(
        payload,
        ArmSpec {
            bound: Some(Bound::Iterations(1_000)),
            class: None,
            lease_ms: None,
        },
    )
    .expect("bounded arm")
}

const PAYLOAD: &str = "finish the incident report";

#[test]
fn typed_text_never_replaces_the_armed_payload() {
    let mut a = live(PAYLOAD);
    // A correction aimed at the agent, typed mid-loop:
    match a.typed("hey, stop using kimi for this") {
        TypedOutcome::PayloadDrift { .. } => {} // reported, payload KEPT
        TypedOutcome::PlainMessage => panic!("typed text must be reported as drift"),
    }
    assert_eq!(a.payload(), PAYLOAD, "armed payload is immutable");
    // The steer applies to exactly ONE tick...
    assert_eq!(a.fire().as_deref(), Some("hey, stop using kimi for this"));
    // ...then the armed payload returns:
    assert_eq!(a.fire().as_deref(), Some(PAYLOAD));
    assert_eq!(a.fire().as_deref(), Some(PAYLOAD));
}

#[test]
fn explicit_rearm_is_the_only_swap() {
    let mut a = live(PAYLOAD);
    a.typed("correction"); // drift, never a swap
    a.rearm("new body, explicit");
    assert_eq!(a.payload(), "new body, explicit");
    assert_eq!(a.fire().as_deref(), Some("new body, explicit"));
}

#[test]
fn esc_pauses_the_timer_and_never_hot_rearms() {
    let mut a = live(PAYLOAD);
    a.pause(); // Esc
               // The trap: between an off/on toggle the 800ms timer fired the OLD
               // payload. A paused loop must fire NOTHING.
    assert_eq!(a.fire(), None);
    // Typed text while paused is a plain message for the agent, never
    // an implicit re-arm:
    match a.typed("are you done?") {
        TypedOutcome::PlainMessage => {}
        TypedOutcome::PayloadDrift { .. } => panic!("paused loop cannot drift: nothing is armed"),
    }
    assert_eq!(a.fire(), None);
    assert_eq!(
        a.payload(),
        PAYLOAD,
        "still the old payload, NOT hot-rearmed"
    );
    // Only the explicit re-arm revives governed firing:
    a.rearm("revived body");
    assert_eq!(a.fire().as_deref(), Some("revived body"));
}

#[test]
fn pending_one_shot_steer_dies_at_pause() {
    let mut a = live(PAYLOAD);
    a.typed("one-shot steer");
    a.pause();
    // The one-shot steer was for the NEXT TICK of a LIVE loop; a paused
    // loop has no next tick, and the steer must not survive into a
    // later re-arm as a surprise payload.
    a.rearm(PAYLOAD);
    assert_eq!(a.fire().as_deref(), Some(PAYLOAD));
}
