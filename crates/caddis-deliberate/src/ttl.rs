//! ttl.rs — P1 slice 2: the F10 seat-lifecycle state machine — TTL
//! transitions and per-state re-probe cadence, PURE over its inputs (no
//! clocks read, no probes sent, no stream written; the P3 executor
//! supplies `now` and performs the actions).
//!
//! Laws transcribed (plan P1; brief Rulings 5+8; F10):
//! - **EXPIRED carries TTL → auto Failed/Retired** and removal from panel
//!   selection. Expired is the quota-calendar COOLDOWN (the preserved
//!   council-toolkit pattern): within its TTL a seat is NOT re-probed
//!   (hammering a dead lane is the wedge lesson); when the TTL lapses
//!   without renewal the seat transitions.
//! - **Renewable free lanes NEVER auto-retire** (Rulings 5+8): a seat on
//!   a quota-renewable lane (measured free = 0/0 billing, [`quota_renewable`])
//!   lands `Failed` at TTL lapse — and Failed retries on its own cadence
//!   forever. Retire is the NON-renewable arm: a paid lane stuck Expired
//!   past its TTL retires (reviving it is a money-attached operator
//!   ruling through the slice-3 edit path — the taste boundary).
//! - **Cadence is DATA** ([`Cadence`], prior defaults): the operator may
//!   re-rule every duration; the machine never hard-codes one.
//! - **`since_epoch_s == 0` means "no clock data"** — exactly the
//!   collector seed (deterministic bytes, no clocks in cards). For a
//!   `Probing` seed it means the probe NEVER STARTED: the first probe is
//!   DUE NOW ([`Step::ReprobeDue`]), not a timeout failure. For any other
//!   state 0 cannot occur in a real stream (every state-change row stamps
//!   `now`); if it ever does, elapsed-from-epoch applies — every timeout
//!   lapses at once, fail-closed.
//! - **One step per sweep**: [`sweep`] applies a single transition per
//!   seat, never a cascade — the next sweep re-derives from the new row
//!   (deterministic replay, honest intermediate states in the stream).

use crate::registry::{Card, Registry, SeatCard};
use crate::CostClass;
use crate::SeatState;

/// Per-state cadence table, SECONDS — DATA with prior defaults. A ruling
/// (slice-3 edit path) may re-rule any value; `0` is a legal choice
/// meaning "act immediately".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cadence {
    /// Live: a probe older than this makes the seat stale — the sweep
    /// EXPIRES it (stale truth beats a stale live flag, F10).
    pub live_probe_every_s: u64,
    /// Expired: the quota-calendar cooldown TTL. Within it: no re-probe.
    /// At lapse: [`Step::Fail`] (renewable) / [`Step::Retire`] (not).
    pub expired_ttl_s: u64,
    /// RateLimited: cooldown until the lane is worth probing again.
    pub rate_limited_cooldown_s: u64,
    /// Probing: a probe unanswered this long has wedged — Fail it.
    pub probing_timeout_s: u64,
    /// Failed: re-probe cadence. Never gives up (Rulings 5+8) — a
    /// renewable lane retries forever; there is no Failed→Retired arm.
    pub failed_retry_every_s: u64,
    /// Unprobeable: re-probe cadence (Q6 amendment) — the seat stays in
    /// the rotation so auth landing via the edits path lifts it on the
    /// NEXT rotation automatically. Never Failed, never retired from
    /// here; the alert already fired at the transition.
    pub unprobeable_retry_every_s: u64,
}

impl Default for Cadence {
    /// Priors (DATA, re-ruleable): hourly liveness re-probes, a daily
    /// quota calendar, a 15-minute rate-limit cooldown, a 5-minute probe
    /// timeout, hourly retries. Mirrors the council toolkit's rhythms.
    /// `unprobeable_retry_every_s` = hourly (Q6: lift checks ride the
    /// rotation itself, so it must stay cheap — a $0 listing per check).
    fn default() -> Self {
        Cadence {
            live_probe_every_s: 3600,
            expired_ttl_s: 86_400,
            rate_limited_cooldown_s: 900,
            probing_timeout_s: 300,
            failed_retry_every_s: 3600,
            unprobeable_retry_every_s: 3600,
        }
    }
}

/// What the machine says should happen to a seat at `now`. Pure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// No transition; nothing due.
    Keep,
    /// Live → Expired: the probe went stale (the Expired TTL window
    /// starts at this sweep's `now`).
    Expire,
    /// → Failed: Expired TTL lapsed on a renewable lane, or a probe
    /// wedged past its timeout.
    Fail,
    /// → Retired: Expired TTL lapsed on a NON-renewable lane. Terminal
    /// until an operator ruling revives the seat.
    Retire,
    /// A (re-)probe is DUE — an ACTION for the P3 dispatcher, not a
    /// state change: no card is written until the probe's RESULT lands.
    ReprobeDue,
}

/// One seat's step at `now`. Pure; `renewable` selects the Expired-TTL
/// arm (transcribe: [`quota_renewable`]; a ruling may override).
pub fn step(card: &SeatCard, now_epoch_s: u64, cadence: &Cadence, renewable: bool) -> Step {
    // Seed law: a Probing seat with no clock data has never been probed —
    // the first probe is due NOW, not a timeout failure.
    if card.state == SeatState::Probing && card.since_epoch_s == 0 {
        return Step::ReprobeDue;
    }
    let elapsed = now_epoch_s.saturating_sub(card.since_epoch_s);
    match card.state {
        SeatState::Live => {
            if elapsed >= cadence.live_probe_every_s {
                Step::Expire
            } else {
                Step::Keep
            }
        }
        SeatState::Expired => {
            if elapsed >= cadence.expired_ttl_s {
                if renewable {
                    Step::Fail
                } else {
                    Step::Retire
                }
            } else {
                Step::Keep
            }
        }
        SeatState::RateLimited => {
            if elapsed >= cadence.rate_limited_cooldown_s {
                Step::ReprobeDue
            } else {
                Step::Keep
            }
        }
        SeatState::Probing => {
            if elapsed >= cadence.probing_timeout_s {
                Step::Fail
            } else {
                Step::Keep
            }
        }
        SeatState::Failed => {
            if elapsed >= cadence.failed_retry_every_s {
                Step::ReprobeDue
            } else {
                Step::Keep
            }
        }
        SeatState::Unprobeable => {
            if elapsed >= cadence.unprobeable_retry_every_s {
                Step::ReprobeDue
            } else {
                Step::Keep
            }
        }
        SeatState::Retired => Step::Keep,
    }
}

/// Rulings 5+8 transcription: a seat on a quota-renewable lane. Measured
/// FREE billing (0/0 — a fact, not taste) means the lane's capacity is a
/// renewable quota; Mid/Premium lanes are bought capacity — an Expired
/// one past its TTL retires to the ruling path.
pub fn quota_renewable(card: &SeatCard) -> bool {
    card.cost_class == CostClass::Free
}

/// Sweep the whole registry at `now`: one step per seat, seat-id order
/// (BTreeMap — deterministic). Returns the TRANSITION CARDS to append to
/// the stream (state flipped, `since_epoch_s` = `now`, everything else
/// identical). `Keep` and `ReprobeDue` produce no card — the caller
/// appends what it gets, refolds, and the new rows are the truth (F2).
/// The renewable predicate is a parameter so a ruling can override
/// [`quota_renewable`] without touching the machine.
pub fn sweep(
    reg: &Registry,
    now_epoch_s: u64,
    cadence: &Cadence,
    renewable: fn(&SeatCard) -> bool,
) -> Vec<Card> {
    let mut out = Vec::new();
    for seat in reg.seats.values() {
        let next = match step(seat, now_epoch_s, cadence, renewable(seat)) {
            Step::Keep | Step::ReprobeDue => continue,
            Step::Expire => SeatState::Expired,
            Step::Fail => SeatState::Failed,
            Step::Retire => SeatState::Retired,
        };
        let mut card = seat.clone();
        card.state = next;
        card.since_epoch_s = now_epoch_s;
        out.push(Card::Seat(card));
    }
    out
}

#[cfg(test)]
#[path = "ttl_tests.rs"]
mod tests;
