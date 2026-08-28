//! quorum_tests.rs — P2 slice 2 tests. Plan P2 Done-When slice: the
//! quorum card validates mechanically; the F9 disjoint pool, floor 2/3,
//! degradation asterisk, fail-closed paths, VERDICT.md artifact, ledger
//! row round-trip, and the F11 re-dispatch are proven on fixtures.

use crate::council::{self, Stakes};
use crate::protocol::{Protocol, ProtocolKind};
use crate::quorum::*;
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

/// Eight live seats: four FREE (the council panel takes these — four
/// families, floors 4/2/1 satisfied) plus four disjoint candidates whose
/// selection order (free-first, lane_id ties) is
/// anthropic/claude < cohere/c < gemini/g < mistral/m — the pool is the
/// first three.
fn candidates() -> Vec<Seat> {
    vec![
        seat("groq/llama", "groq", CostClass::Free, SeatState::Live),
        seat("openai/x", "openai", CostClass::Free, SeatState::Live),
        seat("zai/glm", "zai", CostClass::Free, SeatState::Live),
        seat("nvidia/nem", "nvidia", CostClass::Free, SeatState::Live),
        seat(
            "anthropic/claude",
            "anthropic",
            CostClass::Free,
            SeatState::Live,
        ),
        seat("cohere/c", "cohere", CostClass::Free, SeatState::Live),
        seat("gemini/g", "gemini", CostClass::Free, SeatState::Live),
        seat("mistral/m", "mistral", CostClass::Free, SeatState::Live),
    ]
}

fn card(version: u32, floors: Floors) -> Protocol {
    Protocol {
        version,
        kind: ProtocolKind::Quorum,
        stages: QUORUM_STAGES.iter().map(|s| s.to_string()).collect(),
        floors,
    }
}

// Warden ledger fixtures — rows built through the warden's OWN body law
// (council_tests precedent, never a hand-copied format).
const ACTOR: &str = "terminal.ashpac";

fn warden_row(typ: &str, from: &str, body_text: &str) -> String {
    format!(
        "{{\"seq\":1,\"v\":1,\"id\":\"x\",\"idem_key\":\"k\",\"type\":\"{typ}\",\
         \"from\":\"{from}\",\"to\":\"warden\",\"body\":\"{body_text}\",\"ts\":\"1\"}}\n"
    )
}

fn gate_open() -> String {
    warden_row(
        "card.open",
        ACTOR,
        &caddis_warden::card_state::body("open", "CARD-0011", "_card_y.md", "feedface"),
    )
}

fn council_convened() -> council::CouncilSession {
    council::convene(
        "c1",
        "rule on the staff-system brief",
        Stakes::Complex,
        &council::protocol_v1(),
        &candidates(),
        ACTOR,
        &gate_open(),
    )
    .unwrap()
}

fn convened_under_v1() -> QuorumSession {
    convene(
        "q1",
        &council_convened(),
        &protocol_v1(),
        &candidates(),
        ACTOR,
        &gate_open(),
    )
    .unwrap()
}

fn reply(lane: &str, position: council::Position) -> council::Reply {
    council::Reply {
        lane_id: lane.to_string(),
        transport_served_model: format!("{lane}-served-model"),
        position,
    }
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

// --- the card ---------------------------------------------------------------

#[test]
fn card_v1_is_the_seven_stage_quorum_card() {
    let p = protocol_v1();
    assert_eq!(p.version, 1);
    assert_eq!(p.kind, ProtocolKind::Quorum);
    assert_eq!(p.stages, QUORUM_STAGES);
    assert_eq!(p.floors.panel_size, QUORUM_POOL_SIZE);
    assert_eq!(p.floors.panel_size, 3);
    assert_eq!(p.floors.min_families, 2);
    assert_eq!(p.floors.min_non_chinese, 1);
    validate_card(&p).unwrap();
    let pin = p.pin();
    assert_eq!(pin.len(), 64);
    assert!(pin.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')));
    // Stable: same card, same pin (deterministic canonical bytes), and
    // the quorum pin DIFFERS from the council pin (kind is in the bytes).
    assert_eq!(protocol_v1().pin(), pin);
    assert_ne!(pin, council::protocol_v1().pin());
}

#[test]
fn validate_refuses_wrong_kind() {
    let mut p = protocol_v1();
    p.kind = ProtocolKind::Council;
    let err = validate_card(&p).unwrap_err();
    assert!(matches!(err, QuorumErr::CardInvalid(_)));
    assert!(err.is_refusal());
}

#[test]
fn validate_refuses_stage_drift() {
    let mut missing = protocol_v1();
    missing.stages.remove(2);
    assert!(matches!(
        validate_card(&missing).unwrap_err(),
        QuorumErr::CardInvalid(_)
    ));

    let mut extra = protocol_v1();
    extra.stages.push("improvise".into());
    assert!(matches!(
        validate_card(&extra).unwrap_err(),
        QuorumErr::CardInvalid(_)
    ));

    let mut panel_word = protocol_v1();
    panel_word.stages[1] = "panel".into();
    assert!(matches!(
        validate_card(&panel_word).unwrap_err(),
        QuorumErr::CardInvalid(_)
    ));
}

#[test]
fn validate_refuses_version_zero_and_incoherent_floors() {
    assert!(matches!(
        validate_card(&card(0, protocol_v1().floors)).unwrap_err(),
        QuorumErr::CardInvalid(_)
    ));
    let beyond_pool = Floors {
        min_families: protocol_v1().floors.panel_size + 1,
        ..protocol_v1().floors
    };
    assert!(matches!(
        validate_card(&card(1, beyond_pool)).unwrap_err(),
        QuorumErr::CardInvalid(_)
    ));
    let solo = Floors {
        panel_size: 1,
        min_families: 1,
        min_non_chinese: 1,
    };
    let err = validate_card(&card(1, solo)).unwrap_err();
    assert!(matches!(err, QuorumErr::CardInvalid(_)));
    assert!(err.to_string().contains("majority floor"));
}

#[test]
fn decision_floor_is_the_strict_majority_of_the_full_pool() {
    assert_eq!(decision_floor(3), 2); // the v1 "floor 2/3" ruling
    assert_eq!(decision_floor(2), 2);
    assert_eq!(decision_floor(4), 3);
    assert_eq!(decision_floor(5), 3);
}

// --- convene (+ pool) -------------------------------------------------------

#[test]
fn convene_happy_path_links_the_council_and_pins_the_card() {
    let council = council_convened();
    let session = convene(
        "q1",
        &council,
        &protocol_v1(),
        &candidates(),
        ACTOR,
        &gate_open(),
    )
    .expect("the happy day convenes");
    // The SAME staged questions — the council's task verbatim.
    assert_eq!(session.task, council.convening.task);
    assert_eq!(session.council_convening, "c1");
    assert_eq!(session.stakes, Stakes::Complex);
    // F3: the ONE pin, stored at convene.
    assert_eq!(session.pinned_protocol, protocol_v1().pin());
    assert_eq!(session.rerun_of, None);
    assert_eq!(session.gate.actor, ACTOR);
    // The pool: three peers, in selection order, F9-disjoint from the
    // council panel.
    let pool_lanes: Vec<&str> = session
        .pool
        .seats
        .iter()
        .map(|s| s.lane_id.as_str())
        .collect();
    assert_eq!(pool_lanes, vec!["mistral/m", "nvidia/nem", "openai/x"]);
    session
        .pool
        .check_disjoint_from(&council.convening.panel)
        .expect("F9: zero overlap with the council panel");
}

#[test]
fn convene_refuses_when_the_gate_is_closed() {
    let err = convene(
        "q1",
        &council_convened(),
        &protocol_v1(),
        &candidates(),
        ACTOR,
        "", // no warden ledger rows: no active card
    )
    .unwrap_err();
    assert!(matches!(err, QuorumErr::GateClosed { .. }));
    assert!(err.is_refusal());
}

#[test]
fn convene_pool_skips_council_lanes_before_ordering() {
    // Council lanes are FREE; the only disjoint candidates are PREMIUM.
    // Skip-then-order: the overlapped free seats never compete — the
    // pool is still disjoint, never short-handed or overlapping.
    let mut cands = vec![
        seat("groq/llama", "groq", CostClass::Free, SeatState::Live),
        seat("openai/x", "openai", CostClass::Free, SeatState::Live),
        seat("zai/glm", "zai", CostClass::Free, SeatState::Live),
        seat("nvidia/nem", "nvidia", CostClass::Free, SeatState::Live),
        seat(
            "anthropic/x",
            "anthropic",
            CostClass::Premium,
            SeatState::Live,
        ),
        seat("gemini/y", "gemini", CostClass::Premium, SeatState::Live),
        seat("mistral/z", "mistral", CostClass::Premium, SeatState::Live),
    ];
    cands.sort_by(|a, b| a.lane_id.cmp(&b.lane_id));
    let council = council::convene(
        "c9",
        "rule on the staff-system brief",
        Stakes::Complex,
        &council::protocol_v1(),
        &cands,
        ACTOR,
        &gate_open(),
    )
    .unwrap();
    let session = convene("q1", &council, &protocol_v1(), &cands, ACTOR, &gate_open()).unwrap();
    let pool_lanes: Vec<&str> = session
        .pool
        .seats
        .iter()
        .map(|s| s.lane_id.as_str())
        .collect();
    assert_eq!(pool_lanes, vec!["anthropic/x", "gemini/y", "mistral/z"]);
    session
        .pool
        .check_disjoint_from(&council.convening.panel)
        .expect("F9 holds even when the council lanes are cheapest");
}

#[test]
fn convene_refuses_the_degraded_day_honestly() {
    // Only TWO disjoint live candidates exist — the pool of 3 cannot form.
    let short_day = vec![
        seat(
            "anthropic/claude",
            "anthropic",
            CostClass::Free,
            SeatState::Live,
        ),
        seat("cohere/c", "cohere", CostClass::Free, SeatState::Live),
        seat("gemini/g", "gemini", CostClass::Free, SeatState::Live),
        seat("groq/llama", "groq", CostClass::Free, SeatState::Live),
        seat("mistral/m", "mistral", CostClass::Free, SeatState::Live),
        seat("nvidia/nem", "nvidia", CostClass::Free, SeatState::Live),
    ];
    let err = convene(
        "q1",
        &council_convened(),
        &protocol_v1(),
        &short_day,
        ACTOR,
        &gate_open(),
    )
    .unwrap_err();
    match err {
        QuorumErr::Pool(crate::disjoint::DisjointErr::InsufficientDisjointPool {
            have,
            want,
            ..
        }) => {
            assert_eq!((have, want), (2, 3));
        }
        other => panic!("expected InsufficientDisjointPool, got {other:?}"),
    }
    assert!(err.is_refusal());
}

// --- dispatch (as DATA) -----------------------------------------------------

#[test]
fn dispatch_plan_serializes_a_capped_provider_across_waves() {
    let session = convened_under_v1();
    let reg = registry_with(
        &[("shared", 1)],
        &[
            ("mistral/m", "shared"),
            ("nvidia/nem", "shared"),
            ("openai/x", "shared"),
        ],
    );
    let plan = dispatch_plan(&session, &reg).unwrap();
    assert_eq!(
        plan,
        vec![
            vec!["mistral/m".to_string()],
            vec!["nvidia/nem".to_string()],
            vec!["openai/x".to_string()],
        ]
    );
}

// --- collect ----------------------------------------------------------------

#[test]
fn collect_happy_full_pool_is_pool_ordered() {
    let session = convened_under_v1();
    let votes = collect(
        &session,
        &[
            reply("openai/x", council::Position::Ship),
            reply("mistral/m", council::Position::Ship),
            reply("nvidia/nem", council::Position::ShipWithChanges),
        ],
    )
    .unwrap();
    assert!(votes.missing.is_empty());
    let lanes: Vec<&str> = votes.replies.iter().map(|r| r.lane_id.as_str()).collect();
    assert_eq!(lanes, vec!["mistral/m", "nvidia/nem", "openai/x"]);
}

#[test]
fn collect_tolerates_a_missing_seat_and_records_it() {
    let session = convened_under_v1();
    let votes = collect(
        &session,
        &[
            reply("mistral/m", council::Position::Ship),
            reply("nvidia/nem", council::Position::Ship),
        ],
    )
    .unwrap();
    assert_eq!(votes.missing, vec!["openai/x"]);
    assert_eq!(votes.replies.len(), 2);
}

#[test]
fn collect_refuses_below_the_floor() {
    let session = convened_under_v1();
    let err = collect(&session, &[reply("mistral/m", council::Position::Ship)]).unwrap_err();
    match &err {
        QuorumErr::CollectIncomplete {
            missing,
            have,
            floor,
        } => {
            assert_eq!(
                missing,
                &vec!["nvidia/nem".to_string(), "openai/x".to_string()]
            );
            assert_eq!((*have, *floor), (1, 2));
        }
        other => panic!("expected CollectIncomplete, got {other:?}"),
    }
    assert!(err.is_refusal());
}

#[test]
fn collect_refuses_crossing_duplicate_and_blank_provenance() {
    let session = convened_under_v1();
    // Identity crossing: a lane outside the pool.
    let crossing = collect(
        &session,
        &[
            reply("mistral/m", council::Position::Ship),
            reply("nvidia/nem", council::Position::Ship),
            reply("groq/llama", council::Position::Ship), // council lane!
        ],
    )
    .unwrap_err();
    assert!(matches!(crossing, QuorumErr::Defect(_)));
    assert!(!crossing.is_refusal());

    let dup = collect(
        &session,
        &[
            reply("mistral/m", council::Position::Ship),
            reply("mistral/m", council::Position::Ship),
            reply("nvidia/nem", council::Position::Ship),
        ],
    )
    .unwrap_err();
    assert!(matches!(dup, QuorumErr::Defect(_)));

    let mut blank = reply("mistral/m", council::Position::Ship);
    blank.transport_served_model.clear();
    let blank = collect(&session, &[blank]).unwrap_err();
    assert!(matches!(blank, QuorumErr::Defect(_)));
    assert!(blank.to_string().contains("blank form"));
}

// --- integrate + verdict ----------------------------------------------------

#[test]
fn integrate_reuses_the_council_clustering_law() {
    let session = convened_under_v1();
    let votes = collect(
        &session,
        &[
            reply("mistral/m", council::Position::Ship),
            reply("nvidia/nem", council::Position::Ship),
            reply("openai/x", council::Position::DoNotShip),
        ],
    )
    .unwrap();
    let map = integrate(&votes);
    // Fixed cluster order, all three positions present, lanes sorted.
    assert_eq!(map.summary(), "ship=2,ship_with_changes=0,do_not_ship=1");
    assert_eq!(
        map.holding(council::Position::Ship),
        ["mistral/m", "nvidia/nem"]
    );
    assert!(map.holding(council::Position::ShipWithChanges).is_empty());
    assert_eq!(map.holding(council::Position::DoNotShip), ["openai/x"]);
    assert!(map.disagrees());
}

#[test]
fn verdict_clean_two_of_three_ruling_carries_no_asterisk() {
    let session = convened_under_v1();
    let votes = collect(
        &session,
        &[
            reply("mistral/m", council::Position::Ship),
            reply("nvidia/nem", council::Position::Ship),
            reply("openai/x", council::Position::ShipWithChanges),
        ],
    )
    .unwrap();
    let map = integrate(&votes);
    let (verdict, ruling) = verdict(&session, &votes, &map).unwrap();
    assert_eq!(verdict.ruling, "ship");
    assert!(!verdict.degraded);
    assert_eq!(ruling.position, council::Position::Ship);
    assert_eq!(ruling.agreeing, ["mistral/m", "nvidia/nem"]);
    assert_eq!((ruling.floor, ruling.pool_size), (2, 3));
    assert_eq!(verdict.provenance.len(), 3);
}

#[test]
fn verdict_degraded_ruling_carries_the_asterisk() {
    let session = convened_under_v1();
    let votes = collect(
        &session,
        &[
            reply("mistral/m", council::Position::Ship),
            reply("nvidia/nem", council::Position::Ship),
        ],
    )
    .unwrap();
    let map = integrate(&votes);
    let (verdict, ruling) = verdict(&session, &votes, &map).unwrap();
    assert_eq!(verdict.ruling, "ship*");
    assert!(verdict.degraded);
    // The floor is met by the FULL pool's majority (2 of 3) — the seat
    // is missing, the floor is not discounted.
    assert_eq!((ruling.floor, ruling.pool_size), (2, 3));
}

#[test]
fn verdict_split_and_sub_floor_splits_refuse_fail_closed() {
    let session = convened_under_v1();
    // Full pool, three-way split: no position at the floor.
    let votes = collect(
        &session,
        &[
            reply("mistral/m", council::Position::Ship),
            reply("nvidia/nem", council::Position::ShipWithChanges),
            reply("openai/x", council::Position::DoNotShip),
        ],
    )
    .unwrap();
    let map = integrate(&votes);
    let err = verdict(&session, &votes, &map).unwrap_err();
    match &err {
        QuorumErr::FloorUnmet { counts, floor } => {
            assert_eq!(*floor, 2);
            assert_eq!(counts, "ship=1,ship_with_changes=1,do_not_ship=1");
        }
        other => panic!("expected FloorUnmet, got {other:?}"),
    }
    assert!(err.is_refusal());

    // Two answering seats in DISAGREEMENT: the floor is unreachable.
    let votes = collect(
        &session,
        &[
            reply("mistral/m", council::Position::Ship),
            reply("nvidia/nem", council::Position::DoNotShip),
        ],
    )
    .unwrap();
    let map = integrate(&votes);
    let err = verdict(&session, &votes, &map).unwrap_err();
    assert!(matches!(err, QuorumErr::FloorUnmet { .. }));
    assert!(err.is_refusal());
}

#[test]
fn verdict_md_is_the_deterministic_artifact_with_asterisk_and_missing() {
    let session = convened_under_v1();
    let votes = collect(
        &session,
        &[
            reply("mistral/m", council::Position::Ship),
            reply("nvidia/nem", council::Position::Ship),
        ],
    )
    .unwrap();
    let map = integrate(&votes);
    let (degraded_v, degraded_r) = verdict(&session, &votes, &map).unwrap();
    let md = verdict_md(&session, &degraded_v, &degraded_r, &map, &votes);
    assert_eq!(
        md,
        "# VERDICT — q1\n\
         \n\
         verdict: ship*\n\
         floor: 2/3\n\
         council: c1\n\
         degraded: true\n\
         table: ship=2,ship_with_changes=0,do_not_ship=0\n\
         seats:\n\
         - mistral/m -> ship (served by mistral/m-served-model)\n\
         - nvidia/nem -> ship (served by nvidia/nem-served-model)\n\
         missing: openai/x\n"
    );
    // Clean variant: no asterisk, missing none.
    let votes = collect(
        &session,
        &[
            reply("mistral/m", council::Position::Ship),
            reply("nvidia/nem", council::Position::Ship),
            reply("openai/x", council::Position::Ship),
        ],
    )
    .unwrap();
    let map = integrate(&votes);
    let (clean_v, clean_r) = verdict(&session, &votes, &map).unwrap();
    let md = verdict_md(&session, &clean_v, &clean_r, &map, &votes);
    assert!(md.contains("verdict: ship\n"));
    assert!(md.contains("degraded: false"));
    assert!(md.ends_with("missing: none\n"));
}

// --- ledger -----------------------------------------------------------------

#[test]
fn ledger_row_round_trips_and_reuses_the_one_digest_law() {
    let session = convened_under_v1();
    let votes = collect(
        &session,
        &[
            reply("mistral/m", council::Position::Ship),
            reply("nvidia/nem", council::Position::Ship),
        ],
    )
    .unwrap();
    let map = integrate(&votes);
    let (verdict, ruling) = verdict(&session, &votes, &map).unwrap();
    let row = ledger_row(&session, &verdict, &ruling, &map, &votes);
    let parsed = parse_ledger_row(&row).unwrap();
    assert_eq!(parsed.conv, "q1");
    assert_eq!(parsed.council, "c1");
    assert_eq!(parsed.pin, session.pinned_protocol);
    assert_eq!(parsed.stakes, "complex");
    assert_eq!(parsed.rerun_of, "");
    assert_eq!(parsed.actor, ACTOR);
    assert_eq!(parsed.warden_card, "CARD-0011");
    assert_eq!(
        (parsed.ship, parsed.ship_with_changes, parsed.do_not_ship),
        (2, 0, 0)
    );
    assert_eq!(parsed.missing, 1);
    assert_eq!(parsed.ruled, "ship");
    assert_eq!(parsed.floor, "2/3");
    assert!(parsed.degraded);
    // ONE digest law: the quorum digest IS the council canonical bytes
    // hashed — shared framing, never a second format.
    assert_eq!(
        parsed.verdict_digest,
        crate::sha256::hex(council::canonical_verdict_bytes(&verdict, &map).as_bytes())
    );
}

#[test]
fn parse_refuses_kind_drift_field_drift_and_law_violations() {
    let session = convened_under_v1();
    let votes = collect(
        &session,
        &[
            reply("mistral/m", council::Position::Ship),
            reply("nvidia/nem", council::Position::Ship),
            reply("openai/x", council::Position::Ship),
        ],
    )
    .unwrap();
    let map = integrate(&votes);
    let (verdict, ruling) = verdict(&session, &votes, &map).unwrap();
    let row = ledger_row(&session, &verdict, &ruling, &map, &votes);
    parse_ledger_row(&row).unwrap();

    // A council row is not a quorum row.
    let wrong_kind = row.replace("\"kind\":\"quorum\"", "\"kind\":\"council\"");
    assert!(matches!(
        parse_ledger_row(&wrong_kind).unwrap_err(),
        QuorumErr::Defect(_)
    ));
    // Field drift (unknown extra field).
    let extra_field = row.replace('}', ",\"bonus\":1}");
    assert!(matches!(
        parse_ledger_row(&extra_field).unwrap_err(),
        QuorumErr::Defect(_)
    ));
    // The majority law enforced on READ: floor 1/3 is not a majority.
    let bad_floor = row
        .replace("\"floor\":\"3/3\"", "\"floor\":\"1/3\"")
        .replace("\"ship\":3", "\"ship\":1");
    assert!(matches!(
        parse_ledger_row(&bad_floor).unwrap_err(),
        QuorumErr::Defect(_)
    ));
    // Degradation is exactly missing-ness.
    let fake_degraded = row.replace("\"degraded\":\"false\"", "\"degraded\":\"true\"");
    assert!(matches!(
        parse_ledger_row(&fake_degraded).unwrap_err(),
        QuorumErr::Defect(_)
    ));
    // Digest must be 64 lowercase hex.
    let bad_digest = row.replace(&parsed_digest(&row), "XYZ");
    assert!(matches!(
        parse_ledger_row(&bad_digest).unwrap_err(),
        QuorumErr::Defect(_)
    ));
}

fn parsed_digest(row: &str) -> String {
    parse_ledger_row(row).unwrap().verdict_digest
}

// --- F3 + F11 ---------------------------------------------------------------

#[test]
fn check_pin_reports_intact_and_moved() {
    let session = convened_under_v1();
    assert_eq!(
        check_pin(&session, &protocol_v1()).unwrap(),
        council::PinOutcome::Intact
    );
    let moved_card = card(2, protocol_v1().floors);
    match check_pin(&session, &moved_card).unwrap() {
        council::PinOutcome::Moved(m) => {
            assert_eq!(m.pinned, session.pinned_protocol);
            assert_eq!(m.actual, moved_card.pin());
        }
        council::PinOutcome::Intact => panic!("a version bump must move the pin"),
    }
}

#[test]
fn pause_and_re_dispatch_bumps_reselects_and_archives() {
    let session = convened_under_v1();
    let council = council_convened();
    let v2 = card(2, protocol_v1().floors);
    let out = pause_and_re_dispatch(
        session.clone(),
        &protocol_v1(),
        &v2,
        &council,
        &candidates(),
        ACTOR,
        &gate_open(),
    )
    .unwrap();
    assert_eq!(out.re_dispatched.id, "q1#r2");
    assert_eq!(out.re_dispatched.rerun_of, Some("q1".to_string()));
    assert_ne!(out.re_dispatched.pinned_protocol, session.pinned_protocol);
    assert_eq!(out.re_dispatched.pinned_protocol, v2.pin());
    assert_eq!(out.re_dispatched.task, session.task);
    assert_eq!(out.re_dispatched.council_convening, "c1");
    // The archived original is immutable — moved, never mutated.
    assert_eq!(out.archived, session);
    // The re-dispatched pool is re-proven F9-disjoint.
    out.re_dispatched
        .pool
        .check_disjoint_from(&council.convening.panel)
        .unwrap();
}

#[test]
fn pause_refuses_missing_bump_wrong_card_and_wrong_council() {
    let session = convened_under_v1();
    let council = council_convened();
    // No bump.
    let err = pause_and_re_dispatch(
        session.clone(),
        &protocol_v1(),
        &card(1, protocol_v1().floors),
        &council,
        &candidates(),
        ACTOR,
        &gate_open(),
    )
    .unwrap_err();
    assert!(matches!(
        err,
        QuorumErr::VersionNotBumped {
            have: 1,
            try_use: 1
        }
    ));
    assert!(err.is_refusal());

    // Archived card is not what the session ran under.
    let err = pause_and_re_dispatch(
        session.clone(),
        &card(9, protocol_v1().floors),
        &card(10, protocol_v1().floors),
        &council,
        &candidates(),
        ACTOR,
        &gate_open(),
    )
    .unwrap_err();
    assert!(matches!(err, QuorumErr::Defect(_)));

    // A DIFFERENT council — F9 disjointness is proven against the SAME
    // panel; swapping councils mid-flight is a Defect.
    let other_council = council::convene(
        "c2",
        "rule on the staff-system brief",
        Stakes::Complex,
        &council::protocol_v1(),
        &candidates(),
        ACTOR,
        &gate_open(),
    )
    .unwrap();
    let err = pause_and_re_dispatch(
        session,
        &protocol_v1(),
        &card(2, protocol_v1().floors),
        &other_council,
        &candidates(),
        ACTOR,
        &gate_open(),
    )
    .unwrap_err();
    assert!(matches!(err, QuorumErr::Defect(_)));
    assert!(err.to_string().contains("SAME council"));
}
