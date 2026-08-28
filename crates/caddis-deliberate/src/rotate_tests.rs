//! rotate_tests.rs — the fake-probe battery (brief §6): every status
//! class, the Q6 ×3-unprobeable transition, streak reset, sweep
//! integration, the lock law, and the config exact-field law. No network:
//! the transport is an injected fn; the prober's own wire tests live in
//! prober_tests.rs.

use super::*;
use crate::registry::{Card, ProviderCard, SeatCard};
use crate::CostClass;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

fn provider(id: &str, base_url: &str, auth_path: &str) -> Card {
    Card::Provider(ProviderCard {
        id: id.into(),
        lane_type: LaneType::Http,
        base_url: base_url.into(),
        auth_path: auth_path.into(),
        probe_path: String::new(),
        caps: 1,
        source: "test".into(),
    })
}

fn seat(id: &str, provider_id: &str, state: crate::SeatState, since: u64) -> Card {
    Card::Seat(SeatCard {
        id: id.into(),
        provider: provider_id.into(),
        family: provider_id.into(),
        model: "test-model".into(),
        lane_type: LaneType::Http,
        cost_class: CostClass::Free,
        state,
        since_epoch_s: since,
        caps: 1,
        cost_in_usd_per_mtok: 0.0,
        cost_out_usd_per_mtok: 0.0,
        context_window: 8192,
        max_tokens: 4096,
        source: "test".into(),
    })
}

fn home_with(cards: &[Card]) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "caddis-rotate-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("mkdir home");
    std::fs::write(stream_path(&dir), registry::render_seed(cards)).expect("seed stream");
    dir
}

fn answered(status: u16) -> prober::ProbeOutcome {
    prober::ProbeOutcome {
        status: Some(status),
        error: None,
    }
}

fn refused(reason: &str) -> prober::ProbeOutcome {
    prober::ProbeOutcome {
        status: None,
        error: Some(reason.into()),
    }
}

/// Fake transport keyed by base_url.
fn router(responses: BTreeMap<&str, prober::ProbeOutcome>) -> impl ProbeFn {
    let m: BTreeMap<String, prober::ProbeOutcome> = responses
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
    move |base_url: &str, _probe_path: &str, _auth: &str, _cfg: &prober::ProbeCfg| {
        m.get(base_url)
            .cloned()
            .unwrap_or_else(|| refused("no route"))
    }
}

const NOW: u64 = 1_800_000_000;

fn default_home_single(base_url: &str) -> std::path::PathBuf {
    home_with(&[
        provider("prov", base_url, ""),
        seat("prov/m1", "prov", crate::SeatState::Probing, 0),
    ])
}

fn fold_view(home: &std::path::Path) -> Registry {
    let (reg, _) = registry::load_and_sync(&stream_path(home), &view_path(home)).expect("fold");
    reg
}

// ---------------------------------------------------------------------------
// Status classes (the ruled table, §5.3)
// ---------------------------------------------------------------------------

#[test]
fn class_200_lands_live() {
    let home = default_home_single("https://lane.example");
    let rep = rotate(
        &home,
        NOW,
        &RotateCfg::default(),
        router(BTreeMap::from([("https://lane.example", answered(200))])),
    )
    .expect("rotate ok");
    assert_eq!(rep.probed, 1);
    assert_eq!(rep.live, 1);
    assert_eq!(rep.cards_appended, 1, "one Live card");
    assert_eq!(rep.alerts.len(), 0);
    assert!(rep.view_synced);
    let reg = fold_view(&home);
    let s = reg.seats.get("prov/m1").expect("seat");
    assert_eq!(s.state, crate::SeatState::Live);
    assert_eq!(s.since_epoch_s, NOW);
}

#[test]
fn class_402_429_401_with_auth() {
    for (url, status, want) in [
        ("https://a.example", 402u16, crate::SeatState::Expired),
        ("https://b.example", 429u16, crate::SeatState::RateLimited),
        ("https://c.example", 401u16, crate::SeatState::Failed),
    ] {
        let home = home_with(&[
            provider("prov", url, "C:/vault/key.path"),
            seat("prov/m1", "prov", crate::SeatState::Probing, 0),
        ]);
        let rep = rotate(
            &home,
            NOW,
            &RotateCfg::default(),
            router(BTreeMap::from([(url, answered(status))])),
        )
        .expect("rotate ok");
        let reg = fold_view(&home);
        let s = reg.seats.get("prov/m1").expect("seat");
        assert_eq!(s.state, want, "status {status}");
        assert_eq!(rep.cards_appended, 1);
    }
}

#[test]
fn class_401_without_auth_is_unprobeable_not_failed() {
    let home = default_home_single("https://lane.example");
    let rep = rotate(
        &home,
        NOW,
        &RotateCfg::default(),
        router(BTreeMap::from([("https://lane.example", answered(401))])),
    )
    .expect("rotate ok");
    assert_eq!(rep.unprobeable, 1);
    assert_eq!(rep.cards_appended, 0, "UNPROBEABLE appends NOTHING");
    assert_eq!(rep.alerts.len(), 0, "no alert before the threshold");
    let reg = fold_view(&home);
    assert_eq!(
        reg.seats.get("prov/m1").unwrap().state,
        crate::SeatState::Probing,
        "stays probing (ReprobeDue lingers honestly)"
    );
    // The streak was booked.
    let st = RotationState::load(&home).expect("state");
    assert_eq!(st.streaks.get("prov/m1"), Some(&1));
}

#[test]
fn class_transient_and_unlisted_append_nothing() {
    for out in [
        refused("network unreachable"),
        answered(408),
        answered(504),
        answered(418),
    ] {
        let home = default_home_single("https://lane.example");
        let rep = rotate(
            &home,
            NOW,
            &RotateCfg::default(),
            router(BTreeMap::from([("https://lane.example", out.clone())])),
        )
        .expect("rotate ok");
        assert_eq!(rep.transient, 1, "out={out:?}");
        assert_eq!(rep.cards_appended, 0);
        assert_eq!(rep.transient_reasons.len(), 1);
    }
}

// ---------------------------------------------------------------------------
// Q6 amendment: ×3 consecutive unprobeable → state + ONE alert
// ---------------------------------------------------------------------------

#[test]
fn unprobeable_times3_flips_state_once() {
    let home = default_home_single("https://lane.example");
    let f = router(BTreeMap::from([("https://lane.example", answered(401))]));
    // Rotations 1 and 2: no card, no alert.
    for i in 1..=2 {
        let rep = rotate(&home, NOW + i * 100, &RotateCfg::default(), |u, p, a, c| {
            f(u, p, a, c)
        })
        .expect("rotate ok");
        assert_eq!(rep.cards_appended, 0, "rotation {i}");
        assert_eq!(rep.alerts.len(), 0);
    }
    // Rotation 3: the transition — one card, ONE alert.
    let rep = rotate(&home, NOW + 300, &RotateCfg::default(), |u, p, a, c| {
        f(u, p, a, c)
    })
    .expect("rotate ok");
    assert_eq!(rep.cards_appended, 1);
    assert_eq!(rep.alerts.len(), 1);
    assert!(rep.alerts[0].contains("prov/m1"));
    let reg = fold_view(&home);
    assert_eq!(
        reg.seats.get("prov/m1").unwrap().state,
        crate::SeatState::Unprobeable
    );
    // Rotation 4: still unprobeable — NO second alert, no duplicate card.
    let rep = rotate(&home, NOW + 10_000, &RotateCfg::default(), |u, p, a, c| {
        f(u, p, a, c)
    })
    .expect("rotate ok");
    assert_eq!(rep.alerts.len(), 0, "alert is per TRANSITION");
    assert_eq!(rep.cards_appended, 0);
}

#[test]
fn streak_resets_on_any_observed_result() {
    let home = default_home_single("https://lane.example");
    let unauth401 = answered(401);
    // Two unprobeable rotations...
    for i in 1..=2 {
        let _ = rotate(&home, NOW + i * 100, &RotateCfg::default(), |u, p, a, c| {
            let _ = (u, p, a, c);
            unauth401.clone()
        })
        .expect("rotate ok");
    }
    // ...a transient break in between...
    let _ = rotate(&home, NOW + 300, &RotateCfg::default(), |u, p, a, c| {
        let _ = (u, p, a, c);
        refused("boom")
    })
    .expect("rotate ok");
    let st = RotationState::load(&home).unwrap();
    assert!(
        !st.streaks.contains_key("prov/m1"),
        "transient breaks the chain"
    );
    // ...then unprobeable again: streak restarts at 1, no flip at "3 total".
    let rep = rotate(&home, NOW + 500, &RotateCfg::default(), |u, p, a, c| {
        let _ = (u, p, a, c);
        unauth401.clone()
    })
    .expect("rotate ok");
    assert_eq!(rep.cards_appended, 0);
    assert_eq!(rep.alerts.len(), 0);
    let st = RotationState::load(&home).unwrap();
    assert_eq!(st.streaks.get("prov/m1"), Some(&1));
}

#[test]
fn auth_landing_lifts_unprobeable_next_rotation() {
    let home = default_home_single("https://lane.example");
    // Drive the seat into unprobeable (threshold via config: 1).
    let cfg1 = RotateCfg {
        unprobeable_after: 1,
        ..RotateCfg::default()
    };
    let rep = rotate(&home, NOW, &cfg1, |u, p, a, c| {
        let _ = (u, p, a, c);
        answered(401)
    })
    .expect("rotate ok");
    assert_eq!(rep.alerts.len(), 1);
    // Next rotation (past the unprobeable cadence) answers 200: the seat
    // lifts to Live automatically.
    let lift_at = NOW + RotateCfg::default().cadence.unprobeable_retry_every_s + 1;
    let rep = rotate(&home, lift_at, &RotateCfg::default(), |u, p, a, c| {
        let _ = (u, p, a, c);
        answered(200)
    })
    .expect("rotate ok");
    assert_eq!(rep.live, 1);
    let reg = fold_view(&home);
    assert_eq!(
        reg.seats.get("prov/m1").unwrap().state,
        crate::SeatState::Live
    );
    let st = RotationState::load(&home).unwrap();
    assert!(!st.streaks.contains_key("prov/m1"));
}

#[test]
fn blank_base_url_never_dials_and_counts_unprobeable() {
    let home = default_home_single("");
    let calls = Arc::new(AtomicU32::new(0));
    let calls2 = calls.clone();
    let cfg = RotateCfg {
        unprobeable_after: 1,
        ..RotateCfg::default()
    };
    let rep = rotate(&home, NOW, &cfg, move |_u, _p, _a, _c| {
        calls2.fetch_add(1, Ordering::SeqCst);
        answered(200)
    })
    .expect("rotate ok");
    assert_eq!(rep.unprobeable, 1);
    assert_eq!(rep.alerts.len(), 1);
    let reg = fold_view(&home);
    assert_eq!(
        reg.seats.get("prov/m1").unwrap().state,
        crate::SeatState::Unprobeable
    );
}

// ---------------------------------------------------------------------------
// Sweep + due + lock + state/config laws
// ---------------------------------------------------------------------------

#[test]
fn sweep_lands_ttl_transitions() {
    // A stale Live seat (last probe 10h ago with hourly cadence) → Expired
    // card from the SWEEP; Expired is NOT due (quota cooldown), so zero probes.
    let home = home_with(&[
        provider("prov", "https://lane.example", ""),
        seat("prov/m1", "prov", crate::SeatState::Live, NOW - 10 * 3600),
    ]);
    let rep = rotate(&home, NOW, &RotateCfg::default(), |_u, _p, _a, _c| {
        panic!("no probe expected")
    })
    .expect("rotate ok");
    assert_eq!(rep.sweep_appended, 1);
    assert_eq!(rep.probed, 0);
    let reg = fold_view(&home);
    assert_eq!(
        reg.seats.get("prov/m1").unwrap().state,
        crate::SeatState::Expired
    );
}

#[test]
fn nothing_due_is_quiet() {
    let home = home_with(&[
        provider("prov", "https://lane.example", ""),
        seat("prov/m1", "prov", crate::SeatState::Live, NOW - 60),
    ]);
    match rotate(&home, NOW, &RotateCfg::default(), |_u, _p, _a, _c| {
        answered(200)
    }) {
        Err(RotateErr::NothingDue(rep)) => {
            assert_eq!(rep.due, 0);
            assert_eq!(rep.cards_appended, 0);
        }
        other => panic!("expected NothingDue, got {other:?}"),
    }
}

#[test]
fn young_lock_is_defect_stale_lock_is_stolen() {
    let home = default_home_single("https://lane.example");
    // Young held lock: defect, nothing runs.
    std::fs::write(
        lock_path(&home),
        format!("{{\"pid\":1,\"started_epoch_s\":{}}}\n", NOW - 10),
    )
    .unwrap();
    match rotate(&home, NOW, &RotateCfg::default(), |_u, _p, _a, _c| {
        answered(200)
    }) {
        Err(RotateErr::Defect(m)) => assert!(m.contains("held"), "{m}"),
        other => panic!("expected Defect, got {other:?}"),
    }
    // Stale lock (older than LOCK_STALE_S): stolen, rotation proceeds.
    std::fs::write(
        lock_path(&home),
        format!(
            "{{\"pid\":1,\"started_epoch_s\":{}}}\n",
            NOW - LOCK_STALE_S - 5
        ),
    )
    .unwrap();
    let rep = rotate(&home, NOW, &RotateCfg::default(), |u, p, a, c| {
        router(BTreeMap::from([("https://lane.example", answered(200))]))(u, p, a, c)
    })
    .expect("rotate ok past stale lock");
    assert!(rep.lock_stolen);
    // Lock released at the end.
    assert!(!lock_path(&home).exists());
}

#[test]
fn malformed_rotation_state_is_defect() {
    let home = default_home_single("https://lane.example");
    std::fs::write(state_path(&home), "{ not json").unwrap();
    match rotate(&home, NOW, &RotateCfg::default(), |_u, _p, _a, _c| {
        answered(200)
    }) {
        Err(RotateErr::Defect(m)) => assert!(m.contains("rotation-state"), "{m}"),
        other => panic!("expected Defect, got {other:?}"),
    }
}

#[test]
fn config_exact_field_law() {
    let home = default_home_single("https://lane.example");
    // Absent: defaults.
    assert_eq!(load_cfg(&home).unwrap(), RotateCfg::default());
    // Full valid override.
    std::fs::write(
        config_path(&home),
        "{\"cadence\":{\"live_probe_every_s\":120},\"probe\":{\"connect_timeout_s\":3,\"total_timeout_s\":7},\"unprobeable_after\":5}",
    )
    .unwrap();
    let cfg = load_cfg(&home).unwrap();
    assert_eq!(cfg.cadence.live_probe_every_s, 120);
    assert_eq!(cfg.probe.connect_timeout, std::time::Duration::from_secs(3));
    assert_eq!(cfg.probe.total_timeout, std::time::Duration::from_secs(7));
    assert_eq!(cfg.unprobeable_after, 5);
    // Unknown field: defect.
    std::fs::write(config_path(&home), "{\"nope\":1}").unwrap();
    assert!(load_cfg(&home).is_err());
    // Cadence unknown field: defect.
    std::fs::write(config_path(&home), "{\"cadence\":{\"nope\":1}}").unwrap();
    assert!(load_cfg(&home).is_err());
}

#[test]
fn map_status_table_unit() {
    use ProbeClass::*;
    assert_eq!(map_status(Some(200), false), Live);
    assert_eq!(map_status(Some(200), true), Live);
    assert_eq!(map_status(Some(402), true), Expired);
    assert_eq!(map_status(Some(429), false), RateLimited);
    assert_eq!(map_status(Some(401), true), Failed);
    assert_eq!(map_status(Some(403), true), Failed);
    assert_eq!(map_status(Some(401), false), Unprobeable);
    assert_eq!(map_status(Some(403), false), Unprobeable);
    assert_eq!(map_status(Some(408), true), Transient);
    assert_eq!(map_status(Some(504), false), Transient);
    assert_eq!(map_status(Some(500), true), Transient);
    assert_eq!(map_status(Some(418), true), Transient);
    assert_eq!(map_status(None, true), Transient);
    assert_eq!(map_status(None, false), Transient);
}

#[test]
fn rotation_log_line_lands() {
    let home = default_home_single("https://lane.example");
    let _ = rotate(&home, NOW, &RotateCfg::default(), |u, p, a, c| {
        router(BTreeMap::from([("https://lane.example", answered(200))]))(u, p, a, c)
    })
    .expect("rotate ok");
    let log = std::fs::read_to_string(log_path(&home)).expect("log exists");
    let v = crate::json::parse(log.trim()).expect("log line is JSON");
    assert_eq!(v.get("verb").and_then(|x| x.as_str()), Some("rotate"));
    assert_eq!(v.get("live").and_then(|x| x.as_f64()), Some(1.0));
}

#[test]
fn probe_path_flows_from_card_to_transport() {
    // The provider card's probe_path override reaches the transport
    // verbatim (base_url unchanged) — the gemini law end-to-end.
    let prov = match provider("prov", "https://lane.example/v1beta/openai", "") {
        Card::Provider(mut pc) => {
            pc.probe_path = "/v1beta/models".into();
            Card::Provider(pc)
        }
        _ => panic!("provider card expected"),
    };
    let home = home_with(&[prov, seat("prov/m1", "prov", crate::SeatState::Probing, 0)]);
    let rep = rotate(&home, NOW, &RotateCfg::default(), |u, p, _a, _c| {
        assert_eq!(
            u, "https://lane.example/v1beta/openai",
            "base_url unchanged"
        );
        assert_eq!(p, "/v1beta/models", "override flows through");
        answered(200)
    })
    .expect("rotate ok");
    assert_eq!(rep.live, 1, "override-probed seat lifts Live");
}
