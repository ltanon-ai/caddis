//! lib_tests.rs — P0 slice 1 gates (plan Done-When, slice 1): panel
//! construction (roles, free-first ordering, floors-as-data refusals,
//! non-Live exclusion, determinism) + serde round-trip of every public
//! type (dev-only serde — zero runtime deps law holds).

use std::time::SystemTime;

use crate::{
    construct_panel, is_chinese_family, CostClass, Floors, LaneType, PanelErr, Role, Seat,
    SeatState, ROLE_ORDER,
};

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
        last_probe: Some(SystemTime::UNIX_EPOCH),
    }
}

/// Happy path: four live seats across three families, one Premium left
/// OUT — free-first ordering and ROLE_ORDER assignment both proven.
#[test]
fn happy_path_orders_roles_free_first() {
    let candidates = vec![
        seat("openai-x", "openai", CostClass::Premium, SeatState::Live),
        seat("zai-b", "zai", CostClass::Free, SeatState::Live),
        seat("gemini", "google", CostClass::Free, SeatState::Live),
        seat("mistral", "mistral", CostClass::Mid, SeatState::Live),
        seat("zai-a", "zai", CostClass::Free, SeatState::Live),
    ];
    let panel = construct_panel(&candidates, &Floors::default()).unwrap();
    // Free first (lex ties): gemini, zai-a, zai-b; then Mid: mistral.
    // The Premium openai seat must NOT be seated.
    let ids: Vec<&str> = panel
        .seats
        .iter()
        .map(|ps| ps.seat.lane_id.as_str())
        .collect();
    assert_eq!(ids, vec!["gemini", "zai-a", "zai-b", "mistral"]);
    let roles: Vec<Role> = panel.seats.iter().map(|ps| ps.role).collect();
    assert_eq!(roles, ROLE_ORDER.to_vec());
    assert_eq!(panel.family_count(), 3);
    assert_eq!(panel.non_chinese_count(), 2);
}

/// Monoculture: all seats one family -> FamiliesFloor refusal.
#[test]
fn families_floor_refuses_monoculture() {
    let candidates = vec![
        seat("zai-a", "zai", CostClass::Free, SeatState::Live),
        seat("zai-b", "zai", CostClass::Free, SeatState::Live),
        seat("zai-c", "zai", CostClass::Free, SeatState::Live),
        seat("zai-d", "zai", CostClass::Free, SeatState::Live),
    ];
    assert_eq!(
        construct_panel(&candidates, &Floors::default()),
        Err(PanelErr::FamiliesFloor { have: 1, want: 2 })
    );
}

/// Families floor PASSES (4 Chinese families) but the non-Chinese floor
/// refuses: distinct-family is not enough, monoculture of ORIGIN is the
/// floor's meaning.
#[test]
fn non_chinese_floor_refuses_chinese_cluster() {
    let candidates = vec![
        seat("zai-a", "zai", CostClass::Free, SeatState::Live),
        seat("deepseek-a", "deepseek", CostClass::Free, SeatState::Live),
        seat("qwen-a", "qwen", CostClass::Free, SeatState::Live),
        seat("moonshot-a", "moonshot", CostClass::Free, SeatState::Live),
    ];
    assert_eq!(
        construct_panel(&candidates, &Floors::default()),
        Err(PanelErr::NonChineseFloor { have: 0, want: 1 })
    );
}

/// Fixed refusal order: when BOTH floors fail, families is reported first.
#[test]
fn refusal_order_families_before_non_chinese() {
    let candidates = vec![
        seat("zai-a", "zai", CostClass::Free, SeatState::Live),
        seat("zai-b", "zai", CostClass::Free, SeatState::Live),
        seat("zai-c", "zai", CostClass::Free, SeatState::Live),
        seat("zai-d", "zai", CostClass::Free, SeatState::Live),
        seat("gemini", "google", CostClass::Premium, SeatState::Live),
    ];
    // Cheapest 4 = all zai -> families 1 < 2 AND non-Chinese 0 < 1.
    assert_eq!(
        construct_panel(&candidates, &Floors::default()),
        Err(PanelErr::FamiliesFloor { have: 1, want: 2 })
    );
}

/// F10 vocabulary: every non-Live state is excluded from selection; two
/// Live seats cannot fill a 4-seat panel -> NotEnoughLiveSeats.
#[test]
fn non_live_states_never_seated() {
    let candidates = vec![
        seat("live-a", "zai", CostClass::Free, SeatState::Live),
        seat("live-b", "google", CostClass::Free, SeatState::Live),
        seat("exp", "google", CostClass::Free, SeatState::Expired),
        seat("rl", "mistral", CostClass::Free, SeatState::RateLimited),
        seat("ret", "mistral", CostClass::Free, SeatState::Retired),
        seat("prb", "openai", CostClass::Free, SeatState::Probing),
        seat("fld", "openai", CostClass::Free, SeatState::Failed),
    ];
    assert_eq!(
        construct_panel(&candidates, &Floors::default()),
        Err(PanelErr::NotEnoughLiveSeats { have: 2, need: 4 })
    );
    // A degraded-day operator ruling of a 2-seat panel works on the Live pair.
    let floors = Floors {
        panel_size: 2,
        ..Floors::default()
    };
    let panel = construct_panel(&candidates, &floors).unwrap();
    assert_eq!(panel.seats.len(), 2);
    assert_eq!(panel.seats[0].role, Role::Chair);
    assert_eq!(panel.seats[1].role, Role::Synthesist);
}

/// Floors are DATA: a smaller panel, a zeroed monoculture floor — the same
/// constructor serves them without code changes.
#[test]
fn floors_are_data() {
    let chinese_pool = vec![
        seat("zai-a", "zai", CostClass::Free, SeatState::Live),
        seat("deepseek-a", "deepseek", CostClass::Free, SeatState::Live),
        seat("qwen-a", "qwen", CostClass::Free, SeatState::Live),
        seat("moonshot-a", "moonshot", CostClass::Free, SeatState::Live),
    ];
    // Same pool that failed the default non-Chinese floor: ruled to 0, it
    // convenes (families floor still enforced by its own value).
    let floors = Floors {
        min_non_chinese: 0,
        ..Floors::default()
    };
    let panel = construct_panel(&chinese_pool, &floors).unwrap();
    assert_eq!(panel.family_count(), 4);
    assert_eq!(panel.non_chinese_count(), 0);

    // panel_size is data too: 3 seats, three roles, no LogicChecker.
    let floors3 = Floors {
        panel_size: 3,
        min_non_chinese: 0,
        ..Floors::default()
    };
    let panel3 = construct_panel(&chinese_pool, &floors3).unwrap();
    assert_eq!(panel3.seats.len(), 3);
    assert_eq!(panel3.seats[2].role, Role::Critic);
}

/// Malformed floor sizes refuse: 0 and above ROLE_ORDER length.
#[test]
fn panel_size_out_of_range() {
    let candidates = vec![
        seat("zai-a", "zai", CostClass::Free, SeatState::Live),
        seat("gemini", "google", CostClass::Free, SeatState::Live),
    ];
    let zero = Floors {
        panel_size: 0,
        ..Floors::default()
    };
    assert_eq!(
        construct_panel(&candidates, &zero),
        Err(PanelErr::PanelSizeOutOfRange { given: 0, max: 4 })
    );
    let over = Floors {
        panel_size: 5,
        ..Floors::default()
    };
    assert_eq!(
        construct_panel(&candidates, &over),
        Err(PanelErr::PanelSizeOutOfRange { given: 5, max: 4 })
    );
}

/// Determinism (F1 replay law): input order must not matter, and the same
/// input yields the same panel.
#[test]
fn deterministic_regardless_of_input_order() {
    let candidates = vec![
        seat("zai-b", "zai", CostClass::Free, SeatState::Live),
        seat("gemini", "google", CostClass::Free, SeatState::Live),
        seat("zai-a", "zai", CostClass::Free, SeatState::Live),
        seat("mistral", "mistral", CostClass::Mid, SeatState::Live),
    ];
    let mut reversed = candidates.clone();
    reversed.reverse();
    assert_eq!(
        construct_panel(&candidates, &Floors::default()).unwrap(),
        construct_panel(&reversed, &Floors::default()).unwrap()
    );
    assert_eq!(
        construct_panel(&candidates, &Floors::default()).unwrap(),
        construct_panel(&candidates, &Floors::default()).unwrap()
    );
}

/// CHINESE_FAMILIES data table: case-insensitive membership, known outs.
#[test]
fn chinese_family_table_is_data() {
    assert!(is_chinese_family("zai"));
    assert!(is_chinese_family("ZAI"));
    assert!(is_chinese_family("DeepSeek"));
    assert!(is_chinese_family("qwen"));
    assert!(!is_chinese_family("google"));
    assert!(!is_chinese_family("mistral"));
    assert!(!is_chinese_family("openai"));
}

/// Plan Done-When: types round-trip through serde (dev-only proof; the
/// wire format freeze is a later phase).
#[test]
fn serde_round_trips_public_types() {
    let s = seat("zai-a", "zai", CostClass::Mid, SeatState::RateLimited);
    let j = serde_json::to_string(&s).unwrap();
    assert_eq!(serde_json::from_str::<Seat>(&j).unwrap(), s);

    let candidates = vec![
        seat("zai-a", "zai", CostClass::Free, SeatState::Live),
        seat("gemini", "google", CostClass::Free, SeatState::Live),
        seat("zai-b", "zai", CostClass::Free, SeatState::Live),
        seat("mistral", "mistral", CostClass::Mid, SeatState::Live),
    ];
    let panel = construct_panel(&candidates, &Floors::default()).unwrap();
    let j = serde_json::to_string(&panel).unwrap();
    assert_eq!(serde_json::from_str::<crate::Panel>(&j).unwrap(), panel);

    let floors = Floors::default();
    let j = serde_json::to_string(&floors).unwrap();
    assert_eq!(serde_json::from_str::<Floors>(&j).unwrap(), floors);

    let role = Role::LogicChecker;
    let j = serde_json::to_string(&role).unwrap();
    assert_eq!(serde_json::from_str::<Role>(&j).unwrap(), role);

    let lt = LaneType::Cli;
    let j = serde_json::to_string(&lt).unwrap();
    assert_eq!(serde_json::from_str::<LaneType>(&j).unwrap(), lt);
}
