//! P3 escalation state machine (O2 + F4 + QQ1a + R1 + QQ2/R9): what happens
//! AFTER a verified fail — a RED-TEST red on the failed lane's work, i.e. a
//! `fail` outcome row in the ledger. A warden policy-deny is NOT a row and
//! can never reach this machine (QQ1a is a type law upstream in
//! [`crate::ledger`]).
//!
//! PURE like everything in the crate (F1): (chain context, capability fold,
//! lane registry, policy) -> ONE climb decision or ONE honest stop. No I/O,
//! no clock, no LLM. Every stop is FAIL-SAFE: the chain halts, nothing is
//! harmed, and the P4 wiring surfaces each stop as an operator alert.
//!
//! Ladder law (F4: ladder ordering = MEASURED quality per class, not a fixed
//! tier list): candidates are the class's alive, tier-permitted, own-measured
//! (N >= min_samples), floor-clearing, non-decayed lanes whose EWMA is
//! STRICTLY above the failed lane's POST-FAIL EWMA. The climb takes the
//! SMALLEST quality step up — escalation is as cheap as the ladder allows,
//! O3 in quality space; ties break by the route() total order (cost, tier,
//! id), so the machine is deterministic. REDO never resume (F4/nemotron):
//! the failed lane itself is excluded — the escalated run is a NEW decision
//! referencing the failed one (F3/R10), never a resumed session on it.
//!
//! Budget law (R1, overriding QQ1b): the per-chain cost cap is a STATIC
//! PER-CLASS budget ceiling — operator-settable data, never a multiple of a
//! failing lane's baseline (2x a free lane's ~zero baseline makes premium
//! escalation mathematically unreachable). No ceiling ruled for the class =
//! fail closed (budgets are never guessed). A candidate whose hop would push
//! the chain past the ceiling is SKIPPED — the machine walks the ladder until
//! one fits or reports [`EscalationErr::OverCeiling`]; a better lane can be
//! cheaper, so a budget-blocked rung never ends the search.
//!
//! Decay + hysteresis (QQ2/R9/R2): the failed run already decayed the lane
//! in the capability fold — the caller MUST pass a [`CapsReport`] folded from
//! the ledger INCLUDING the just-recorded fail (decay-before-climb). A lane
//! at [`HYSTERESIS_FAILS`] trailing fails is DECAYED: out of the selection
//! pool ([`crate::route`] filters it) and never an escalation rung here.
//! Recovery is AUTO (QQ2): one pass clears the counter and the floor still
//! guards re-entry — the death spiral is structurally unreachable. The R2
//! transient->persistent promotion (a ledger row for the demotion) and the
//! operator alert emission are P4 wiring, deliberately not here.

use crate::lane::{DataClass, Lane, LaneTier};
use crate::policy::RoutePolicy;
use crate::stats::{CapsReport, HYSTERESIS_FAILS};

/// F4 (both council lanes): at most 3 decisions per chain — the initial run
/// plus two escalation climbs.
pub const MAX_HOPS: u32 = 3;

/// The chain context the dispatch adapter (P4) tracks for one task card.
/// All fields are DATA the adapter derives from the ledger, never from
/// memory.
#[derive(Debug, Clone, PartialEq)]
pub struct EscalationCtx {
    pub task_class: String,
    pub data_class: DataClass,
    pub failed_lane_id: String,
    /// Decisions already spent on this chain (the initial run = 1).
    pub hops_so_far: u32,
    /// Sum of the chain's outcome `cost_usd_est` so far.
    pub chain_spent_usd: f64,
}

/// One climb decision — the data the escalated decision row is built from.
#[derive(Debug, Clone, PartialEq)]
pub struct Escalation {
    pub from_lane_id: String,
    pub to_lane_id: String,
    pub to_lane_tier: LaneTier,
    /// The candidate's own measured quality for the class (the rung height).
    pub to_quality: f64,
    pub cost_per_task_usd: f64,
    /// chain_spent + this hop — what the chain will have spent if it runs.
    /// R1: fits the class ceiling BY CONSTRUCTION of the choice.
    pub projected_chain_usd: f64,
}

#[derive(Debug, PartialEq)]
pub enum EscalationErr {
    /// F4: the chain already used [`MAX_HOPS`] decisions. Fail-safe stop;
    /// alert.
    MaxHops,
    /// Class has no floor (routing defect — route() would have refused the
    /// initial hop). Fail-safe stop; alert.
    NoFloorForClass,
    /// R1: no static per-class ceiling ruled — budgets are never guessed.
    /// Fail-safe stop; alert (the operator sets ceilings via the P5
    /// propose->confirm surface).
    NoCeilingForClass,
    /// Caller defect: the failed lane has no capability row for the class,
    /// though it just produced an outcome the fold must know. Honest refuse.
    UnknownFailedLane,
    /// No measured lane sits strictly above the failed one while clearing
    /// floor + hysteresis. Fail-safe stop; alert.
    TopOfLadder,
    /// Ladder candidates exist but every climb would push the chain past the
    /// class ceiling. Fail-safe stop; alert.
    OverCeiling,
}

/// Error precedence is deliberate and total: MaxHops > NoFloorForClass >
/// NoCeilingForClass > UnknownFailedLane > TopOfLadder > OverCeiling —
/// the cheapest independent check first, ladder findings last.
pub fn escalate(
    ctx: &EscalationCtx,
    caps: &CapsReport,
    lanes: &[Lane],
    policy: &RoutePolicy,
) -> Result<Escalation, EscalationErr> {
    if ctx.hops_so_far >= MAX_HOPS {
        return Err(EscalationErr::MaxHops);
    }
    let floor = policy
        .floor(&ctx.task_class)
        .ok_or(EscalationErr::NoFloorForClass)?;
    let ceiling = policy
        .ceiling(&ctx.task_class)
        .ok_or(EscalationErr::NoCeilingForClass)?;
    let failed = caps
        .lane_cap(&ctx.failed_lane_id, &ctx.task_class)
        .ok_or(EscalationErr::UnknownFailedLane)?;

    // F4 measured ladder: alive -> tier-permitted -> own measurements
    // (floor + hysteresis clean) -> STRICTLY above the failed rung -> never
    // the failed lane itself (REDO, not resume).
    let mut candidates: Vec<&Lane> = lanes
        .iter()
        .filter(|l| l.alive)
        .filter(|l| policy.permits(ctx.data_class, l.tier))
        .filter(|l| l.id != ctx.failed_lane_id)
        .filter(|l| {
            l.caps.get(&ctx.task_class).is_some_and(|c| {
                c.samples >= policy.min_samples()
                    && c.quality >= floor
                    && c.consecutive_failures < HYSTERESIS_FAILS
                    && c.quality > failed.ewma
            })
        })
        .collect();
    if candidates.is_empty() {
        return Err(EscalationErr::TopOfLadder);
    }
    // Smallest quality step up first; ties by the route() total order.
    candidates.sort_by(|a, b| {
        ord_key(a, &ctx.task_class)
            .partial_cmp(&ord_key(b, &ctx.task_class))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // R1: walk the ladder past budget-blocked rungs (a better lane can be
    // cheaper); the boundary INCLUDES the ceiling (projected == ceiling
    // fits, mirroring the floor's >= convention).
    for lane in candidates {
        let projected = ctx.chain_spent_usd + lane.cost_per_task_usd;
        if projected <= ceiling {
            let cap = lane.caps.get(&ctx.task_class).expect("filter proved it");
            return Ok(Escalation {
                from_lane_id: ctx.failed_lane_id.clone(),
                to_lane_id: lane.id.clone(),
                to_lane_tier: lane.tier,
                to_quality: cap.quality,
                cost_per_task_usd: lane.cost_per_task_usd,
                projected_chain_usd: projected,
            });
        }
    }
    Err(EscalationErr::OverCeiling)
}

/// Ladder order key: (quality step, cost, tier, id). Quality compares only
/// among lanes that PASSED the candidate filter, so the NaN arm of
/// `unwrap_or` is unreachable for candidates — kept only so malformed data
/// stays deterministic instead of panicking.
fn ord_key<'a>(l: &'a Lane, class: &'a str) -> (f64, f64, LaneTier, &'a str) {
    let q = l.caps.get(class).map_or(f64::NAN, |c| c.quality);
    (q, l.cost_per_task_usd, l.tier, l.id.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lane::Capability;
    use crate::ledger::parse_stream;
    use std::collections::BTreeMap;

    /// Fold outcome rows for one lane into the report the SM must see —
    /// INCLUDING the just-recorded fail (decay-before-climb law).
    fn caps_for(lane: &str, seq_base: u64, results: &[bool]) -> CapsReport {
        let lines: Vec<String> = results
            .iter()
            .enumerate()
            .map(|(i, pass)| {
                format!(
                    "{{\"seq\":{},\"ts\":\"t\",\"kind\":\"outcome\",\"card_id\":\"c\",\
                     \"task_class\":\"chair\",\"lane_id\":\"{lane}\",\"model\":\"m\",\
                     \"cost_tokens\":1,\"cost_usd_est\":0.001,\"latency_ms\":10,\
                     \"verify_outcome\":\"{}\",\"escalated_to\":null}}",
                    seq_base + i as u64,
                    if *pass { "pass" } else { "fail" }
                )
            })
            .collect();
        CapsReport::from_rows(&parse_stream(&lines.join("\n")))
    }

    fn cap(quality: f64, samples: u32) -> Capability {
        Capability {
            quality,
            samples,
            consecutive_failures: 0,
        }
    }

    fn ladder_lane(id: &str, tier: LaneTier, cost: f64, quality: f64, samples: u32) -> Lane {
        let mut caps = BTreeMap::new();
        caps.insert("chair".to_string(), cap(quality, samples));
        Lane {
            id: id.to_string(),
            family: "test".to_string(),
            tier,
            alive: true,
            cost_per_task_usd: cost,
            caps,
        }
    }

    fn ctx(failed: &str, hops: u32, spent: f64) -> EscalationCtx {
        EscalationCtx {
            task_class: "chair".to_string(),
            data_class: DataClass::Public,
            failed_lane_id: failed.to_string(),
            hops_so_far: hops,
            chain_spent_usd: spent,
        }
    }

    fn policy_with_ceiling(usd: f64) -> RoutePolicy {
        let mut p = RoutePolicy::default();
        p.set_ceiling("chair", usd);
        p
    }

    #[test]
    fn climbs_the_smallest_measured_rung() {
        // Failed lane: pass,pass,fail -> post-fail EWMA 0.7 (decay already
        // folded). Rungs at 0.72 and 0.95: the SM takes the SMALLER step.
        let caps = caps_for("cheap", 1, &[true, true, false]);
        let lanes = [
            ladder_lane("cheap", crate::lane::LaneTier::Free, 0.001, 0.7, 3),
            ladder_lane("mid-rung", crate::lane::LaneTier::Mid, 0.02, 0.72, 6),
            ladder_lane("top-rung", crate::lane::LaneTier::Premium, 0.5, 0.95, 9),
        ];
        let e = escalate(
            &ctx("cheap", 1, 0.001),
            &caps,
            &lanes,
            &policy_with_ceiling(10.0),
        )
        .expect("climbs");
        assert_eq!(e.from_lane_id, "cheap");
        assert_eq!(e.to_lane_id, "mid-rung");
        assert!((e.to_quality - 0.72).abs() < 1e-12);
        assert!((e.projected_chain_usd - 0.021).abs() < 1e-12);
    }

    #[test]
    fn redo_excludes_failed_lane_and_demands_strictly_above() {
        let caps = caps_for("cheap", 1, &[true, true, false]); // EWMA 0.7
                                                               // equal-rung (0.7, cheapest) and the failed lane itself (0.99 here
                                                               // as data — the lane's OWN caps are caller-supplied and must not
                                                               // rescue it): both are not rungs.
        let lanes = [
            ladder_lane("cheap", crate::lane::LaneTier::Free, 0.001, 0.7, 3),
            ladder_lane("equal", crate::lane::LaneTier::Free, 0.0001, 0.7, 9),
        ];
        assert_eq!(
            escalate(
                &ctx("cheap", 1, 0.0),
                &caps,
                &lanes,
                &policy_with_ceiling(10.0)
            ),
            Err(EscalationErr::TopOfLadder)
        );
    }

    #[test]
    fn max_hops_stops_before_any_lookup() {
        let caps = CapsReport::default();
        // No floor/ceiling lookup even happens: MaxHops wins the precedence.
        assert_eq!(
            escalate(
                &ctx("any", MAX_HOPS, 0.0),
                &caps,
                &[],
                &RoutePolicy::default()
            ),
            Err(EscalationErr::MaxHops)
        );
    }

    #[test]
    fn missing_floor_and_ceiling_each_fail_closed() {
        let caps = caps_for("l", 1, &[false]);
        let mut no_floor = ctx("l", 1, 0.0);
        no_floor.task_class = "unrated".to_string();
        assert_eq!(
            escalate(&no_floor, &caps, &[], &policy_with_ceiling(10.0)),
            Err(EscalationErr::NoFloorForClass)
        );

        // Floor exists, ceiling does not (the DEFAULT policy state — no
        // guessed budgets): fail closed even with a healthy ladder present.
        let lanes = [ladder_lane(
            "good",
            crate::lane::LaneTier::Free,
            0.001,
            0.95,
            9,
        )];
        assert_eq!(
            escalate(&ctx("l", 1, 0.0), &caps, &lanes, &RoutePolicy::default()),
            Err(EscalationErr::NoCeilingForClass)
        );
    }

    #[test]
    fn unknown_failed_lane_is_an_honest_caller_defect() {
        let lanes = [ladder_lane(
            "good",
            crate::lane::LaneTier::Free,
            0.001,
            0.95,
            9,
        )];
        let caps = caps_for("someone-else", 1, &[true]);
        assert_eq!(
            escalate(
                &ctx("ghost", 1, 0.0),
                &caps,
                &lanes,
                &policy_with_ceiling(10.0)
            ),
            Err(EscalationErr::UnknownFailedLane)
        );
    }

    #[test]
    fn budget_walks_past_a_pricey_rung_to_a_cheap_better_one() {
        let caps = caps_for("cheap", 1, &[true, true, false]); // EWMA 0.7
        let lanes = [
            // Smallest quality step but too dear for the remaining budget.
            ladder_lane("mid-rung", crate::lane::LaneTier::Mid, 8.0, 0.72, 6),
            // Higher rung, CHEAPER hop — R1's point: never stop the search
            // at the first budget-blocked rung.
            ladder_lane("top-rung", crate::lane::LaneTier::Premium, 1.0, 0.95, 9),
        ];
        // spent 5 + 8 = 13 > 10 (skip); 5 + 1 = 6 <= 10 (take).
        let e = escalate(
            &ctx("cheap", 1, 5.0),
            &caps,
            &lanes,
            &policy_with_ceiling(10.0),
        )
        .expect("walks the ladder");
        assert_eq!(e.to_lane_id, "top-rung");
        assert!((e.projected_chain_usd - 6.0).abs() < 1e-12);

        // Same ladder, tighter ceiling: NOTHING fits -> OverCeiling.
        assert_eq!(
            escalate(
                &ctx("cheap", 1, 5.0),
                &caps,
                &lanes,
                &policy_with_ceiling(5.9)
            ),
            Err(EscalationErr::OverCeiling)
        );
        // Boundary is inclusive: projected == ceiling fits.
        let e2 = escalate(
            &ctx("cheap", 1, 5.0),
            &caps,
            &lanes,
            &policy_with_ceiling(6.0),
        )
        .expect("boundary includes");
        assert_eq!(e2.to_lane_id, "top-rung");
    }

    #[test]
    fn secret_stays_local_on_the_ladder_too() {
        // F5 holds during escalation: Secret permits Local only, so the
        // cloud rungs above the failed LOCAL lane are invisible rungs.
        let caps = caps_for("local-a", 1, &[true, true, false]); // EWMA 0.7
        let lanes = [
            ladder_lane("local-a", crate::lane::LaneTier::Local, 0.0, 0.7, 3),
            ladder_lane("cloud-best", crate::lane::LaneTier::Premium, 0.001, 0.99, 9),
        ];
        let mut secret = ctx("local-a", 1, 0.0);
        secret.data_class = DataClass::Secret;
        assert_eq!(
            escalate(&secret, &caps, &lanes, &policy_with_ceiling(10.0)),
            Err(EscalationErr::TopOfLadder)
        );

        // A better LOCAL rung exists -> the climb stays inside the fence.
        let with_local = [
            ladder_lane("local-a", crate::lane::LaneTier::Local, 0.0, 0.7, 3),
            ladder_lane("local-b", crate::lane::LaneTier::Local, 0.0, 0.9, 7),
        ];
        let e = escalate(&secret, &caps, &with_local, &policy_with_ceiling(10.0))
            .expect("climbs locally");
        assert_eq!(e.to_lane_id, "local-b");
        assert_eq!(e.to_lane_tier, LaneTier::Local);
    }

    #[test]
    fn decayed_and_unestablished_lanes_are_not_rungs() {
        let caps = caps_for("cheap", 1, &[true, true, false]); // EWMA 0.7
        let mut decayed = ladder_lane("decayed", crate::lane::LaneTier::Premium, 0.001, 0.95, 9);
        decayed.caps.insert(
            "chair".to_string(),
            Capability {
                quality: 0.95,
                samples: 9,
                consecutive_failures: HYSTERESIS_FAILS,
            },
        );
        // Still in holdout (samples < 5): never a rung regardless of quality.
        let thin = ladder_lane("thin", crate::lane::LaneTier::Premium, 0.001, 0.95, 4);
        assert_eq!(
            escalate(
                &ctx("cheap", 1, 0.0),
                &caps,
                &[decayed, thin],
                &policy_with_ceiling(10.0)
            ),
            Err(EscalationErr::TopOfLadder)
        );
    }

    #[test]
    fn equal_rungs_tie_break_like_route_cost_tier_id() {
        let caps = caps_for("cheap", 1, &[true, true, false]); // EWMA 0.7
        let lanes = [
            ladder_lane("z-local", crate::lane::LaneTier::Local, 0.02, 0.8, 6),
            ladder_lane("a-free", crate::lane::LaneTier::Free, 0.02, 0.8, 6),
            ladder_lane("cheap-free", crate::lane::LaneTier::Free, 0.01, 0.8, 6),
        ];
        // Same quality: cheaper wins; cost tie would break Local-first, id
        // last — cheap-free (0.01) wins over both 0.02 lanes.
        let e = escalate(
            &ctx("cheap", 1, 0.0),
            &caps,
            &lanes,
            &policy_with_ceiling(10.0),
        )
        .expect("deterministic");
        assert_eq!(e.to_lane_id, "cheap-free");
    }
}
