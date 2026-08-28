//! The P1 decision core: a PURE function from (profile, data class, lanes,
//! policy) to ONE decision row or an honest error. No I/O, no clock, no LLM
//! — same inputs always yield the same decision (auditability by construction).
//!
//! Filter ladder, each emptiness its own honest error:
//! alive -> tier-permitted (F5) -> class-measured with N >= min_samples (F2)
//! -> quality >= floor (F6/R9). Selection among the suitable: CHEAPEST
//! (O3), ties Local > Free > Mid > Premium (LaneTier Ord), final tie lane id
//! — total order, zero coin flips.
//!
//! R4 degraded mode: every measured lane below floor -> Internal/Public
//! proceed on the best measured lane with `degraded = true` (P4 raises the
//! operator alert); Secret/Pii FAIL CLOSED.

use crate::lane::{DataClass, Lane, LaneTier};
use crate::policy::RoutePolicy;
use crate::profile::TaskProfile;

/// The decision row a task card references via `route_id` (F3: a ledger ROW,
/// not a card). P2 persists these into the append-only stream; P1 only
/// produces the value. `route_id` is the deterministic reference key
/// (card::lane) — uniqueness per routing EVENT is the ledger's seq job.
#[derive(Debug, Clone, PartialEq)]
pub struct RouteDecision {
    pub route_id: String,
    pub card_id: String,
    pub task_class: String,
    pub lane_id: String,
    pub lane_tier: LaneTier,
    pub cost_per_task_usd: f64,
    /// R4: selected BELOW floor because no measured lane of the class reached
    /// it. Carried on the row so the alert can never be lost downstream.
    pub degraded: bool,
}

#[derive(Debug, PartialEq)]
pub enum RouteErr {
    /// Not one lane alive.
    NoAliveLane,
    /// F5: the data-class tier filter emptied the pool — fail closed.
    NoPermittedLane,
    /// F2: alive + permitted lanes exist, but none is measured for the class
    /// with N >= min_samples. Cold-start family medians are P2's job; P1
    /// refuses to guess.
    NoMeasuredLane,
    /// F6: no floor pinned for this class — thresholds are never guessed.
    NoFloorForClass,
    /// R4: every measured lane below floor on a Secret/Pii task — fail closed.
    BelowFloorFailClosed,
}

pub fn route(
    profile: &TaskProfile,
    data_class: DataClass,
    lanes: &[Lane],
    policy: &RoutePolicy,
) -> Result<RouteDecision, RouteErr> {
    let floor = policy
        .floor(&profile.class)
        .ok_or(RouteErr::NoFloorForClass)?;

    let alive: Vec<&Lane> = lanes.iter().filter(|l| l.alive).collect();
    if alive.is_empty() {
        return Err(RouteErr::NoAliveLane);
    }

    let permitted: Vec<&Lane> = alive
        .into_iter()
        .filter(|l| policy.permits(data_class, l.tier))
        .collect();
    if permitted.is_empty() {
        return Err(RouteErr::NoPermittedLane);
    }

    let measured: Vec<&Lane> = permitted
        .into_iter()
        .filter(|l| {
            l.caps.get(&profile.class).is_some_and(|c| {
                c.samples >= policy.min_samples()
                    // P3 decay wiring (QQ2/R9): a lane at hysteresis
                    // (HYSTERESIS_FAILS trailing fails) is DECAYED — out of
                    // the cheap pool. One pass clears the counter; the
                    // floor filter below still guards re-entry.
                    && c.consecutive_failures < crate::stats::HYSTERESIS_FAILS
            })
        })
        .collect();
    if measured.is_empty() {
        return Err(RouteErr::NoMeasuredLane);
    }

    let quality = |l: &Lane| l.caps.get(&profile.class).map(|c| c.quality);
    let above: Vec<&Lane> = measured
        .iter()
        .copied()
        .filter(|l| quality(l).is_some_and(|q| q >= floor))
        .collect();

    let (pool, degraded) = if above.is_empty() {
        if matches!(data_class, DataClass::Secret | DataClass::Pii) {
            return Err(RouteErr::BelowFloorFailClosed);
        }
        (measured, true)
    } else {
        (above, false)
    };

    // NaN quality never passes `q >= floor`, and NaN cost compares Equal in
    // the key — determinism survives even malformed capability rows.
    let best = pool
        .into_iter()
        .min_by(|a, b| {
            ord_key(a)
                .partial_cmp(&ord_key(b))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .expect("pool is non-empty by construction");

    Ok(RouteDecision {
        route_id: format!("{}::{}", profile.card_id, best.id),
        card_id: profile.card_id.clone(),
        task_class: profile.class.clone(),
        lane_id: best.id.clone(),
        lane_tier: best.tier,
        cost_per_task_usd: best.cost_per_task_usd,
        degraded,
    })
}

/// Total order key: (cost, tier, id). Cost compares WITHIN a class only (O3);
/// tier preference is the LaneTier Ord (Local first); id is the final
/// deterministic tiebreak.
fn ord_key(l: &Lane) -> (f64, LaneTier, &str) {
    (l.cost_per_task_usd, l.tier, l.id.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lane::Capability;
    use std::collections::BTreeMap;

    fn cap(quality: f64, samples: u32) -> Capability {
        Capability {
            quality,
            samples,
            consecutive_failures: 0,
        }
    }

    fn cap_decay(quality: f64, samples: u32, fails: u32) -> Capability {
        Capability {
            quality,
            samples,
            consecutive_failures: fails,
        }
    }

    fn lane(id: &str, tier: LaneTier, cost: f64, quality: f64, samples: u32) -> Lane {
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

    fn profile() -> TaskProfile {
        TaskProfile {
            card_id: "C-1".to_string(),
            class: "chair".to_string(),
            done_when: "widget ships".to_string(),
            red_test: "npm test green".to_string(),
        }
    }

    #[test]
    fn no_floor_for_class_fails_closed() {
        let p = TaskProfile {
            class: "unrated".to_string(),
            ..profile()
        };
        let r = route(
            &p,
            DataClass::Public,
            &[lane("l", LaneTier::Free, 0.0, 0.99, 9)],
            &RoutePolicy::default(),
        );
        assert_eq!(r, Err(RouteErr::NoFloorForClass));
    }

    #[test]
    fn no_alive_lane_is_its_own_error() {
        let mut dead = lane("l", LaneTier::Local, 0.0, 0.99, 9);
        dead.alive = false;
        assert_eq!(
            route(
                &profile(),
                DataClass::Public,
                &[dead],
                &RoutePolicy::default()
            ),
            Err(RouteErr::NoAliveLane)
        );
    }

    #[test]
    fn secret_never_leaves_local() {
        let foreign = lane("cloud-free", LaneTier::Free, 0.0, 0.99, 9);
        assert_eq!(
            route(
                &profile(),
                DataClass::Secret,
                &[foreign],
                &RoutePolicy::default()
            ),
            Err(RouteErr::NoPermittedLane)
        );
    }

    #[test]
    fn cold_start_refuses_to_guess() {
        let never_ran = {
            let mut l = lane("l", LaneTier::Local, 0.0, 0.99, 9);
            l.caps.remove("chair");
            l
        };
        assert_eq!(
            route(
                &profile(),
                DataClass::Public,
                &[never_ran],
                &RoutePolicy::default()
            ),
            Err(RouteErr::NoMeasuredLane)
        );
        let thin = lane("l", LaneTier::Local, 0.0, 0.99, 4); // F2: N < 5
        assert_eq!(
            route(
                &profile(),
                DataClass::Public,
                &[thin],
                &RoutePolicy::default()
            ),
            Err(RouteErr::NoMeasuredLane)
        );
    }

    #[test]
    fn cheapest_above_floor_wins_and_floor_boundary_includes() {
        // floor(chair)=0.70; 0.70 exactly is ABOVE the floor (>=).
        let pricey = lane("premium-lane", LaneTier::Premium, 0.10, 0.95, 9);
        let cheap = lane("free-lane", LaneTier::Free, 0.001, 0.70, 5);
        let r = route(
            &profile(),
            DataClass::Public,
            &[pricey, cheap],
            &RoutePolicy::default(),
        )
        .expect("selects");
        assert_eq!(r.lane_id, "free-lane");
        assert!((r.cost_per_task_usd - 0.001).abs() < f64::EPSILON);
        assert!(!r.degraded);
        assert_eq!(r.route_id, "C-1::free-lane");
        assert_eq!(r.task_class, "chair");
    }

    #[test]
    fn cost_tie_breaks_local_first_then_id() {
        let r = route(
            &profile(),
            DataClass::Public,
            &[
                lane("a-free", LaneTier::Free, 0.01, 0.9, 6),
                lane("z-local", LaneTier::Local, 0.01, 0.9, 6),
            ],
            &RoutePolicy::default(),
        )
        .expect("selects");
        assert_eq!(r.lane_id, "z-local");

        let r2 = route(
            &profile(),
            DataClass::Public,
            &[
                lane("z-local", LaneTier::Local, 0.01, 0.9, 6),
                lane("a-local", LaneTier::Local, 0.01, 0.9, 6),
            ],
            &RoutePolicy::default(),
        )
        .expect("selects");
        assert_eq!(r2.lane_id, "a-local"); // id is the final tiebreak
    }

    #[test]
    fn below_floor_degrades_for_public_fails_closed_for_secret() {
        // Local tier: permitted for BOTH Public and Secret (default F5
        // allowlists), so the difference in outcomes isolates R4 alone.
        let weak = lane("weak", LaneTier::Local, 0.001, 0.40, 9);
        let r = route(
            &profile(),
            DataClass::Public,
            std::slice::from_ref(&weak),
            &RoutePolicy::default(),
        )
        .expect("degraded proceed");
        assert!(r.degraded);
        assert_eq!(r.lane_id, "weak");
        assert_eq!(
            route(
                &profile(),
                DataClass::Secret,
                &[weak],
                &RoutePolicy::default()
            ),
            Err(RouteErr::BelowFloorFailClosed)
        );
    }

    #[test]
    fn below_floor_lane_loses_to_above_floor_even_when_cheaper() {
        // R9: below-floor lanes LEAVE the pool; cheap cannot buy back in.
        let cheap_weak = lane("cheap-weak", LaneTier::Free, 0.0001, 0.50, 9);
        let pricey_good = lane("pricey-good", LaneTier::Premium, 0.5, 0.95, 9);
        let r = route(
            &profile(),
            DataClass::Public,
            &[cheap_weak, pricey_good],
            &RoutePolicy::default(),
        )
        .expect("selects");
        assert_eq!(r.lane_id, "pricey-good");
        assert!(!r.degraded);
    }

    #[test]
    fn dead_lane_measured_for_class_still_not_selected() {
        let mut dead = lane("dead-good", LaneTier::Local, 0.0, 0.99, 9);
        dead.alive = false;
        let alive_weak = lane("alive-weak", LaneTier::Free, 0.001, 0.40, 9);
        let r = route(
            &profile(),
            DataClass::Public,
            &[dead, alive_weak],
            &RoutePolicy::default(),
        )
        .expect("degraded proceed on the ALIVE one");
        assert_eq!(r.lane_id, "alive-weak");
        assert!(r.degraded);
    }

    #[test]
    fn hysteresis_decayed_lane_leaves_the_pool_even_above_floor() {
        // P3 decay wiring: quality 0.95 CLEARS the chair floor (0.70), but
        // 2 trailing RED-TEST fails = DECAYED — cheap cannot buy back in.
        let mut decayed_cheap = lane("decayed-cheap", LaneTier::Free, 0.0001, 0.95, 9);
        decayed_cheap
            .caps
            .insert("chair".to_string(), cap_decay(0.95, 9, 2));
        let pricey_good = lane("pricey-good", LaneTier::Premium, 0.5, 0.95, 9);
        let r = route(
            &profile(),
            DataClass::Public,
            &[decayed_cheap, pricey_good.clone()],
            &RoutePolicy::default(),
        )
        .expect("selects the healthy lane");
        assert_eq!(r.lane_id, "pricey-good");
        assert!(!r.degraded);

        // One fail is NOT decay — hysteresis demands two consecutive.
        let mut one_fail = lane("one-fail", LaneTier::Free, 0.0001, 0.95, 9);
        one_fail
            .caps
            .insert("chair".to_string(), cap_decay(0.95, 9, 1));
        let r2 = route(
            &profile(),
            DataClass::Public,
            &[one_fail.clone(), pricey_good.clone()],
            &RoutePolicy::default(),
        )
        .expect("one fail still selectable");
        assert_eq!(r2.lane_id, "one-fail");
    }

    #[test]
    fn decayed_lane_alone_degrades_or_fails_like_any_unmeasured_lane() {
        let mut decayed = lane("decayed", LaneTier::Local, 0.0, 0.95, 9);
        decayed
            .caps
            .insert("chair".to_string(), cap_decay(0.95, 9, 2));
        // The ONLY measured lane is decayed -> the measured pool is empty:
        // honest NoMeasuredLane (the P4 alert reads the caps report to say
        // WHY — decay, not cold-start).
        assert_eq!(
            route(
                &profile(),
                DataClass::Public,
                &[decayed],
                &RoutePolicy::default()
            ),
            Err(RouteErr::NoMeasuredLane)
        );
    }

    #[test]
    fn one_pass_heals_the_fold_end_to_end() {
        // QQ2 auto-recovery through the REAL fold: 4 passes, 2 fails
        // (decayed, EWMA 0.49 < floor 0.70 anyway), then 2 passes heal the
        // counter (0) AND the EWMA to 0.3 + 0.7*0.643 = 0.750 >= floor —
        // the lane re-enters selection with the floor still guarding it.
        let mut lines = Vec::new();
        let mut seq = 0;
        for pass in [true, true, true, true, false, false, true, true] {
            seq += 1;
            lines.push(format!(
                "{{\"seq\":{seq},\"ts\":\"t\",\"kind\":\"outcome\",\"card_id\":\"c\",\
                 \"task_class\":\"chair\",\"lane_id\":\"healer\",\"model\":\"m\",\
                 \"cost_tokens\":1,\"cost_usd_est\":0.001,\"latency_ms\":10,\
                 \"verify_outcome\":\"{}\",\"escalated_to\":null}}",
                if pass { "pass" } else { "fail" }
            ));
        }
        let loaded = crate::ledger::parse_stream(&lines.join("\n"));
        let caps = crate::stats::CapsReport::from_rows(&loaded);
        let mut lane = Lane {
            id: "healer".to_string(),
            family: "test".to_string(),
            tier: LaneTier::Free,
            alive: true,
            cost_per_task_usd: 0.001,
            caps: caps.p1_caps("healer"),
        };
        let healed = lane.caps.get("chair").expect("folded");
        assert_eq!(healed.consecutive_failures, 0);
        assert!(healed.quality >= 0.70, "ewma {} >= floor", healed.quality);
        let r = route(
            &profile(),
            DataClass::Public,
            std::slice::from_ref(&lane),
            &RoutePolicy::default(),
        )
        .expect("healed lane selected");
        assert_eq!(r.lane_id, "healer");
        assert!(!r.degraded);

        // And mid-decay (after exactly the 2 fails) the SAME lane is out.
        let mid = crate::ledger::parse_stream(&lines[..6].join("\n"));
        let mid_caps = crate::stats::CapsReport::from_rows(&mid);
        lane.caps = mid_caps.p1_caps("healer");
        assert_eq!(
            route(
                &profile(),
                DataClass::Public,
                std::slice::from_ref(&lane),
                &RoutePolicy::default()
            ),
            Err(RouteErr::NoMeasuredLane)
        );
    }
}
