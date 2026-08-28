//! council_tests.rs — P2 slice 1 tests. Plan P2 Done-When slice: the
//! council card validates mechanically; pin-mismatch and
//! mid-flight-edit→re-dispatch paths proven on fixtures.

use crate::council::*;
use crate::protocol::{Protocol, ProtocolKind};
use crate::registry::{ProviderCard, Registry, SeatCard};
use crate::{CostClass, Floors, LaneType, Seat, SeatState};

// --- fixtures ---------------------------------------------------------------

fn seat(id: &str, family: &str, cost: CostClass, state: SeatState) -> Seat {
    Seat {
        lane_id: id.to_string(),
        lane_type: LaneType::Http,
        family: family.to_string(),
        provider: family.to_string(),
        model: format!("{family}-model"),
        cost_class: cost,
        state,
        caps: 1,
        last_probe: Some(std::time::SystemTime::UNIX_EPOCH),
    }
}

/// Four live seats across four families — the happy council day.
fn happy_candidates() -> Vec<Seat> {
    vec![
        seat("groq/llama", "groq", CostClass::Free, SeatState::Live),
        seat("openai/x", "openai", CostClass::Free, SeatState::Live),
        seat("zai/glm", "zai", CostClass::Free, SeatState::Live),
        seat("nvidia/nem", "nvidia", CostClass::Free, SeatState::Live),
    ]
}

fn card(version: u32, floors: Floors) -> Protocol {
    Protocol {
        version,
        kind: ProtocolKind::Council,
        stages: COUNCIL_STAGES.iter().map(|s| s.to_string()).collect(),
        floors,
    }
}

// Warden ledger fixtures — rows built through the warden's OWN body law
// (edits_tests precedent, never a hand-copied format).
const ACTOR: &str = "terminal.ashpac";

fn warden_row(typ: &str, from: &str, body_text: &str) -> String {
    format!(
        "{{\"seq\":1,\"v\":1,\"id\":\"x\",\"idem_key\":\"k\",\"type\":\"{typ}\",\
         \"from\":\"{from}\",\"to\":\"warden\",\"body\":\"{body_text}\",\"ts\":\"1\"}}\n"
    )
}

fn warden_open(from: &str, card_id: &str) -> String {
    warden_row(
        "card.open",
        from,
        &caddis_warden::card_state::body("open", card_id, "_card_x.md", "deadbeef"),
    )
}

fn gate_open() -> String {
    warden_open(ACTOR, "CARD-0007")
}

fn convened_under_v1() -> CouncilSession {
    convene(
        "c1",
        "rule on the organ plan",
        Stakes::Medium,
        &protocol_v1(),
        &happy_candidates(),
        ACTOR,
        &gate_open(),
    )
    .unwrap()
}

fn registry_with(providers: &[(&str, u32)], seats: &[(&str, &str)]) -> Registry {
    Registry {
        providers: providers
            .iter()
            .map(|(id, caps)| {
                (
                    id.to_string(),
                    ProviderCard {
                        id: id.to_string(),
                        lane_type: LaneType::Http,
                        base_url: String::new(),
                        auth_path: String::new(),
                        caps: *caps,
                        source: "test".into(),
                    },
                )
            })
            .collect(),
        seats: seats
            .iter()
            .map(|(id, provider)| {
                (
                    id.to_string(),
                    SeatCard {
                        id: id.to_string(),
                        provider: provider.to_string(),
                        family: provider.to_string(),
                        model: id.to_string(),
                        lane_type: LaneType::Http,
                        cost_class: CostClass::Free,
                        state: SeatState::Live,
                        since_epoch_s: 0,
                        caps: 1,
                        cost_in_usd_per_mtok: 0.0,
                        cost_out_usd_per_mtok: 0.0,
                        context_window: 0,
                        max_tokens: 0,
                        source: "test".into(),
                    },
                )
            })
            .collect(),
    }
}

fn reply(lane: &str, position: Position) -> Reply {
    Reply {
        lane_id: lane.to_string(),
        transport_served_model: format!("{lane}-served-model"),
        position,
    }
}

// --- the card ---------------------------------------------------------------

#[test]
fn card_v1_is_the_seven_stage_council_card() {
    let p = protocol_v1();
    assert_eq!(p.version, 1);
    assert_eq!(p.kind, ProtocolKind::Council);
    assert_eq!(p.stages, COUNCIL_STAGES);
    assert_eq!(p.floors, Floors::default());
    // Floors default = ROLE ladder size, families 2, non-Chinese 1 (priors).
    assert_eq!(p.floors.panel_size, crate::ROLE_ORDER.len());
    assert_eq!(p.floors.min_families, 2);
    assert_eq!(p.floors.min_non_chinese, 1);
    validate_card(&p).unwrap();
    let pin = p.pin();
    assert_eq!(pin.len(), 64);
    assert!(pin.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')));
    // Stable: same card, same pin (deterministic canonical bytes).
    assert_eq!(protocol_v1().pin(), pin);
}

#[test]
fn validate_refuses_wrong_kind() {
    let mut p = protocol_v1();
    p.kind = ProtocolKind::Quorum;
    let err = validate_card(&p).unwrap_err();
    assert!(matches!(err, CouncilErr::CardInvalid(_)));
    assert!(err.is_refusal());
}

#[test]
fn validate_refuses_stage_drift() {
    let mut missing = protocol_v1();
    missing.stages.remove(2);
    assert!(matches!(
        validate_card(&missing).unwrap_err(),
        CouncilErr::CardInvalid(_)
    ));

    let mut extra = protocol_v1();
    extra.stages.push("improvise".into());
    assert!(matches!(
        validate_card(&extra).unwrap_err(),
        CouncilErr::CardInvalid(_)
    ));

    let mut swapped = protocol_v1();
    swapped.stages.swap(0, 1);
    assert!(matches!(
        validate_card(&swapped).unwrap_err(),
        CouncilErr::CardInvalid(_)
    ));
}

#[test]
fn validate_refuses_version_zero_and_incoherent_floors() {
    assert!(matches!(
        validate_card(&card(0, Floors::default())).unwrap_err(),
        CouncilErr::CardInvalid(_)
    ));
    let beyond_panel = Floors {
        min_families: Floors::default().panel_size + 1,
        ..Floors::default()
    };
    assert!(matches!(
        validate_card(&card(1, beyond_panel)).unwrap_err(),
        CouncilErr::CardInvalid(_)
    ));
    let zero_panel = Floors {
        panel_size: 0,
        ..Floors::default()
    };
    assert!(matches!(
        validate_card(&card(1, zero_panel)).unwrap_err(),
        CouncilErr::CardInvalid(_)
    ));
}

// --- F1 gate + convene ------------------------------------------------------

#[test]
fn convene_refuses_when_the_gate_is_closed() {
    let err = convene(
        "c1",
        "task",
        Stakes::Medium,
        &protocol_v1(),
        &happy_candidates(),
        ACTOR,
        "",
    )
    .unwrap_err();
    assert!(matches!(err, CouncilErr::GateClosed { .. }));
    assert!(err.is_refusal());
}

#[test]
fn convene_defects_on_an_unreadable_warden_ledger() {
    let torn = format!("{}{{\"type\":\"card.open\" broken\n", gate_open());
    let err = convene(
        "c1",
        "task",
        Stakes::Medium,
        &protocol_v1(),
        &happy_candidates(),
        ACTOR,
        &torn,
    )
    .unwrap_err();
    // Unreadable rows can never look like "closed" — fail-closed Defect.
    assert!(matches!(err, CouncilErr::Defect(_)));
    assert!(!err.is_refusal());
}

#[test]
fn convene_pins_the_protocol_and_records_the_gate() {
    let s = convened_under_v1();
    assert_eq!(s.convening.pinned_protocol, protocol_v1().pin());
    assert_eq!(s.convening.id, "c1");
    assert_eq!(s.stakes, Stakes::Medium);
    assert_eq!(s.rerun_of, None);
    assert_eq!(s.gate.actor, ACTOR);
    assert_eq!(s.gate.warden_card, "CARD-0007");
    // Panel seated at floor size with roles in ladder order.
    assert_eq!(
        s.convening.panel.seats.len(),
        protocol_v1().floors.panel_size
    );
    assert_eq!(s.convening.panel.seats[0].role, crate::Role::Chair);
}

#[test]
fn convene_ignores_cards_for_other_callers() {
    // A card opened by a DIFFERENT actor never gates this convening.
    let other = warden_open("terminal.someone-else", "CARD-0008");
    let err = convene(
        "c1",
        "task",
        Stakes::Small,
        &protocol_v1(),
        &happy_candidates(),
        ACTOR,
        &other,
    )
    .unwrap_err();
    assert!(matches!(err, CouncilErr::GateClosed { .. }));
}

#[test]
fn convene_refuses_a_degraded_day() {
    // All seats one family — monoculture floor unsatisfiable → honest
    // refusal, never a degraded panel.
    let monoculture = vec![
        seat("zai/a", "zai", CostClass::Free, SeatState::Live),
        seat("zai/b", "zai", CostClass::Free, SeatState::Live),
        seat("zai/c", "zai", CostClass::Free, SeatState::Live),
        seat("zai/d", "zai", CostClass::Free, SeatState::Live),
    ];
    let err = convene(
        "c1",
        "task",
        Stakes::Medium,
        &protocol_v1(),
        &monoculture,
        ACTOR,
        &gate_open(),
    )
    .unwrap_err();
    assert!(matches!(err, CouncilErr::Panel(_)));
    assert!(err.is_refusal());
}

#[test]
fn convene_skips_non_live_seats() {
    let mut candidates = happy_candidates();
    candidates[0].state = SeatState::Expired; // cheapest lane lapses (F10)
    let err = convene(
        "c1",
        "task",
        Stakes::Medium,
        &protocol_v1(),
        &candidates,
        ACTOR,
        &gate_open(),
    )
    .unwrap_err();
    // Only 3 live remain for a panel of 4 → refusal, never a short panel.
    assert!(matches!(err, CouncilErr::Panel(_)));
    assert!(err.is_refusal());
}

// --- dispatch plan ----------------------------------------------------------

#[test]
fn dispatch_plan_serializes_a_capped_provider_into_waves() {
    let s = convened_under_v1();
    // Four seats, four providers, caps 1 — one wave; panel order is the
    // free-first selection order (ties by lane_id).
    let reg = registry_with(
        &[("groq", 1), ("openai", 1), ("zai", 1), ("nvidia", 1)],
        &[
            ("groq/llama", "groq"),
            ("openai/x", "openai"),
            ("zai/glm", "zai"),
            ("nvidia/nem", "nvidia"),
        ],
    );
    assert_eq!(
        dispatch_plan(&s, &reg).unwrap(),
        vec![vec![
            "groq/llama".to_string(),
            "nvidia/nem".to_string(),
            "openai/x".to_string(),
            "zai/glm".to_string(),
        ]]
    );

    // Two seats SHARING capped provider "ollama" (Ruling 7: 1 concurrent)
    // never share a wave; the others proceed. Families stay distinct
    // (ollama / ollama-b) so the floors hold; the PROVIDER is the cap
    // domain, not the family.
    let mut oa = seat("ollama/a", "ollama", CostClass::Free, SeatState::Live);
    let mut ob = seat("ollama/b", "ollama-b", CostClass::Free, SeatState::Live);
    oa.provider = "ollama".into();
    ob.provider = "ollama".into();
    let shared = vec![
        seat("groq/llama", "groq", CostClass::Free, SeatState::Live),
        oa,
        ob,
        seat("openai/x", "openai", CostClass::Free, SeatState::Live),
    ];
    let s2 = convene(
        "c2",
        "task",
        Stakes::Medium,
        &protocol_v1(),
        &shared,
        ACTOR,
        &gate_open(),
    )
    .unwrap();
    let reg2 = registry_with(
        &[("groq", 1), ("openai", 1), ("ollama", 1)],
        &[
            ("groq/llama", "groq"),
            ("openai/x", "openai"),
            ("ollama/a", "ollama"),
            ("ollama/b", "ollama"),
        ],
    );
    // Panel order: groq/llama, ollama/a, ollama/b, openai/x. The planner
    // is greedy over dispatch order (P1 law): when ollama/b finds its
    // provider at cap the wave CLOSES — ollama/b opens the next wave and
    // openai/x joins THAT one. The Ruling-7 invariant holds: the two
    // ollama seats never share a wave.
    assert_eq!(
        dispatch_plan(&s2, &reg2).unwrap(),
        vec![
            vec!["groq/llama".to_string(), "ollama/a".to_string()],
            vec!["ollama/b".to_string(), "openai/x".to_string()],
        ]
    );
}

// --- collect / integrate / verdict ------------------------------------------

#[test]
fn collect_orders_the_bundle_in_panel_order() {
    let s = convened_under_v1();
    let shuffled = vec![
        reply("zai/glm", Position::Ship),
        reply("nvidia/nem", Position::DoNotShip),
        reply("groq/llama", Position::Ship),
        reply("openai/x", Position::ShipWithChanges),
    ];
    let b = collect(&s.convening, &shuffled).unwrap();
    let lanes: Vec<&str> = b.replies.iter().map(|r| r.lane_id.as_str()).collect();
    let panel_lanes: Vec<&str> = s
        .convening
        .panel
        .seats
        .iter()
        .map(|ps| ps.seat.lane_id.as_str())
        .collect();
    assert_eq!(lanes, panel_lanes);
}

#[test]
fn collect_refuses_an_incomplete_bundle() {
    let s = convened_under_v1();
    let partial = vec![
        reply("groq/llama", Position::Ship),
        reply("openai/x", Position::Ship),
        reply("zai/glm", Position::Ship),
    ];
    let err = collect(&s.convening, &partial).unwrap_err();
    match err {
        CouncilErr::CollectIncomplete { ref missing } => assert_eq!(missing, &["nvidia/nem"]),
        other => panic!("expected CollectIncomplete, got {other:?}"),
    }
    assert!(err.is_refusal());
}

#[test]
fn collect_defects_on_crossing_duplicate_and_blank_provenance() {
    let s = convened_under_v1();
    // Lane outside the panel — identity crossing.
    let crossing = vec![
        reply("groq/llama", Position::Ship),
        reply("openai/x", Position::Ship),
        reply("zai/glm", Position::Ship),
        reply("mystery/lane", Position::Ship),
    ];
    assert!(matches!(
        collect(&s.convening, &crossing).unwrap_err(),
        CouncilErr::Defect(_)
    ));
    // Duplicate reply from one seat.
    let dup = vec![
        reply("groq/llama", Position::Ship),
        reply("groq/llama", Position::DoNotShip),
        reply("zai/glm", Position::Ship),
        reply("nvidia/nem", Position::Ship),
    ];
    assert!(matches!(
        collect(&s.convening, &dup).unwrap_err(),
        CouncilErr::Defect(_)
    ));
    // Blank transport-served model — provenance has no empty form.
    let mut blank = reply("groq/llama", Position::Ship);
    blank.transport_served_model = String::new();
    assert!(matches!(
        collect(&s.convening, &[blank]).unwrap_err(),
        CouncilErr::Defect(_)
    ));
}

#[test]
fn integrate_maps_disagreement_never_averages() {
    let s = convened_under_v1();
    let b = collect(
        &s.convening,
        &vec![
            reply("groq/llama", Position::Ship),
            reply("openai/x", Position::Ship),
            reply("zai/glm", Position::ShipWithChanges),
            reply("nvidia/nem", Position::DoNotShip),
        ],
    )
    .unwrap();
    let m = integrate(&b);
    // Fixed cluster order, all three positions present, sorted lanes.
    let order: Vec<&str> = m.clusters.iter().map(|c| c.position.as_str()).collect();
    assert_eq!(order, ["ship", "ship_with_changes", "do_not_ship"]);
    assert_eq!(m.holding(Position::Ship), ["groq/llama", "openai/x"]);
    assert_eq!(m.holding(Position::ShipWithChanges), ["zai/glm"]);
    assert_eq!(m.holding(Position::DoNotShip), ["nvidia/nem"]);
    assert!(m.disagrees());
    assert_eq!(m.summary(), "ship=2,ship_with_changes=1,do_not_ship=1");

    // Unanimous panel: no disagreement, table still carries all rows.
    let u = collect(
        &s.convening,
        &vec![
            reply("groq/llama", Position::Ship),
            reply("openai/x", Position::Ship),
            reply("zai/glm", Position::Ship),
            reply("nvidia/nem", Position::Ship),
        ],
    )
    .unwrap();
    let mu = integrate(&u);
    assert!(!mu.disagrees());
    assert_eq!(mu.summary(), "ship=4,ship_with_changes=0,do_not_ship=0");
}

#[test]
fn verdict_carries_transport_provenance_and_the_table() {
    let s = convened_under_v1();
    let b = collect(
        &s.convening,
        &vec![
            reply("groq/llama", Position::Ship),
            reply("openai/x", Position::ShipWithChanges),
            reply("zai/glm", Position::Ship),
            reply("nvidia/nem", Position::Ship),
        ],
    )
    .unwrap();
    let m = integrate(&b);
    let v = verdict(&s, &b, &m);
    assert_eq!(v.convening_id, "c1");
    assert_eq!(v.ruling, "ship=3,ship_with_changes=1,do_not_ship=0");
    // Council never lands degraded: partial bundles are refused upstream.
    assert!(!v.degraded);
    let mut lanes: Vec<&str> = v.provenance.iter().map(|p| p.lane_id.as_str()).collect();
    lanes.sort();
    assert_eq!(lanes, ["groq/llama", "nvidia/nem", "openai/x", "zai/glm"]);
    for p in &v.provenance {
        assert_eq!(
            p.transport_served_model,
            format!("{}-served-model", p.lane_id)
        );
    }
}

// --- ledger row -------------------------------------------------------------

#[test]
fn ledger_row_round_trips_through_the_one_parser() {
    let s = convened_under_v1();
    let b = collect(
        &s.convening,
        &vec![
            reply("groq/llama", Position::Ship),
            reply("openai/x", Position::ShipWithChanges),
            reply("zai/glm", Position::Ship),
            reply("nvidia/nem", Position::Ship),
        ],
    )
    .unwrap();
    let m = integrate(&b);
    let v = verdict(&s, &b, &m);
    let line = ledger_row(&s, &v, &m);
    let row = parse_ledger_row(&line).unwrap();
    assert_eq!(row.conv, "c1");
    assert_eq!(row.pin, s.convening.pinned_protocol);
    assert_eq!(row.stakes, "medium");
    assert_eq!(row.rerun_of, "");
    assert_eq!(row.actor, ACTOR);
    assert_eq!(row.warden_card, "CARD-0007");
    assert_eq!(row.ship, 3);
    assert_eq!(row.ship_with_changes, 1);
    assert_eq!(row.do_not_ship, 0);
    // The digest is tamper-evidence for THIS verdict's canonical bytes.
    assert_eq!(
        row.verdict_digest,
        crate::sha256::hex(canonical_verdict_bytes(&v, &m).as_bytes())
    );
}

#[test]
fn parse_ledger_row_refuses_drift() {
    let s = convened_under_v1();
    let b = collect(
        &s.convening,
        &vec![
            reply("groq/llama", Position::Ship),
            reply("openai/x", Position::Ship),
            reply("zai/glm", Position::Ship),
            reply("nvidia/nem", Position::Ship),
        ],
    )
    .unwrap();
    let m = integrate(&b);
    let v = verdict(&s, &b, &m);
    let line = ledger_row(&s, &v, &m);
    // Unknown extra field — exact-field law.
    let mut tampered = line.clone();
    tampered.insert_str(line.len() - 1, ",\"oops\":1");
    assert!(matches!(
        parse_ledger_row(&tampered).unwrap_err(),
        CouncilErr::Defect(_)
    ));
    // Truncated digest.
    let short = line.replace(&row_digest(&line), &"ab".repeat(31));
    assert!(matches!(
        parse_ledger_row(&short).unwrap_err(),
        CouncilErr::Defect(_)
    ));
}

/// Pull the verdict_digest value back out of an encoded row (test helper).
fn row_digest(line: &str) -> String {
    let v = crate::json::parse(line).unwrap();
    v.get("verdict_digest")
        .and_then(|x| x.as_str())
        .unwrap()
        .to_string()
}

// --- F3 check + F11 re-dispatch ---------------------------------------------

#[test]
fn check_pin_detects_a_moved_protocol() {
    let s = convened_under_v1();
    assert_eq!(check_pin(&s, &protocol_v1()).unwrap(), PinOutcome::Intact);
    let v2 = card(
        2,
        Floors {
            panel_size: 3,
            min_families: 2,
            min_non_chinese: 1,
        },
    );
    match check_pin(&s, &v2).unwrap() {
        PinOutcome::Moved(m) => {
            assert_eq!(m.pinned, s.convening.pinned_protocol);
            assert_eq!(m.actual, v2.pin());
        }
        PinOutcome::Intact => panic!("v2 must not verify against a v1 pin"),
    }
}

#[test]
fn re_dispatch_archives_the_original_and_re_runs_under_the_new_card() {
    let s = convened_under_v1();
    let v2 = card(
        2,
        Floors {
            panel_size: 3,
            min_families: 2,
            min_non_chinese: 1,
        },
    );
    let out = pause_and_re_dispatch(
        s,
        &protocol_v1(),
        &v2,
        &happy_candidates(),
        ACTOR,
        &gate_open(),
    )
    .unwrap();
    // Archived original: unchanged id, still pinned to v1.
    assert_eq!(out.archived.convening.id, "c1");
    assert_eq!(out.archived.convening.pinned_protocol, protocol_v1().pin());
    assert_eq!(out.archived.rerun_of, None);
    // Re-run: new id, new pin, re-run flag pointing at the original.
    assert_eq!(out.re_dispatched.convening.id, "c1#r2");
    assert_eq!(out.re_dispatched.convening.pinned_protocol, v2.pin());
    assert_eq!(out.re_dispatched.rerun_of, Some("c1".to_string()));
    assert_eq!(out.re_dispatched.gate.warden_card, "CARD-0007");
    // Panel re-constructed under the NEW floors.
    assert_eq!(out.re_dispatched.convening.panel.seats.len(), 3);
}

#[test]
fn re_dispatch_refuses_unbumped_and_mismatched_cards() {
    let s = convened_under_v1();
    // Same version, different floors — the bump is MANDATORY.
    let unbumped = card(
        1,
        Floors {
            panel_size: 3,
            min_families: 2,
            min_non_chinese: 1,
        },
    );
    let err = pause_and_re_dispatch(
        s.clone(),
        &protocol_v1(),
        &unbumped,
        &happy_candidates(),
        ACTOR,
        &gate_open(),
    )
    .unwrap_err();
    assert!(matches!(
        err,
        CouncilErr::VersionNotBumped {
            have: 1,
            try_use: 1
        }
    ));
    assert!(err.is_refusal());
    // Lower version than the archived card — same law. (Version 0 never
    // reaches the bump check: cards validate at >= 1 first. So the
    // reachable form archives under v3 and attempts v2.)
    let s3 = convene(
        "c3",
        "task",
        Stakes::Medium,
        &card(3, Floors::default()),
        &happy_candidates(),
        ACTOR,
        &gate_open(),
    )
    .unwrap();
    assert!(matches!(
        pause_and_re_dispatch(
            s3,
            &card(3, Floors::default()),
            &card(2, Floors::default()),
            &happy_candidates(),
            ACTOR,
            &gate_open()
        )
        .unwrap_err(),
        CouncilErr::VersionNotBumped {
            have: 3,
            try_use: 2
        }
    ));
    // Wrong archived card (pin mismatch with the session) — Defect.
    let not_the_card = card(
        9,
        Floors {
            panel_size: 3,
            min_families: 2,
            min_non_chinese: 1,
        },
    );
    assert!(matches!(
        pause_and_re_dispatch(
            s,
            &not_the_card,
            &card(10, Floors::default()),
            &happy_candidates(),
            ACTOR,
            &gate_open()
        )
        .unwrap_err(),
        CouncilErr::Defect(_)
    ));
}

#[test]
fn re_dispatch_rechecks_the_gate() {
    let s = convened_under_v1();
    let v2 = card(
        2,
        Floors {
            panel_size: 3,
            min_families: 2,
            min_non_chinese: 1,
        },
    );
    let err =
        pause_and_re_dispatch(s, &protocol_v1(), &v2, &happy_candidates(), ACTOR, "").unwrap_err();
    // EVERY convening is F1-gated, the re-dispatch included.
    assert!(matches!(err, CouncilErr::GateClosed { .. }));
}

#[test]
fn end_to_end_full_pipeline_lands_one_row() {
    // The seven stages, one pass: convene → panel → dispatch(plan) →
    // collect → integrate → verdict → ledger row.
    let s = convened_under_v1();
    let reg = registry_with(
        &[("groq", 1), ("openai", 1), ("zai", 1), ("nvidia", 1)],
        &[
            ("groq/llama", "groq"),
            ("openai/x", "openai"),
            ("zai/glm", "zai"),
            ("nvidia/nem", "nvidia"),
        ],
    );
    let plan = dispatch_plan(&s, &reg).unwrap();
    assert_eq!(plan.len(), 1);
    let b = collect(
        &s.convening,
        &vec![
            reply("groq/llama", Position::ShipWithChanges),
            reply("openai/x", Position::ShipWithChanges),
            reply("zai/glm", Position::Ship),
            reply("nvidia/nem", Position::ShipWithChanges),
        ],
    )
    .unwrap();
    let m = integrate(&b);
    assert!(m.disagrees());
    let v = verdict(&s, &b, &m);
    let row = parse_ledger_row(&ledger_row(&s, &v, &m)).unwrap();
    assert_eq!(row.ship, 1);
    assert_eq!(row.ship_with_changes, 3);
    assert_eq!(row.do_not_ship, 0);
}
