//! caps_tests.rs — P1 slice 2 gates: the Ruling-7 cap law and the pure
//! dispatch planner (Done-When: a capped provider SERIALIZES in dispatch
//! order).

use super::*;
use crate::registry::{Card, ProviderCard, Registry, SeatCard};

fn prov(id: &str, caps: u32) -> Card {
    Card::Provider(ProviderCard {
        id: id.into(),
        lane_type: crate::LaneType::Http,
        base_url: format!("https://{id}.example/v1"),
        auth_path: String::new(),
        caps,
        source: "models.json#deadbeef".into(),
    })
}

fn seat(id: &str, caps: u32) -> Card {
    let provider = id.split('/').next().unwrap();
    Card::Seat(SeatCard {
        id: id.into(),
        provider: provider.into(),
        family: provider.into(),
        model: id.rsplit('/').next().unwrap().into(),
        lane_type: crate::LaneType::Http,
        cost_class: crate::CostClass::Free,
        state: crate::SeatState::Live,
        since_epoch_s: 0,
        caps,
        cost_in_usd_per_mtok: 0.0,
        cost_out_usd_per_mtok: 0.0,
        context_window: 128_000,
        max_tokens: 16_384,
        source: "models.json#deadbeef".into(),
    })
}

// --- the Ruling-7 DATA table.

#[test]
fn ruled_caps_table_and_f4_default() {
    assert_eq!(ruled_caps("ollama"), 1);
    assert_eq!(ruled_caps("ollama-cloud"), 1);
    assert_eq!(ruled_caps("zai-coding"), DEFAULT_CAPS);
    assert_eq!(hard_ceiling("ollama"), Some(2));
    assert_eq!(hard_ceiling("ollama-cloud"), Some(2));
    assert_eq!(hard_ceiling("zai-coding"), None);
    // The table itself carries the ruling exactly as transcribed.
    assert!(RULED_CAPS.contains(&("ollama", 1, 2)));
    assert!(RULED_CAPS.contains(&("ollama-cloud", 1, 2)));
}

#[test]
fn cap_law_accepts_and_refuses() {
    // ollama: 1 ruled, 2 = the ceiling (a ruling may raise TO it), 3+ killed.
    assert!(check_provider_caps("ollama", 1).is_ok());
    assert!(check_provider_caps("ollama", 2).is_ok());
    assert_eq!(
        check_provider_caps("ollama", 3),
        Err(CapsErr::AboveHardCeiling {
            provider: "ollama".into(),
            caps: 3,
            ceiling: 2
        })
    );
    // caps == 0 is broken everywhere: a 0-cap provider can never dispatch.
    assert_eq!(
        check_provider_caps("ollama", 0),
        Err(CapsErr::ZeroCaps {
            provider: "ollama".into()
        })
    );
    assert_eq!(
        check_provider_caps("zai-coding", 0),
        Err(CapsErr::ZeroCaps {
            provider: "zai-coding".into()
        })
    );
    // No named ceiling for other providers: only the >= 1 law.
    assert!(check_provider_caps("zai-coding", 5).is_ok());
}

// --- registry validation: laws over the folded stream.

#[test]
fn validate_registry_catches_all_three_drifts() {
    // Provider above its hard ceiling.
    let reg = Registry::fold(&[prov("ollama", 3)]);
    assert_eq!(
        crate::caps::validate_registry(&reg),
        Err(CapsErr::AboveHardCeiling {
            provider: "ollama".into(),
            caps: 3,
            ceiling: 2
        })
    );

    // Seat caps above its provider caps (drift, reported loudly).
    let reg = Registry::fold(&[prov("zai-coding", 1), seat("zai-coding/glm", 2)]);
    assert_eq!(
        crate::caps::validate_registry(&reg),
        Err(CapsErr::SeatAboveProvider {
            seat_id: "zai-coding/glm".into(),
            seat_caps: 2,
            provider: "zai-coding".into(),
            provider_caps: 1,
        })
    );

    // Seat with no provider card: the planner never guesses a provider.
    let reg = Registry::fold(&[seat("ghost/model", 1)]);
    assert_eq!(
        crate::caps::validate_registry(&reg),
        Err(CapsErr::SeatMissingProvider {
            seat_id: "ghost/model".into(),
            provider: "ghost".into()
        })
    );

    // A clean registry passes.
    let reg = Registry::fold(&[
        prov("ollama", 1),
        prov("zai-coding", 1),
        seat("ollama/qwen", 1),
        seat("zai-coding/glm", 1),
    ]);
    assert!(crate::caps::validate_registry(&reg).is_ok());
}

#[test]
fn effective_caps_is_the_min() {
    let p1 = match prov("ollama", 1) {
        Card::Provider(p) => p,
        _ => unreachable!(),
    };
    let p2 = match prov("zai-coding", 2) {
        Card::Provider(p) => p,
        _ => unreachable!(),
    };
    let s1 = match seat("ollama/a", 2) {
        Card::Seat(s) => s,
        _ => unreachable!(),
    };
    let s2 = match seat("zai-coding/b", 1) {
        Card::Seat(s) => s,
        _ => unreachable!(),
    };
    assert_eq!(effective_caps(&s1, &p1), 1); // seat 2 above provider 1 -> 1
    assert_eq!(effective_caps(&s2, &p2), 1); // seat 1 under provider 2 -> 1
}

// --- the planner: the P1 Done-When proof.

/// THE Done-When: "a capped provider serializes in dispatch order."
/// Two seats of a caps-1 provider never share a wave; other providers'
/// requests proceed in parallel; input order is preserved across waves.
#[test]
fn capped_provider_serializes_in_dispatch_order() {
    let reg = Registry::fold(&[
        prov("ollama", 1),
        prov("zai-coding", 1),
        seat("ollama/a", 1),
        seat("ollama/b", 1),
        seat("zai-coding/c", 1),
    ]);
    let waves = plan_batches(&["ollama/a", "zai-coding/c", "ollama/b"], &reg).unwrap();

    // No wave holds both ollama seats.
    for wave in &waves {
        let ollama_in_wave = wave.iter().filter(|id| id.starts_with("ollama/")).count();
        assert!(
            ollama_in_wave <= 1,
            "wave {wave:?} has {ollama_in_wave} ollama seats"
        );
    }
    // The zai seat runs alongside the FIRST ollama seat (parallel).
    assert_eq!(
        waves.len(),
        2,
        "two waves for three requests under a caps-1 provider"
    );
    assert!(waves[0].contains(&"ollama/a".to_string()));
    assert!(waves[0].contains(&"zai-coding/c".to_string()));
    // Dispatch order preserved: ollama/a strictly before ollama/b.
    let pos = |id: &str| {
        waves
            .iter()
            .position(|w| w.iter().any(|x| x == id))
            .unwrap()
    };
    assert!(pos("ollama/a") < pos("ollama/b"));
}

#[test]
fn raised_to_ceiling_two_share_a_wave() {
    // ollama at its hard ceiling 2 (a legal ruling): two seats may run
    // concurrently — the ceiling is a ceiling, not a jail.
    let reg = Registry::fold(&[prov("ollama", 2), seat("ollama/a", 2), seat("ollama/b", 2)]);
    let waves = plan_batches(&["ollama/a", "ollama/b"], &reg).unwrap();
    assert_eq!(waves.len(), 1, "caps 2 admits both in one wave: {waves:?}");
}

#[test]
fn planner_is_fail_closed() {
    let reg = Registry::fold(&[prov("ollama", 1), seat("ollama/a", 1)]);
    // Unknown seat: refusal, never a guess.
    assert_eq!(
        plan_batches(&["ollama/ghost"], &reg),
        Err(CapsErr::UnknownSeat {
            seat_id: "ollama/ghost".into()
        })
    );
    // Seat whose provider card is missing.
    let reg = Registry::fold(&[seat("ghost/model", 1)]);
    assert_eq!(
        plan_batches(&["ghost/model"], &reg),
        Err(CapsErr::SeatMissingProvider {
            seat_id: "ghost/model".into(),
            provider: "ghost".into()
        })
    );
}

#[test]
fn planner_is_deterministic() {
    let reg = Registry::fold(&[
        prov("ollama", 1),
        prov("zai-coding", 1),
        seat("ollama/a", 1),
        seat("ollama/b", 1),
        seat("zai-coding/c", 1),
    ]);
    let wanted: &[&str] = &["ollama/a", "ollama/b", "zai-coding/c", "ollama/a"];
    let first = plan_batches(wanted, &reg).unwrap();
    let second = plan_batches(wanted, &reg).unwrap();
    assert_eq!(
        first, second,
        "same input, same waves (deterministic replay)"
    );
    // Even a REPEAT request serializes against itself (usage counts).
    for wave in &first {
        let n = wave.iter().filter(|id| **id == "ollama/a").count();
        assert!(n <= 1);
    }
}
