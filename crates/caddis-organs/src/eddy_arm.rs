//! eddy_arm.rs — the ARMED PAYLOAD law (CARD-0230) and the arm-time
//! CONTRACT: bound + loop class (CARD-0231).
//!
//! The operator-hurt of 2026-08-28: the omp submit path ran
//! `if(this.ctx.loopModeEnabled)this.ctx.setLoopPrompt(e)` — every
//! message typed while loop mode was on silently BECAME the infinite
//! payload, with nothing on screen saying so. A correction aimed at the
//! agent was re-fired for 2.5 hours.
//!
//! Payload law (CARD-0230, adopted unanimously by council and quorum):
//! - the armed payload is IMMUTABLE; explicit re-arm is the ONLY swap;
//! - typed text during a live loop is a ONE-SHOT steer for the next
//!   tick only, and `PayloadDrift` is reported;
//! - Esc pauses; a paused loop fires NOTHING — the 800 ms timer firing
//!   the OLD payload between an off/on toggle is the trap this closes;
//! - a pending one-shot steer dies at pause.
//!
//! Contract law (CARD-0231): arm REQUIRES a bound; arm DECLARES a
//! class; `judge` composes the pure `eddy::verdict` with both. The
//! organ may never emit SUCCESS from a hash — `Verdict` has no such
//! variant, and that absence is the law.

use crate::eddy::{HaltReason, Tick, Verdict};

/// The arm-time bound. UNBOUNDED IS REFUSED: `/loop` with no limit
/// left `loopLimit` undefined and re-fired forever (measured 2.5h).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bound {
    Iterations(u32),
    Millis(u64),
}

/// The loop CLASS — converged-vs-waiting is a CONTRACT declared at arm
/// time, never an inference from a hash (CARD-0231, quorum §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopClass {
    /// Repetition means DONE: the loop halts as a converged CANDIDATE.
    UntilChange,
    /// Repetition means WAITING: only bound, fail-streak and fatal
    /// class may halt. Interactive `/loop` defaults here.
    UntilExternal,
}

/// Why an arm was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArmError {
    Unbounded { reason: String },
}

/// What happened to text typed at a (possibly) looping session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypedOutcome {
    /// The loop is live: the text steers exactly the NEXT tick; the
    /// armed payload is kept. This IS the drift report — the host must
    /// surface it, because on 2026-08-28 nothing did.
    PayloadDrift { steered_for_next_tick: String },
    /// No live loop: the text is a plain message for the agent.
    PlainMessage,
}

/// The arm-time contract as ONE struct (CARD-0240: four positional
/// options was a params struct wearing argument clothing).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArmSpec {
    /// REQUIRED at arm; None is refused (the unbounded 2026-08-28
    /// default burned for 2.5h).
    pub bound: Option<Bound>,
    /// None = UntilExternal, the interactive `/loop` default.
    pub class: Option<LoopClass>,
    /// WAITING lease (CARD-0240): how long a STAGNANT run may idle
    /// inside the bound before the loop halts naming the missing
    /// witness. None = no lease law. NO organ default: a default
    /// would be a threshold the organ invented for the host.
    pub lease_ms: Option<u64>,
}

/// An armed governed loop: payload, one-shot steer slot, contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Armed {
    payload: String,
    steer_next: Option<String>,
    live: bool,
    bound: Bound,
    class: LoopClass,
    lease_ms: Option<u64>,
}

impl Armed {
    /// Arm with a REQUIRED bound; class defaults to UntilExternal;
    /// lease is declared, never defaulted (CARD-0240).
    pub fn arm(payload: &str, spec: ArmSpec) -> Result<Armed, ArmError> {
        let Some(bound) = spec.bound else {
            return Err(ArmError::Unbounded {
                reason: "unbounded arm refused: declare a bound (--until N iterations or \
                         --for-ms T) — the unbounded default re-fired for 2.5h on 2026-08-28"
                    .into(),
            });
        };
        Ok(Armed {
            payload: payload.to_string(),
            steer_next: None,
            live: true,
            bound,
            class: spec.class.unwrap_or(LoopClass::UntilExternal),
            lease_ms: spec.lease_ms,
        })
    }

    /// The ONLY swap of the armed payload (also revives a paused loop:
    /// re-arming is explicit by definition). Keeps the declared bound
    /// and class — those are the run's contract, not the payload's.
    pub fn rearm(&mut self, payload: &str) {
        self.payload = payload.to_string();
        self.steer_next = None;
        self.live = true;
    }

    /// Text typed at the session. Live loop -> one-shot steer with a
    /// drift report; otherwise a plain message. NEVER a payload swap.
    pub fn typed(&mut self, text: &str) -> TypedOutcome {
        if !self.live {
            return TypedOutcome::PlainMessage;
        }
        self.steer_next = Some(text.to_string());
        TypedOutcome::PayloadDrift {
            steered_for_next_tick: text.to_string(),
        }
    }

    /// What the 800 ms timer sends. None when paused — the between-toggle
    /// trap. A steer fires exactly once, then the armed payload returns.
    pub fn fire(&mut self) -> Option<String> {
        if !self.live {
            return None;
        }
        Some(
            self.steer_next
                .take()
                .unwrap_or_else(|| self.payload.clone()),
        )
    }

    /// Esc: pause. Kills any pending one-shot steer; keeps the payload.
    /// Only `rearm` revives firing.
    pub fn pause(&mut self) {
        self.live = false;
        self.steer_next = None;
    }

    pub fn payload(&self) -> &str {
        &self.payload
    }

    pub fn is_live(&self) -> bool {
        self.live
    }

    pub fn bound(&self) -> Bound {
        self.bound
    }

    pub fn class(&self) -> LoopClass {
        self.class
    }

    pub fn lease_ms(&self) -> Option<u64> {
        self.lease_ms
    }

    /// The composed judgement: the declared bound, then the PURE
    /// `eddy::verdict` interpreted through the declared class — with
    /// CARD-0239's basis law: when the host supplies ARTIFACT hashes,
    /// THEY are the fixpoint evidence and prose never converges alone
    /// (a loop emitting near-identical prose while its artifacts move
    /// is WORKING, not converged).
    pub fn judge(&self, ticks: &[Tick]) -> Verdict {
        if self.bound_exceeded(ticks) {
            return Verdict::Halt(HaltReason::BoundReached);
        }
        let artifacts_present = ticks.iter().any(|t| t.artifact_hash != 0);
        if artifacts_present {
            let stable = trailing_artifact_window(ticks)
                .map(|w| w >= crate::eddy::STAGNANT_WINDOW)
                .unwrap_or(false);
            if stable {
                return self.stagnation_verdict_with_lease(ticks);
            }
            return Verdict::Continue;
        }
        match crate::eddy::verdict(ticks) {
            Verdict::Stagnant => self.stagnation_verdict_with_lease(ticks),
            v => v,
        }
    }

    /// Stagnation verdict, lease-aware: an idle beyond the declared
    /// lease halts naming the witness that never came (CARD-0240).
    fn stagnation_verdict_with_lease(&self, ticks: &[Tick]) -> Verdict {
        if let Some(lease) = self.lease_ms {
            if let Some(span) = crate::eddy_law::stagnant_run_span(ticks) {
                if span >= lease {
                    return Verdict::Halt(HaltReason::WaitingLeaseExpired);
                }
            }
        }
        self.stagnation_verdict()
    }

    /// The class contract over an observed stagnation: until-change
    /// stops as a converged CANDIDATE (never success); until-external
    /// keeps waiting.
    fn stagnation_verdict(&self) -> Verdict {
        match self.class {
            LoopClass::UntilChange => Verdict::Halt(HaltReason::Converged {
                window: crate::eddy::STAGNANT_WINDOW,
            }),
            LoopClass::UntilExternal => Verdict::Stagnant,
        }
    }

    fn bound_exceeded(&self, ticks: &[Tick]) -> bool {
        match self.bound {
            Bound::Iterations(n) => ticks.len() as u64 >= n as u64,
            Bound::Millis(ms) => elapsed_ms(ticks) >= ms,
        }
    }
}

/// Wall-clock span of the run. Legacy ticks without a clock (ts_ms 0)
/// never exceed a duration bound: an unmeasured span halts nothing.
fn elapsed_ms(ticks: &[Tick]) -> u64 {
    let Some(first) = ticks.first() else {
        return 0;
    };
    let Some(last) = ticks.last() else {
        return 0;
    };
    if first.ts_ms == 0 || last.ts_ms == 0 {
        return 0;
    }
    last.ts_ms.saturating_sub(first.ts_ms)
}

/// Trailing run of ticks with an IDENTICAL NONZERO artifact_hash.
/// Zeros never count: "nothing observed" is not convergence evidence.
fn trailing_artifact_window(ticks: &[Tick]) -> Option<u32> {
    let last = ticks.last()?;
    if last.artifact_hash == 0 {
        return None;
    }
    let mut window: u32 = 1;
    for tick in ticks[..ticks.len() - 1].iter().rev() {
        // CARD-0242: artifact equality across a rollover is not
        // convergence evidence either.
        if tick.artifact_hash != last.artifact_hash || tick.page != last.page {
            break;
        }
        window += 1;
    }
    Some(window)
}
