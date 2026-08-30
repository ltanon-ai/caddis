//! eddy_runner.rs — CARD-0234. The loop-runner HOST adapter: the
//! sergeant's loop-runner (~/.omp/sergeant/organs/loop-runner) is a
//! HOST of the verdict, not a second organ.
//!
//! One law, two hosts: the nerve (caddis eddy tick, CARD-0233) and the
//! runner BOTH call the same pure `eddy::verdict` through the arm
//! contract. This module defines NO threshold of its own — never a
//! second N, never a local counter; the threshold constant the law
//! reads is `watchdog::DEFAULT_MAX_FAILURES`, alone.
//!
//! Context (H3 falsifier, 2026-08-28): today's burn was NOT the
//! runner — all 20 watchdog rows were sergeant/watch, zero respawns.
//! But three kill-runner+respawn rows exist on the bee2 lane
//! (2026-08-27 13:31/14:16/15:16Z, "runner wedged 300s"): a mechanism
//! that kills a process and starts another is the shape of "I killed
//! it and it kept going". Routine, not urgent — and not droppable.

use crate::eddy::{HaltReason, Tick, Verdict};
use crate::eddy_arm::Armed;

/// THE law — re-exported, never reimplemented. A fn-pointer pin in
/// eddy_host_runner.rs makes a second definition a test failure.
pub use crate::eddy_law::verdict;

/// What the runner host does after judging a tick. `Stop` carries the
/// short law name for the runner's log; the full reason text comes
/// from `eddy::halt_reason_text`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunnerAction {
    /// Verdict::Continue — re-fire the loop.
    Fire,
    /// Verdict::Stagnant — under until-external this means WAITING:
    /// keep the contract, surface the stagnation, do not stop.
    Wait,
    /// Verdict::Halt — stop the lane. A STOPPED runner stays stopped
    /// until the operator re-arms; respawn-on-halt is forbidden.
    Stop(&'static str),
}

/// The runner's judgement: the arm contract's verdict, mapped. This is
/// the ONLY decision function a runner host may call.
pub fn action(arm: &Armed, ticks: &[Tick]) -> RunnerAction {
    match arm.judge(ticks) {
        Verdict::Continue => RunnerAction::Fire,
        Verdict::Stagnant => RunnerAction::Wait,
        Verdict::UnprovableDone { .. } => RunnerAction::Stop("unprovable done"),
        Verdict::Halt(reason) => RunnerAction::Stop(short_law(&reason)),
    }
}

fn short_law(reason: &HaltReason) -> &'static str {
    match reason {
        HaltReason::FailStreak { .. } => "fail streak",
        HaltReason::Fatal { .. } => "fatal",
        HaltReason::BoundReached => "bound",
        HaltReason::Converged { .. } => "converged",
        HaltReason::WaitingLeaseExpired => "lease expired",
    }
}
