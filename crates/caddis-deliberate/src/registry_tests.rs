//! registry_tests.rs — P1 slice 1 gates for the card stream + view cache.

use super::*;
use std::fs;

fn provider(id: &str) -> Card {
    Card::Provider(ProviderCard {
        id: id.into(),
        lane_type: crate::LaneType::Http,
        base_url: format!("https://{id}.example/v1"),
        auth_path: String::new(),
        probe_path: String::new(),
        caps: 1,
        source: "models.json#deadbeef".into(),
    })
}

fn seat(id: &str, provider: &str, state: crate::SeatState, cost: crate::CostClass) -> Card {
    Card::Seat(SeatCard {
        id: id.into(),
        provider: provider.into(),
        family: provider.into(),
        model: id.rsplit('/').next().unwrap().into(),
        lane_type: crate::LaneType::Http,
        cost_class: cost,
        state,
        since_epoch_s: 0,
        caps: 1,
        cost_in_usd_per_mtok: 0.0,
        cost_out_usd_per_mtok: 0.0,
        context_window: 128_000,
        max_tokens: 16_384,
        source: "models.json#deadbeef".into(),
    })
}

// --- encode/parse round-trip: what the writer writes is the only shape
// --- the loader accepts (audit==obey law).

#[test]
fn round_trip_provider_and_seat() {
    let cards = vec![
        provider("groq"),
        seat(
            "groq/llama-4",
            "groq",
            crate::SeatState::Probing,
            crate::CostClass::Free,
        ),
    ];
    let text = render_seed(&cards);
    let back = parse_stream(&text).expect("round trip parses");
    assert_eq!(back, cards);
}

#[test]
fn encode_is_byte_deterministic() {
    let cards = vec![
        provider("a"),
        seat("a/m1", "a", crate::SeatState::Live, crate::CostClass::Mid),
    ];
    assert_eq!(render_seed(&cards), render_seed(&cards));
}

#[test]
fn seed_renders_idempotent_bytes() {
    // the Done-When: same cards in => same bytes out, twice.
    let c = vec![
        provider("zai-coding"),
        seat(
            "zai-coding/glm-4.6",
            "zai-coding",
            crate::SeatState::Probing,
            crate::CostClass::Free,
        ),
    ];
    let a = render_seed(&c);
    let b = render_seed(&c);
    assert_eq!(a, b);
    assert!(a.ends_with('\n'));
    assert_eq!(a.lines().count(), 2);
}

// --- fail-closed field law.

#[test]
fn unknown_field_is_malformed_with_line_number() {
    let line = encode_card(&provider("groq"));
    let tampered = line.replace("\"source\":", "\"soruce\":\"x\",\"source\":");
    let err = parse_stream(&tampered).unwrap_err();
    match err {
        StreamErr::Malformed { line, ref msg } => {
            assert_eq!(line, 1);
            assert!(msg.contains("unknown field"), "got: {msg}");
        }
    }
}

#[test]
fn missing_field_is_malformed() {
    let line = encode_card(&provider("groq"));
    let cut = line.replace(",\"auth_path\":\"\"", "");
    let err = parse_stream(&cut).unwrap_err();
    match err {
        StreamErr::Malformed { line, ref msg } => {
            assert_eq!(line, 1);
            assert!(msg.contains("auth_path"), "got: {msg}");
        }
    }
}

#[test]
fn probe_path_law() {
    // Absent field = the honest blank: pre-extension rows keep parsing
    // (back-compat law — the stream is append-only and old rows persist).
    let legacy = "{\"class\":\"provider\",\"id\":\"x\",\"lane_type\":\"http\",\
                  \"base_url\":\"https://h/v1beta/openai\",\"auth_path\":\"\",\
                  \"caps\":1,\"source\":\"s\"}";
    let cards = parse_stream(legacy).expect("legacy row parses");
    match &cards[0] {
        Card::Provider(p) => assert_eq!(p.probe_path, ""),
        _ => panic!("provider row expected"),
    }

    // Absolute override parses and round-trips deterministically.
    let gem = match provider("gemini") {
        Card::Provider(mut pc) => {
            pc.base_url = "https://generativelanguage.googleapis.com/v1beta/openai".into();
            pc.probe_path = "/v1beta/models".into();
            pc
        }
        _ => panic!("provider card expected"),
    };
    let text = render_seed(&[Card::Provider(gem.clone())]);
    assert_eq!(
        parse_stream(&text).unwrap(),
        vec![Card::Provider(gem.clone())]
    );

    // Relative path refused (absolute-path law).
    let bad = encode_card(&provider("g"))
        .replace(",\"probe_path\":\"\"", ",\"probe_path\":\"v1beta/models\"");
    let err = parse_stream(&bad).unwrap_err();
    assert!(err.to_string().contains("probe_path"), "{err}");

    // Non-string refused.
    let bad2 = encode_card(&provider("g")).replace(",\"probe_path\":\"\"", ",\"probe_path\":7");
    assert!(
        parse_stream(&bad2).is_err(),
        "non-string probe_path must be refused"
    );
}

#[test]
fn nested_value_is_malformed() {
    let nested = "{\"class\":\"provider\",\"id\":\"x\",\"lane_type\":\"http\",\"base_url\":\"\",\"auth_path\":{\"k\":1},\"source\":\"s\"}";
    let err = parse_stream(nested).unwrap_err();
    assert!(err.to_string().contains("flat"), "{err}");
}

#[test]
fn unknown_class_lane_type_state_cost_refused() {
    for bad in [
        "{\"class\":\"bee\",\"id\":\"x\"}",
        "{\"class\":\"provider\",\"id\":\"x\",\"lane_type\":\"droid\",\"base_url\":\"\",\"auth_path\":\"\",\"source\":\"s\"}",
        "{\"class\":\"seat\",\"id\":\"x\",\"provider\":\"p\",\"family\":\"p\",\"model\":\"m\",\"lane_type\":\"http\",\"cost_class\":\"cheap\",\"state\":\"probing\",\"caps\":1,\"cost_in_usd_per_mtok\":0,\"cost_out_usd_per_mtok\":0,\"context_window\":1,\"max_tokens\":1,\"source\":\"s\"}",
    ] {
        assert!(parse_stream(bad).is_err(), "must refuse: {bad}");
    }
}

#[test]
fn negative_cost_is_malformed() {
    let line = encode_card(&seat(
        "p/m",
        "p",
        crate::SeatState::Live,
        crate::CostClass::Mid,
    ))
    .replace("\"cost_in_usd_per_mtok\":0", "\"cost_in_usd_per_mtok\":-1");
    assert!(parse_stream(&line).is_err());
}

// --- fold: append-only, last row per key wins.

#[test]
fn fold_last_row_wins() {
    let cards = vec![
        seat(
            "groq/m",
            "groq",
            crate::SeatState::Probing,
            crate::CostClass::Free,
        ),
        seat(
            "groq/m",
            "groq",
            crate::SeatState::Live,
            crate::CostClass::Free,
        ),
    ];
    let reg = Registry::fold(&cards);
    assert_eq!(reg.seats.len(), 1);
    assert_eq!(reg.seats["groq/m"].state, crate::SeatState::Live);
}

#[test]
fn fold_keeps_provider_and_seat_namespaces_separate() {
    // same id under two classes = two rows (keys are class+id).
    let cards = vec![
        provider("groq"),
        seat(
            "groq",
            "groq",
            crate::SeatState::Probing,
            crate::CostClass::Free,
        ),
    ];
    let reg = Registry::fold(&cards);
    assert_eq!(reg.providers.len(), 1);
    assert_eq!(reg.seats.len(), 1);
}

// --- view: digest-verified cache, re-synced on each row (F2).

#[test]
fn load_syncs_missing_and_stale_view() {
    let dir = std::env::temp_dir().join(format!("caddis-dlib-reg-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let stream = dir.join("seats.jsonl");
    let view = dir.join("seats-view.json");
    fs::remove_file(&stream).ok();
    fs::remove_file(&view).ok();

    // First load on an empty-but-present stream: view written.
    fs::write(&stream, render_seed(&[provider("groq")])).unwrap();
    let (reg, wrote) = load_and_sync(&stream, &view).unwrap();
    assert!(wrote);
    assert_eq!(reg.providers.len(), 1);
    let v1 = fs::read_to_string(&view).unwrap();
    assert!(v1.contains("\"stream_sha256\""));

    // Second load with an UNCHANGED stream: view is trusted (no rewrite).
    let (_, wrote2) = load_and_sync(&stream, &view).unwrap();
    assert!(!wrote2, "unchanged stream must not rewrite the view");

    // Append a row: view re-syncs (F2) and the fold sees the new card.
    let (_, wrote3) = append_card(
        &stream,
        &view,
        &seat(
            "groq/m",
            "groq",
            crate::SeatState::Probing,
            crate::CostClass::Free,
        ),
    )
    .unwrap();
    assert!(wrote3);
    let (reg3, _) = load_and_sync(&stream, &view).unwrap();
    assert_eq!(reg3.seats.len(), 1);

    // Tamper the view digest: loader re-derives (cache never trusted).
    let tampered = v1.replace("\"rows\":1", "\"rows\":999");
    fs::write(&view, tampered).unwrap();
    let (_, wrote4) = load_and_sync(&stream, &view).unwrap();
    assert!(wrote4, "stale view must be re-derived");
    let fixed = fs::read_to_string(&view).unwrap();
    assert!(
        fixed.contains("\"rows\":2"),
        "view re-derived from the real stream"
    );

    fs::remove_file(&stream).ok();
    fs::remove_file(&view).ok();
}

#[test]
fn stream_digest_matches_the_outside_toolchain() {
    // THE sha256 of the bytes, computed by python hashlib (2026-08-28) and
    // pinned here. `stream_digest` once double-hashed (hex(&sha256(..)) —
    // `hex` is the complete one-shot helper, not an encoder); expectations
    // derived from the same helpers could never see it. Any external
    // verifier (world bridge, seed gate, scripting) MUST land on this
    // exact value for these bytes.
    let pin = "caddis-deliberate stream digest external-truth pin v1\n";
    assert_eq!(
        stream_digest(pin),
        "4dbc7f49b96aac06b1fadada96e966c0019ba1e5f29bb5b60cb1c3f7f24ed0a2"
    );
}

#[test]
fn view_is_byte_deterministic_per_stream() {
    let cards = vec![
        provider("b"),
        provider("a"),
        seat("b/m", "b", crate::SeatState::Live, crate::CostClass::Free),
    ];
    let reg = Registry::fold(&cards);
    let d = stream_digest("x");
    assert_eq!(reg.encode_view(&d, 3), reg.encode_view(&d, 3));
    // BTree order: provider "a" sorts before "b" in the encoded view.
    let v = reg.encode_view(&d, 3);
    let a = v.find("\"id\":\"a\"").unwrap();
    let b = v.find("\"id\":\"b\"").unwrap();
    assert!(a < b);
}

// --- registry feeds the substrate: one law, one selection order.

#[test]
fn registry_seats_project_onto_substrate_and_construct_panel() {
    // Fresh seeds are `probing` => NOT selectable => panel refuses (F10).
    let seeds = vec![
        provider("groq"),
        seat(
            "groq/m",
            "groq",
            crate::SeatState::Probing,
            crate::CostClass::Free,
        ),
        seat(
            "zai-coding/glm",
            "zai-coding",
            crate::SeatState::Probing,
            crate::CostClass::Free,
        ),
    ];
    let reg = Registry::fold(&seeds);
    let err = crate::construct_panel(&reg.seats(), &crate::Floors::default()).unwrap_err();
    assert!(
        matches!(err, crate::PanelErr::NotEnoughLiveSeats { .. }),
        "{err:?}"
    );

    // Append two `live` supersessions => panel constructs free-first
    // (panel_size 2: the two free seats, two distinct families).
    let mut cards = seeds;
    cards.push(seat(
        "groq/m",
        "groq",
        crate::SeatState::Live,
        crate::CostClass::Free,
    ));
    cards.push(seat(
        "zai-coding/glm",
        "zai-coding",
        crate::SeatState::Live,
        crate::CostClass::Free,
    ));
    cards.push(seat(
        "openai-codex/gpt",
        "openai-codex",
        crate::SeatState::Live,
        crate::CostClass::Mid,
    ));
    let reg = Registry::fold(&cards);
    let floors = crate::Floors {
        panel_size: 2,
        ..crate::Floors::default()
    };
    let panel = crate::construct_panel(&reg.seats(), &floors).unwrap();
    let ids: Vec<&str> = panel
        .seats
        .iter()
        .map(|s| s.seat.lane_id.as_str())
        .collect();
    assert_eq!(
        ids,
        vec!["groq/m", "zai-coding/glm"],
        "free-first, ties by lane_id"
    );
}

#[test]
fn malformed_stream_never_half_loads() {
    let good = encode_card(&provider("groq"));
    let bad = "{\"class\":\"provider\"}";
    let text = format!("{good}\n{bad}\n");
    let err = parse_stream(&text).unwrap_err();
    match err {
        StreamErr::Malformed { line, .. } => assert_eq!(line, 2),
    }
}
