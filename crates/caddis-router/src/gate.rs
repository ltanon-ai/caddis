//! P4 slice 3 — the DISPATCH-PATH GATE: the one surface the real dispatch
//! paths (TinyAGI consult / omp task / bee card consume) call instead of the
//! pure functions. F1 holds: the gate NEVER dispatches — it makes every
//! [`crate::route::route`] refusal and every [`crate::escalation::escalate`]
//! refusal LOUD (operator alert, persisted) and every routing decision REAL
//! (a ledger row, F3 — a decision that is not a row never happened).
//!
//! Laws landed here:
//! - **F3** on success the decision row is APPENDED before the caller may
//!   dispatch. If the row cannot persist, the routing event does not exist:
//!   [`GateErr::Ledger`] fail-closes the dispatch. Nothing is announced that
//!   did not happen.
//! - **R4** a `degraded = true` selection (Internal/Public only; Secret/Pii
//!   fail closed inside route()) is announced BEFORE the run may proceed:
//!   the Degraded alert is appended first, and if the alert cannot persist
//!   the dispatch fail-closes ([`GateErr::Alert`]) — a degraded run the
//!   operator never heard of is the one failure mode R4 exists to prevent.
//!   Alert-then-row is the mandatory order on this path; if the row append
//!   then fails, the alert stays TRUE (route() did engage degraded mode) and
//!   [`GateErr::Ledger`] still stops the dispatch.
//! - **F5/P4 slice 2** every route() refusal persists one
//!   [`Alert::from_route_stop`] row; every escalate() refusal persists one
//!   [`Alert::from_escalation_stop`] row. If the alert itself cannot persist,
//!   the refusal error still propagates carrying [`GateErr::Route`] /
//!   [`GateErr::Escalation`] with the secondary alert failure alongside —
//!   the dispatch path still halts (fail closed), the lost loudness is
//!   visible in the error, never swallowed.
//! - **R10 reading (P3 ruling)** the escalation CLIMB needs no second
//!   decision row here: the climb run's persistence is the OUTCOME row the
//!   dispatch path already appends after the run (`escalated_to` names the
//!   rung). [`EscalationCtx`] deliberately carries no card identity; a
//!   decision row without it would be fabricated data. If a later ruling
//!   wants the climb as its own decision row, the ctx gains card identity
//!   first.
//!
//! Purity split (F1): the gate is the ONLY module outside `ledger`/`alerts`
//! allowed to write, and only through their append laws (R6). No lane
//! registry, no clock, no probing lives here — lanes and policy are INPUTS
//! the dispatch path owns (registry slice follows).

use crate::alerts::{Alert, AlertErr, AlertKind, Alerts};
use crate::escalation::{escalate, Escalation, EscalationCtx, EscalationErr};
use crate::lane::{DataClass, Lane};
use crate::ledger::{DecisionRow, Ledger, LedgerErr, Row};
use crate::policy::RoutePolicy;
use crate::profile::TaskProfile;
use crate::route::{route, RouteDecision, RouteErr};

/// The dispatch-path gate: one ledger + one alert stream, one task at a
/// time. Construct per routing event; it holds no state of its own.
pub struct Gate<'a> {
    ledger: &'a Ledger,
    alerts: &'a Alerts,
}

impl<'a> Gate<'a> {
    pub fn new(ledger: &'a Ledger, alerts: &'a Alerts) -> Self {
        Gate { ledger, alerts }
    }

    /// Route one task card, or halt loudly. On success the decision row IS
    /// in the ledger (F3) before this returns; the caller may dispatch on
    /// the returned [`RouteDecision`] only.
    pub fn route_gated(
        &self,
        profile: &TaskProfile,
        data_class: DataClass,
        lanes: &[Lane],
        policy: &RoutePolicy,
    ) -> Result<(RouteDecision, u64), GateErr> {
        let decision = match route(profile, data_class, lanes, policy) {
            Ok(d) => d,
            Err(e) => {
                let alert =
                    self.alerts
                        .append(&Alert::from_route_stop(data_class, &profile.class, &e));
                return Err(GateErr::Route(e, alert.err()));
            }
        };
        // R4: degraded runs are announced before they may happen. The alert
        // carries the CHOSEN lane + class; the detail names the law.
        if decision.degraded {
            let alert = Alert {
                kind: AlertKind::Degraded,
                lane_id: decision.lane_id.clone(),
                class: decision.task_class.clone(),
                detail: format!(
                    "R4: routed below floor on {} (degraded) — best measured lane, proceed allowed",
                    data_class.as_str()
                ),
            };
            self.alerts.append(&alert).map_err(GateErr::Alert)?;
        }
        // F3: the decision IS the row. An unpersisted decision = no dispatch.
        let seq = self
            .ledger
            .append(&Row::Decision(DecisionRow::from(&decision)))?;
        Ok((decision, seq))
    }

    /// Escalate after a verified RED-TEST fail, or halt loudly. On success
    /// nothing is written (R10 reading above — the climb run's outcome row
    /// carries `escalated_to`); on refusal the stop alert IS in the stream.
    pub fn escalate_gated(
        &self,
        ctx: &EscalationCtx,
        caps: &crate::stats::CapsReport,
        lanes: &[Lane],
        policy: &RoutePolicy,
    ) -> Result<Escalation, GateErr> {
        match escalate(ctx, caps, lanes, policy) {
            Ok(e) => Ok(e),
            Err(e) => {
                let alert = self.alerts.append(&Alert::from_escalation_stop(ctx, &e));
                Err(GateErr::Escalation(e, alert.err()))
            }
        }
    }
}

#[derive(Debug)]
pub enum GateErr {
    /// route() refused (fail closed). The Option carries a SECONDARY failure
    /// to persist the RouteStop alert — the halt stands either way; the lost
    /// loudness is reported, never swallowed.
    Route(RouteErr, Option<AlertErr>),
    /// escalate() refused (fail safe). Same secondary-alert law as Route.
    Escalation(EscalationErr, Option<AlertErr>),
    /// The decision row (F3) could not be appended — the routing event does
    /// not exist; the caller must NOT dispatch. No stop alert is emitted: a
    /// RouteStop would lie (routing did not stop — the organ did); the fault
    /// is the caller's to surface and `verify` shows the stream honestly.
    Ledger(LedgerErr),
    /// R4: the Degraded announcement could not be persisted — a degraded run
    /// the operator never heard of is forbidden; the dispatch fail-closes.
    Alert(AlertErr),
}

// io::Error is not PartialEq: compare by KIND (same law as ledger/alerts).
impl PartialEq for GateErr {
    fn eq(&self, other: &Self) -> bool {
        use GateErr as G;
        match (self, other) {
            (G::Route(a, x), G::Route(b, y)) => a == b && x == y,
            (G::Escalation(a, x), G::Escalation(b, y)) => a == b && x == y,
            (G::Ledger(a), G::Ledger(b)) => a == b,
            (G::Alert(a), G::Alert(b)) => a == b,
            _ => false,
        }
    }
}

impl std::fmt::Display for GateErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use GateErr as G;
        match self {
            G::Route(e, None) => write!(f, "route refused (alert persisted): {e:?}"),
            G::Route(e, Some(a)) => write!(f, "route refused: {e:?}; ALERT LOST: {a}"),
            G::Escalation(e, None) => write!(f, "escalation stopped (alert persisted): {e:?}"),
            G::Escalation(e, Some(a)) => write!(f, "escalation stopped: {e:?}; ALERT LOST: {a}"),
            G::Ledger(e) => write!(f, "decision row not persisted — no dispatch (F3): {e}"),
            G::Alert(e) => write!(
                f,
                "degraded announcement not persisted — no dispatch (R4): {e}"
            ),
        }
    }
}

impl From<LedgerErr> for GateErr {
    fn from(e: LedgerErr) -> Self {
        GateErr::Ledger(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alerts::parse_alert_stream;
    use crate::escalation::MAX_HOPS;
    use crate::lane::{Capability, LaneTier};
    use crate::ledger::parse_stream;
    use crate::stats::CapsReport;
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::PathBuf;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rtr-gate-{}-{}", tag, std::process::id()));
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn home(tag: &str) -> (PathBuf, Ledger, Alerts) {
        let d = tmpdir(tag);
        let ledger = Ledger::new(d.join("ledger.jsonl"));
        let alerts = Alerts::new(d.join("alerts.jsonl"));
        (d, ledger, alerts)
    }

    /// One alive, measured, floor-clearing free lane for `coding`.
    fn good_lane() -> Lane {
        lane("groq", LaneTier::Free, 0.9)
    }

    fn lane(id: &str, tier: LaneTier, quality: f64) -> Lane {
        let mut caps = BTreeMap::new();
        caps.insert(
            "coding".to_string(),
            Capability {
                quality,
                samples: 10,
                consecutive_failures: 0,
            },
        );
        Lane {
            id: id.to_string(),
            family: "free".to_string(),
            tier,
            alive: true,
            cost_per_task_usd: 0.001,
            caps,
        }
    }

    fn profile() -> TaskProfile {
        TaskProfile {
            card_id: "T-41".to_string(),
            class: "coding".to_string(),
            done_when: "The gate work ships.".to_string(),
            red_test: "cargo test -p caddis-router green.".to_string(),
        }
    }

    fn policy() -> RoutePolicy {
        let mut p = RoutePolicy::default();
        p.set_floor("coding", 0.85);
        p
    }

    #[test]
    fn success_persists_decision_row_and_no_alert() {
        let (_d, ledger, alerts) = home("ok");
        let gate = Gate::new(&ledger, &alerts);
        let (dec, seq) = gate
            .route_gated(&profile(), DataClass::Internal, &[good_lane()], &policy())
            .expect("routes");
        assert!(!dec.degraded);
        assert_eq!(dec.lane_id, "groq");
        assert_eq!(seq, 1);
        let loaded = ledger.load().unwrap();
        assert_eq!(loaded.bad.len(), 0);
        assert_eq!(loaded.rows.len(), 1);
        match &loaded.rows[0].row {
            Row::Decision(d) => {
                assert_eq!(d.card_id, "T-41");
                assert_eq!(d.task_class, "coding");
                assert_eq!(d.lane_id, "groq");
                assert!(!d.degraded);
            }
            other => panic!("expected decision row, got {other:?}"),
        }
        // Clean routing is SILENT: zero alerts.
        assert_eq!(alerts.load().unwrap().rows.len(), 0);
    }

    #[test]
    fn refusal_persists_route_stop_alert_and_no_row() {
        let (_d, ledger, alerts) = home("refuse");
        let gate = Gate::new(&ledger, &alerts);
        // No floor ruled for the class -> F6 refusal.
        let err = gate
            .route_gated(
                &profile(),
                DataClass::Internal,
                &[good_lane()],
                &RoutePolicy::default(),
            )
            .unwrap_err();
        assert_eq!(
            err,
            GateErr::Route(RouteErr::NoFloorForClass, None),
            "refusal carries the honest stop + a persisted alert"
        );
        // No decision row: nothing was decided.
        let loaded = ledger.load().unwrap();
        assert_eq!(loaded.rows.len(), 0);
        // One RouteStop alert naming the class + filter.
        let la = alerts.load().unwrap();
        assert_eq!(la.rows.len(), 1);
        let a = &la.rows[0].alert;
        assert_eq!(a.kind, AlertKind::RouteStop);
        assert_eq!(a.class, "coding");
        assert!(a.detail.contains("floor"), "detail names the closed filter");
    }

    #[test]
    fn degraded_run_is_announced_before_the_row() {
        let (_d, ledger, alerts) = home("degraded");
        let gate = Gate::new(&ledger, &alerts);
        // Below-floor capability on Internal -> R4 degraded proceed.
        let weak = lane("groq", LaneTier::Free, 0.5);
        let (dec, seq) = gate
            .route_gated(&profile(), DataClass::Internal, &[weak], &policy())
            .expect("degraded proceed is allowed on Internal");
        assert!(dec.degraded);
        assert_eq!(seq, 1);
        let la = alerts.load().unwrap();
        assert_eq!(la.rows.len(), 1, "exactly one Degraded announcement");
        let a = &la.rows[0].alert;
        assert_eq!(a.kind, AlertKind::Degraded);
        assert_eq!(a.lane_id, "groq");
        assert_eq!(a.class, "coding");
        // The row is the decision WITH degraded=true.
        let loaded = ledger.load().unwrap();
        match &loaded.rows[0].row {
            Row::Decision(d) => assert!(d.degraded),
            other => panic!("expected decision row, got {other:?}"),
        }
    }

    #[test]
    fn degraded_with_unpersistable_alert_fails_closed() {
        let d = tmpdir("degraded-alert-dead");
        let ledger = Ledger::new(d.join("ledger.jsonl"));
        // The alerts path IS a directory: every append fails with Io.
        let alerts = Alerts::new(d.join("alerts.jsonl"));
        fs::create_dir_all(d.join("alerts.jsonl")).unwrap();
        let gate = Gate::new(&ledger, &alerts);
        let weak = lane("groq", LaneTier::Free, 0.5);
        let err = gate
            .route_gated(&profile(), DataClass::Internal, &[weak], &policy())
            .unwrap_err();
        assert!(matches!(err, GateErr::Alert(_)), "got {err:?}");
        // Fail closed: NO decision row either.
        assert_eq!(ledger.load().unwrap().rows.len(), 0);
    }

    #[test]
    fn secret_below_floor_never_degrades() {
        let (_d, ledger, alerts) = home("secret");
        let gate = Gate::new(&ledger, &alerts);
        // Local tier is Secret-permitted (F5 prior), measured below floor:
        // the ONLY remaining filter is the floor — and Secret fails closed.
        let weak = lane("ollama", LaneTier::Local, 0.5);
        let err = gate
            .route_gated(&profile(), DataClass::Secret, &[weak], &policy())
            .unwrap_err();
        assert_eq!(err, GateErr::Route(RouteErr::BelowFloorFailClosed, None));
        let la = alerts.load().unwrap();
        assert_eq!(la.rows.len(), 1);
        assert_eq!(la.rows[0].alert.kind, AlertKind::RouteStop);
        assert!(la.rows[0].alert.detail.contains("secret"));
        assert_eq!(ledger.load().unwrap().rows.len(), 0);
    }

    #[test]
    fn refusal_with_dead_alert_stream_reports_lost_loudness() {
        let d = tmpdir("refuse-alert-dead");
        let ledger = Ledger::new(d.join("ledger.jsonl"));
        let alerts = Alerts::new(d.join("alerts.jsonl"));
        fs::create_dir_all(d.join("alerts.jsonl")).unwrap();
        let gate = Gate::new(&ledger, &alerts);
        // No lanes at all: NoAliveLane + alert persistence fails.
        let err = gate
            .route_gated(&profile(), DataClass::Internal, &[], &policy())
            .unwrap_err();
        match err {
            GateErr::Route(RouteErr::NoAliveLane, Some(_)) => {}
            other => panic!("expected Route(NoAliveLane, Some(alert err)), got {other:?}"),
        }
        // The halt still left no decision row.
        assert_eq!(ledger.load().unwrap().rows.len(), 0);
    }

    #[test]
    fn escalate_stop_persists_alert_no_ledger_row() {
        let (_d, ledger, alerts) = home("escalate");
        let gate = Gate::new(&ledger, &alerts);
        // MaxHops: the cheapest independent check, no policy needed.
        let ctx = EscalationCtx {
            task_class: "coding".to_string(),
            data_class: DataClass::Internal,
            failed_lane_id: "groq".to_string(),
            hops_so_far: MAX_HOPS,
            chain_spent_usd: 0.0,
        };
        let err = gate
            .escalate_gated(&ctx, &CapsReport::default(), &[], &policy())
            .unwrap_err();
        assert_eq!(
            err,
            GateErr::Escalation(EscalationErr::MaxHops, None),
            "stop carries the honest reason + a persisted alert"
        );
        let la = alerts.load().unwrap();
        assert_eq!(la.rows.len(), 1);
        let a = &la.rows[0].alert;
        assert_eq!(a.kind, AlertKind::EscalationStop);
        assert_eq!(a.lane_id, "groq");
        assert_eq!(a.class, "coding");
        assert_eq!(ledger.load().unwrap().rows.len(), 0);
    }

    #[test]
    fn escalate_success_writes_neither_stream() {
        let (_d, ledger, alerts) = home("escalate-ok");
        let gate = Gate::new(&ledger, &alerts);
        // Seed the fold through the REAL ledger parser: 6 passes on
        // nemotron, 1 fail on groq (decay-before-climb), clean wire format.
        let mut lines = Vec::new();
        for seq in 1..=6 {
            lines.push(outcome(seq, "nemotron", true));
        }
        lines.push(outcome(7, "groq", false));
        let loaded = parse_stream(&lines.join("\n"));
        assert_eq!(loaded.bad.len(), 0, "fixture must be clean wire format");
        let caps = CapsReport::from_rows(&loaded);

        let mut lanes = vec![good_lane()];
        lanes.push(lane("nemotron", LaneTier::Free, 0.95));
        let mut p = policy();
        p.set_ceiling("coding", 1.0);
        let ctx = EscalationCtx {
            task_class: "coding".to_string(),
            data_class: DataClass::Internal,
            failed_lane_id: "groq".to_string(),
            hops_so_far: 1,
            chain_spent_usd: 0.001,
        };
        let esc = gate
            .escalate_gated(&ctx, &caps, &lanes, &p)
            .expect("climb exists");
        assert_eq!(esc.from_lane_id, "groq");
        assert_eq!(esc.to_lane_id, "nemotron");
        // R10 reading: NO new stream facts on a climb decision.
        assert_eq!(ledger.load().unwrap().rows.len(), 0);
        assert_eq!(alerts.load().unwrap().rows.len(), 0);
    }

    #[test]
    fn two_gated_routes_append_in_order() {
        let (_d, ledger, alerts) = home("seq");
        let gate = Gate::new(&ledger, &alerts);
        let (_, s1) = gate
            .route_gated(&profile(), DataClass::Internal, &[good_lane()], &policy())
            .unwrap();
        let (_, s2) = gate
            .route_gated(&profile(), DataClass::Internal, &[good_lane()], &policy())
            .unwrap();
        assert_eq!((s1, s2), (1, 2));
        let loaded = ledger.load().unwrap();
        assert_eq!(loaded.rows.len(), 2);
        assert_eq!(loaded.rows[1].seq, 2);
    }

    #[test]
    fn alert_stream_round_trips_through_the_real_parser() {
        let (_d, ledger, alerts) = home("wire");
        let gate = Gate::new(&ledger, &alerts);
        let _ = gate
            .route_gated(&profile(), DataClass::Internal, &[], &policy())
            .unwrap_err();
        // Re-read through the public parser: the persisted stop alert is a
        // first-class alert row, not a gate-private format.
        let raw = fs::read_to_string(alerts.path()).unwrap();
        let parsed = parse_alert_stream(&raw);
        assert_eq!(parsed.bad.len(), 0);
        assert_eq!(parsed.rows.len(), 1);
    }

    /// Hand-written wire line (alerts.rs test convention): the ledger's
    /// exact outcome-row shape.
    fn out(seq: u64, lane: &str, class: &str, pass: bool) -> String {
        format!(
            "{{\"seq\":{seq},\"ts\":\"2026-08-28T00:00:00Z\",\"kind\":\"outcome\",\"card_id\":\"C\",\"task_class\":\"{class}\",\"lane_id\":\"{lane}\",\"model\":\"m\",\"cost_tokens\":1,\"cost_usd_est\":0.001,\"latency_ms\":10,\"verify_outcome\":\"{}\",\"escalated_to\":null}}",
            if pass { "pass" } else { "fail" }
        )
    }

    fn outcome(seq: u64, lane: &str, pass: bool) -> String {
        out(seq, lane, "coding", pass)
    }
}
