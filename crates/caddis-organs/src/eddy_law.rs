//! eddy_law.rs — the HALT LAW of the loop organ (CARD-0229 → 0231),
//! split out of eddy.rs under the 280-line law. The verdict stays PURE
//! (no I/O) so the nerve and the loop-runner call the ONE definition
//! (CARD-0234 pins that; a second threshold anywhere is forbidden).
//!
//! Law order inside `verdict`: fail-streak FIRST (the measured burn),
//! then stagnation. Fatal classes (CARD-0232) will slot in front.
//! BOUNDS are declared at arm time and judged by `Armed::judge`
//! (eddy_arm.rs) — they are a contract, not an observation.

use std::path::Path;

use crate::blocker::{file_blocker, Blocker};
use crate::eddy::{StatusClass, Tick};
use crate::util::iso8601_now;

/// The ONE fail-streak threshold of the estate: the watchdog's law
/// (CARD-0234 deleted eddy's duplicate MAX_CONSECUTIVE_FAILURES — one
/// law, two hosts, never a second counter).
use crate::watchdog::DEFAULT_MAX_FAILURES as MAX_CONSECUTIVE_FAILURES;
/// How many trailing ticks with an IDENTICAL outcome_hash before the
/// run reports Stagnant. Three, like the streak: one repeat is noise
/// at temperature > 0, two is a hint, three is a phase.
pub const STAGNANT_WINDOW: u32 = 3;

/// Why a governed loop halted. There is deliberately NO success-shaped
/// variant: success needs an external completion witness, and the organ
/// may never emit it from a hash (CARD-0231, quorum §4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HaltReason {
    FailStreak {
        streak: u32,
    },
    /// Fatal-until-reset class observed ONCE (CARD-0232). A quota is
    /// not "retry three times"; resume_after carries the provider's
    /// reset time when supplied.
    Fatal {
        class: crate::eddy::FatalClass,
        resume_after: Option<u64>,
    },
    /// The declared bound (iterations or duration) was reached. The
    /// bound is the arm-time contract, judged in eddy_arm::judge.
    BoundReached,
    /// The WAITING lease expired (CARD-0240): stagnation idled past
    /// its declared budget inside the bound. A halt naming the
    /// witness that never came — never success.
    WaitingLeaseExpired,
    /// until-change + stagnation: repetition MEANS DONE for this class,
    /// so the loop stops — a CONVERGED CANDIDATE, not a success. The
    /// external witness decides whether it was really done.
    Converged {
        window: u32,
    },
}

/// The verdict over a run's ticks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The loop may re-fire.
    Continue,
    /// The loop must stop now.
    Halt(HaltReason),
    /// Identical outcomes, no external witness: nothing is progressing,
    /// but under until-external that means WAITING, not done. The host
    /// surfaces it; only bound/fail-streak/fatal may halt.
    Stagnant,
    /// Consecutive `unprovable` dispatches reached the threshold
    /// (CARD-0237): the run cannot prove its own done. A stop verdict
    /// for every host of the beekeeper shape — the THIRD host law,
    /// same threshold constant, never a third counter.
    UnprovableDone { streak: u32 },
}

/// PURE, no I/O — the one definition the runner and the nerve both
/// call (CARD-0229; CARD-0234 pins the single-threshold law).
///
/// Law: `MAX_CONSECUTIVE_FAILURES` consecutive Fail ticks halt the
/// loop. Measured basis (2026-08-28): failure count is the law that
/// matches the burn — phantom replies differ in text, so no hash
/// converges, and 429/403 bodies vary by request-id. KNOWN GAP: a
/// non-Fail tick RESETS the streak, so a copy-loop phase (all Ok,
/// nothing progressing) is caught here only as Stagnant — the class
/// interpretation is the arm-time contract (eddy_arm::judge). Law
/// order: FATAL first (one observation, CARD-0232), then the fail
/// streak, then the unprovable streak (CARD-0237), then stagnation.
pub fn verdict(ticks: &[Tick]) -> Verdict {
    if let Some(reason) = fatal_observed(ticks) {
        return Verdict::Halt(reason);
    }
    if let Some(streak) = trailing_fail_streak(ticks) {
        return Verdict::Halt(HaltReason::FailStreak { streak });
    }
    if let Some(streak) = trailing_unprovable_streak(ticks) {
        return Verdict::UnprovableDone { streak };
    }
    if stagnant_window(ticks).is_some() {
        return Verdict::Stagnant;
    }
    Verdict::Continue
}

/// The FIRST fatal tick in the history, as a halt. ONE observation is
/// the law (CARD-0232): a 403 quota is fatal-until-reset, and the K3
/// seat proved error text can be byte-identical — so the class, never
/// the text, is the discriminator.
fn fatal_observed(ticks: &[Tick]) -> Option<HaltReason> {
    let t = ticks
        .iter()
        .find(|t| matches!(t.status_class, StatusClass::Fatal(_)))?;
    let StatusClass::Fatal(class) = t.status_class else {
        return None;
    };
    Some(HaltReason::Fatal {
        class,
        resume_after: t.resume_after,
    })
}

/// Longest TRAILING run of Fail ticks, halting the moment it reaches
/// the threshold (the loop must not run one extra turn on principle).
/// An Unprovable tick BREAKS the fail streak: a withheld dispatch is
/// not a provider failure (CARD-0237).
fn trailing_fail_streak(ticks: &[Tick]) -> Option<u32> {
    let mut streak: u32 = 0;
    for tick in ticks {
        match tick.status_class {
            StatusClass::Fail => streak += 1,
            // A fatal tick is not streak material — fatal_observed
            // already claimed it, and the streak must not double-count.
            StatusClass::Fatal(_) | StatusClass::Ok | StatusClass::Unprovable => streak = 0,
        }
        if streak >= MAX_CONSECUTIVE_FAILURES {
            return Some(streak);
        }
    }
    None
}

/// Trailing run of UNPROVABLE dispatches (CARD-0237): its own streak —
/// a provider Fail does not feed it, and it does not feed the fail
/// streak. Same threshold constant; never a third counter.
fn trailing_unprovable_streak(ticks: &[Tick]) -> Option<u32> {
    let mut streak: u32 = 0;
    for tick in ticks {
        match tick.status_class {
            StatusClass::Unprovable => streak += 1,
            _ => streak = 0,
        }
        if streak >= MAX_CONSECUTIVE_FAILURES {
            return Some(streak);
        }
    }
    None
}

/// Wall-clock span (ms) of the TRAILING run of identical outcomes,
/// None when unmeasured (legacy ts_ms 0) or shorter than the window.
/// CARD-0240's lease clock: measured from the FIRST tick of the
/// current stagnant run.
pub fn stagnant_run_span(ticks: &[Tick]) -> Option<u64> {
    let last = ticks.last()?;
    let mut first_i = ticks.len() - 1;
    while first_i > 0
        && ticks[first_i - 1].outcome_hash == last.outcome_hash
        && ticks[first_i - 1].page == last.page
    {
        first_i -= 1;
    }
    let first = ticks.get(first_i)?;
    if first.ts_ms == 0 || last.ts_ms == 0 {
        return None;
    }
    Some(last.ts_ms.saturating_sub(first.ts_ms))
}

/// Trailing run of ticks with IDENTICAL outcome_hash >= STAGNANT_WINDOW.
fn stagnant_window(ticks: &[Tick]) -> Option<u32> {
    let last = ticks.last()?;
    let mut window: u32 = 1;
    for tick in ticks[..ticks.len() - 1].iter().rev() {
        // CARD-0242: hashes never compare across a page rollover —
        // the pre-boundary ticks belong to a different context.
        if tick.outcome_hash != last.outcome_hash || tick.page != last.page {
            break;
        }
        window += 1;
    }
    (window >= STAGNANT_WINDOW).then_some(window)
}

/// Apply the law to a run's ticks: verdict, and on Halt file ONE
/// blocker the operator must resolve (blocker.rs pattern — the record
/// outlives the process that filed it).
pub fn enforce(run_id: &str, ticks: &[Tick], blocker_path: &Path) -> std::io::Result<Verdict> {
    let v = verdict(ticks);
    if let Verdict::Halt(reason) = &v {
        file_blocker(
            blocker_path,
            &Blocker {
                source: format!("eddy:{run_id}"),
                reason: halt_reason_text(reason),
                ts: iso8601_now(),
            },
        )?;
    }
    Ok(v)
}

pub fn halt_reason_text(reason: &HaltReason) -> String {
    match reason {
        HaltReason::FailStreak { streak } => {
            format!("fail streak {streak}: halting governed loop")
        }
        HaltReason::Fatal {
            class,
            resume_after,
        } => match resume_after {
            Some(t) => format!(
                "fatal class {} at first observation: halting governed loop (provider resume-after {t})",
                class.as_str()
            ),
            None => format!(
                "fatal class {} at first observation: halting governed loop (no resume-after supplied)",
                class.as_str()
            ),
        },
        HaltReason::BoundReached => "declared bound reached: halting governed loop".into(),
        HaltReason::WaitingLeaseExpired => {
            "waiting lease expired: the external witness never came — halting governed loop".into()
        }
        HaltReason::Converged { window } => format!(
            "until-change converged after {window} identical outcomes (candidate, not success): halting"
        ),
    }
}
