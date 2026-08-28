//! protocol_tests.rs — P0 slice 2 gates (plan Done-When, slices 2): the
//! F3 protocol PIN (hash of canonical bytes, flips on ANY field change,
//! escaped-stage injectivity), Convening::open (pin computed never
//! supplied; floor/size refusals), verify_pin mid-flight-edit rejection,
//! and serde round-trips of the new types (dev-only serde law).

use std::time::SystemTime;

use crate::protocol::{
    Convening, ConveningErr, DispatchEntry, PinMismatch, Protocol, ProtocolKind, ProvenanceRow,
    Verdict,
};
use crate::sha256;
use crate::{construct_panel, CostClass, Floors, LaneType, PanelErr, Seat, SeatState};

fn seat(id: &str, family: &str, cost: CostClass) -> Seat {
    Seat {
        lane_id: id.to_string(),
        lane_type: LaneType::Http,
        family: family.to_string(),
        provider: family.to_string(),
        model: format!("{family}-model"),
        cost_class: cost,
        state: SeatState::Live,
        caps: 1,
        last_probe: Some(SystemTime::UNIX_EPOCH),
    }
}

fn protocol() -> Protocol {
    Protocol {
        version: 1,
        kind: ProtocolKind::Council,
        stages: vec![
            "convene".into(),
            "panel".into(),
            "dispatch".into(),
            "collect".into(),
            "integrate".into(),
            "verdict".into(),
            "ledger".into(),
        ],
        floors: Floors::default(),
    }
}

/// A candidate pool that satisfies default floors (3 families, non-Chinese
/// present) — the happy-path substrate for convening tests.
fn pool() -> Vec<Seat> {
    vec![
        seat("gemini", "google", CostClass::Free),
        seat("zai-a", "zai", CostClass::Free),
        seat("zai-b", "zai", CostClass::Free),
        seat("mistral", "mistral", CostClass::Mid),
    ]
}

#[test]
fn pin_is_sha256_hex_of_canonical() {
    let p = protocol();
    let pin = p.pin();
    assert_eq!(pin.len(), 64);
    assert!(pin
        .chars()
        .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    assert_eq!(pin, sha256::hex(p.canonical().as_bytes()));
}

#[test]
fn any_behavioral_field_change_flips_the_pin() {
    let base = protocol();
    let pin = base.pin();
    assert_eq!(base.clone().pin(), pin, "equal protocols must pin equal");

    let bump_version = Protocol {
        version: 2,
        ..base.clone()
    };
    let change_kind = Protocol {
        kind: ProtocolKind::Quorum,
        ..base.clone()
    };
    let edit_stage = Protocol {
        stages: vec!["convene".into(), "panel".into(), "verdict".into()],
        ..base.clone()
    };
    let change_floors = Protocol {
        floors: Floors {
            min_families: 3,
            ..Floors::default()
        },
        ..base
    };
    for edited in [bump_version, change_kind, edit_stage, change_floors] {
        assert_ne!(edited.pin(), pin, "{:?} must re-pin", edited.version);
    }
}

/// Injectivity of the canonical form: a stage name carrying framing
/// characters must never alias a DIFFERENT stage list that would encode to
/// the same raw bytes without escaping.
#[test]
fn canonical_escapes_framing_characters() {
    let sneaky = Protocol {
        stages: vec!["a\",\"b".into()],
        ..protocol()
    };
    let naive = Protocol {
        stages: vec!["a".into(), "b".into()],
        ..protocol()
    };
    assert_ne!(sneaky.pin(), naive.pin());
    assert!(
        sneaky.canonical().contains("a\\\",\\\"b"),
        "{}",
        sneaky.canonical()
    );
}

#[test]
fn convening_open_pins_the_protocol() {
    let p = protocol();
    let panel = construct_panel(&pool(), &p.floors).unwrap();
    let c = Convening::open("conv-1", "rule the organ rewrite", &p, panel).unwrap();
    assert_eq!(c.pinned_protocol, p.pin());
    assert_eq!(c.id, "conv-1");
    assert_eq!(c.task, "rule the organ rewrite");
    assert!(c.dispatch_log.is_empty());
    assert_eq!(c.panel.seats.len(), p.floors.panel_size);
}

/// F3 proof: the pin stored at open time still verifies; the SAME-VERSION
/// sneaky edit (floors changed, version forgot to bump) is REJECTED with
/// both hashes carried as evidence.
#[test]
fn verify_pin_rejects_midflight_edit_f3() {
    let p = protocol();
    let panel = construct_panel(&pool(), &p.floors).unwrap();
    let c = Convening::open("conv-1", "task", &p, panel).unwrap();
    assert!(c.verify_pin(&p).is_ok());

    let edited = Protocol {
        floors: Floors {
            min_families: 3,
            ..p.floors
        },
        ..p
    };
    let err = c.verify_pin(&edited).unwrap_err();
    assert_eq!(
        err,
        PinMismatch {
            pinned: c.pinned_protocol.clone(),
            actual: edited.pin()
        }
    );
    let msg = err.to_string();
    assert!(
        msg.contains(&c.pinned_protocol) && msg.contains(&edited.pin()),
        "{msg}"
    );
    assert!(msg.contains("F3"), "{msg}");
}

#[test]
fn open_refuses_floor_violating_panel() {
    let p = protocol();
    let panel = construct_panel(&pool(), &p.floors).unwrap();
    let stricter = Protocol {
        floors: Floors {
            min_families: 4,
            ..p.floors
        },
        ..p
    };
    assert_eq!(
        Convening::open("conv-1", "task", &stricter, panel),
        Err(ConveningErr::Floor(PanelErr::FamiliesFloor {
            have: 3,
            want: 4
        }))
    );
}

#[test]
fn open_refuses_panel_size_mismatch() {
    let p = protocol();
    let panel = construct_panel(&pool(), &p.floors).unwrap();
    let smaller = Protocol {
        floors: Floors {
            panel_size: 3,
            ..p.floors
        },
        ..p
    };
    assert_eq!(
        Convening::open("conv-1", "task", &smaller, panel),
        Err(ConveningErr::PanelSizeMismatch { have: 4, want: 3 })
    );
}

#[test]
fn convening_and_verdict_round_trip_serde() {
    let p = protocol();
    let panel = construct_panel(&pool(), &p.floors).unwrap();
    let mut c = Convening::open("conv-7", "task text", &p, panel).unwrap();
    c.dispatch_log.push(DispatchEntry {
        stage: "dispatch".into(),
        lane_id: "gemini".into(),
        payload_digest: sha256::hex(b"payload"),
    });
    let json = serde_json::to_string(&c).unwrap();
    assert_eq!(serde_json::from_str::<Convening>(&json).unwrap(), c);

    let v = Verdict {
        convening_id: "conv-7".into(),
        ruling: "SHIP-WITH-CHANGES".into(),
        provenance: vec![ProvenanceRow {
            lane_id: "gemini".into(),
            lane_type: LaneType::Bridge,
            transport_served_model: "gemini-2.5-pro".into(),
        }],
        degraded: true,
    };
    let json = serde_json::to_string(&v).unwrap();
    assert_eq!(serde_json::from_str::<Verdict>(&json).unwrap(), v);
}
