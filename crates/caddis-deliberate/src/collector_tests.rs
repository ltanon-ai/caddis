//! collector_tests.rs — P1 slice 1 gates for the models.json seed
//! collector. The fixture mirrors the REAL desktop catalog's shapes
//! (provider rows with api/baseUrl/apiKey + model rows with
//! contextWindow/maxTokens/cost) but carries ONLY synthetic credentials —
//! the raw-key shape is assembled at runtime from char codes so no
//! key-shaped literal ever enters the file (warden law).

use super::*;
use crate::registry::{parse_stream, Card, Registry};

/// A RAW inline credential of the exact shape models.json carries
/// (built from char codes; the warden refuses key-shaped literals).
fn raw_key() -> String {
    let prefix: String = ['s', 'k', '-'].iter().collect();
    let body: String = ['R'; 44].iter().collect();
    format!("{prefix}{body}")
}

/// The catalog fixture with the raw credential spliced in at runtime.
fn fixture() -> String {
    FIXTURE_TMPL.replace("@RAW_KEY@", &raw_key())
}

const FIXTURE_TMPL: &str = r#"{
  "providers": {
    "ollama-cloud": {
      "api": "openai-completions",
      "baseUrl": "https://ollama.com/v1",
      "apiKey": "@RAW_KEY@",
      "models": [
        {"id": "gpt-oss:20b", "name": "GPT OSS", "reasoning": false, "input": ["text"], "contextWindow": 1000000, "maxTokens": 32768, "cost": {"input": 0, "output": 0}},
        {"id": "qwen3.5:397b", "name": "Qwen", "reasoning": false, "input": ["text"], "contextWindow": 1000000, "maxTokens": 32768, "cost": {"input": 0, "output": 0}}
      ]
    },
    "zai-coding": {
      "api": "openai-completions",
      "baseUrl": "https://api.z.ai/api/coding/paas/v4",
      "apiKey": "C:/Users/alice/vault/keys/zai.key",
      "models": [
        {"id": "glm-4.6", "name": "GLM", "reasoning": true, "input": ["text"], "contextWindow": 128000, "maxTokens": 16384, "cost": {"input": 0, "output": 0}}
      ]
    },
    "openai-codex": {
      "models": [
        {"id": "gpt-5.5", "name": "Codex", "api": "openai-codex-responses", "reasoning": true, "input": ["text"], "contextWindow": 400000, "maxTokens": 128000, "cost": {"input": 5, "output": 30}}
      ]
    },
    "ghost": {
      "models": []
    },
    "broken": {
      "api": "openai-completions",
      "baseUrl": "https://broken.example/v1",
      "models": [
        {"id": "no-ctx", "name": "No ctx", "maxTokens": 100},
        {"id": "fine", "name": "Fine", "contextWindow": 1000, "maxTokens": 100}
      ]
    }
  }
}"#;

#[test]
fn collects_providers_and_seats_with_deterministic_order() {
    let report = collect(&fixture()).expect("fixture collects");
    // providers sorted by id; ghost skipped.
    let prov: Vec<&str> = report
        .cards
        .iter()
        .filter_map(|c| match c {
            Card::Provider(p) => Some(p.id.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        prov,
        vec!["broken", "ollama-cloud", "openai-codex", "zai-coding"]
    );
    // seats: 2 ollama-cloud + 1 zai + 1 codex + 1 broken/fine = 5.
    assert_eq!(report.cards.len(), 4 + 5);
    // deterministic: collect twice => identical card vectors.
    let again = collect(&fixture()).unwrap();
    assert_eq!(report.cards, again.cards);
}

#[test]
fn seed_renders_idempotent_bytes() {
    let (a, _) = render_seed_from(&fixture()).unwrap();
    let (b, _) = render_seed_from(&fixture()).unwrap();
    assert_eq!(a, b, "same catalog => byte-identical seed (Done-When)");
    assert!(a.ends_with('\n'));
}

#[test]
fn raw_api_key_never_enters_the_stream() {
    let (bytes, _) = render_seed_from(&fixture()).unwrap();
    assert!(
        !bytes.contains(&raw_key()),
        "a raw credential must NEVER be carried into the stream"
    );
    // and the round trip still parses.
    parse_stream(&bytes).expect("seed bytes parse as a stream");
}

#[test]
fn pathlike_api_key_becomes_auth_path() {
    let report = collect(&fixture()).unwrap();
    let zai = report
        .cards
        .iter()
        .find_map(|c| match c {
            Card::Provider(p) if p.id == "zai-coding" => Some(p),
            _ => None,
        })
        .unwrap();
    assert_eq!(zai.auth_path, "C:/Users/alice/vault/keys/zai.key");
    // raw-key provider: honest blank, credential stays in the source file.
    let ollama = report
        .cards
        .iter()
        .find_map(|c| match c {
            Card::Provider(p) if p.id == "ollama-cloud" => Some(p),
            _ => None,
        })
        .unwrap();
    assert_eq!(ollama.auth_path, "");
}

#[test]
fn lane_type_http_when_any_api_or_baseurl_present() {
    let report = collect(&fixture()).unwrap();
    for c in &report.cards {
        match c {
            Card::Provider(p) => assert_eq!(p.lane_type, crate::LaneType::Http),
            Card::Seat(s) => assert_eq!(s.lane_type, crate::LaneType::Http),
        }
    }
}

#[test]
fn provider_without_transport_is_skipped_with_cause() {
    let report = collect(&fixture()).unwrap();
    assert!(
        report
            .skipped
            .iter()
            .any(|s| s.contains("ghost") && s.contains("RULING")),
        "skips must name the provider and the cause: {:?}",
        report.skipped
    );
    assert!(
        report.skipped.iter().any(|s| s.contains("no-ctx")),
        "model skips carry causes: {:?}",
        report.skipped
    );
}

#[test]
fn cost_classes_derive_from_measured_cost() {
    let report = collect(&fixture()).unwrap();
    let seat = |id: &str| {
        report
            .cards
            .iter()
            .find_map(|c| match c {
                Card::Seat(s) if s.id == id => Some(s),
                _ => None,
            })
            .unwrap()
    };
    assert_eq!(
        seat("ollama-cloud/gpt-oss:20b").cost_class,
        crate::CostClass::Free
    );
    assert_eq!(
        seat("zai-coding/glm-4.6").cost_class,
        crate::CostClass::Free
    );
    assert_eq!(
        seat("openai-codex/gpt-5.5").cost_class,
        crate::CostClass::Mid
    );
    // premium is NEVER collector-derived (taste = ruling).
    assert!(!report.cards.iter().any(|c| matches!(
        c,
        Card::Seat(s) if s.cost_class == crate::CostClass::Premium
    )));
    assert_eq!(seat("openai-codex/gpt-5.5").cost_in_usd_per_mtok, 5.0);
    assert_eq!(seat("openai-codex/gpt-5.5").cost_out_usd_per_mtok, 30.0);
}

#[test]
fn seeds_are_probing_with_caps_one_and_family_equals_provider() {
    let report = collect(&fixture()).unwrap();
    for c in &report.cards {
        if let Card::Seat(s) = c {
            assert_eq!(s.state, crate::SeatState::Probing, "{} never probed", s.id);
            assert_eq!(s.caps, 1, "{} serialized by default (F4)", s.id);
            assert_eq!(s.since_epoch_s, 0, "{}: seed carries no clock data", s.id);
            assert_eq!(s.family, s.provider);
        }
    }
    // honest consequence: a fresh seed selects NOTHING (F10 fail-closed).
    let reg = Registry::fold(&report.cards);
    let err = crate::construct_panel(&reg.seats(), &crate::Floors::default()).unwrap_err();
    assert!(matches!(err, crate::PanelErr::NotEnoughLiveSeats { .. }));
}

#[test]
fn provider_caps_seed_from_the_ruling7_table() {
    let report = collect(&fixture()).unwrap();
    // ollama-cloud is IN the fixture and IN the Ruling-7 table: caps 1.
    let oc = report
        .cards
        .iter()
        .find_map(|c| match c {
            Card::Provider(p) if p.id == "ollama-cloud" => Some(p),
            _ => None,
        })
        .expect("fixture carries ollama-cloud");
    assert_eq!(oc.caps, crate::caps::ruled_caps("ollama-cloud"));
    assert_eq!(oc.caps, 1);
    // Every seeded provider satisfies the cap law, table or not.
    for c in &report.cards {
        if let Card::Provider(p) = c {
            crate::caps::check_provider_caps(&p.id, p.caps)
                .unwrap_or_else(|e| panic!("seed violates the cap law: {e}"));
        }
    }
    // And the whole folded registry validates.
    let reg = Registry::fold(&report.cards);
    crate::caps::validate_registry(&reg).expect("seeded registry satisfies the cap law");
}

#[test]
fn source_provenance_is_the_catalog_digest_not_a_clock() {
    let fx = fixture();
    let (bytes, _) = render_seed_from(&fx).unwrap();
    // THE sha256 of the catalog bytes, computed by an OUTSIDE toolchain
    // (python hashlib, 2026-08-28) and pinned — the double-hash defect
    // (hex(&sha256(..))) could never be caught by expectations derived
    // from the same helpers.
    let digest8 = "daba1feb";
    assert!(
        bytes.contains(&format!("models.json#{digest8}\"")),
        "provenance must be the single sha256 of the catalog bytes"
    );
    assert!(!bytes.contains("utc"), "no clocks in cards (idempotency)");
}

/// REAL-catalog smoke (runs where the desktop models.json exists; skips
/// elsewhere): the 13-provider registry collects, seeds byte-identically
/// twice, parses as a stream, folds, and carries NO credential value
/// from the source — every `apiKey` string in the catalog is scanned
/// against the rendered seed bytes.
#[test]
fn real_desktop_catalog_smoke() {
    let Some(home) = std::env::var_os("USERPROFILE") else {
        return;
    };
    let path = std::path::Path::new(&home)
        .join(".pi")
        .join("agent")
        .join("models.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return; // not this machine — synthetic gates still hold
    };
    let (seed_a, report) = render_seed_from(&text).expect("real catalog collects");
    let (seed_b, _) = render_seed_from(&text).unwrap();
    assert_eq!(seed_a, seed_b, "real catalog: idempotent seed bytes");
    let cards = parse_stream(&seed_a).expect("real seed parses");
    let reg = Registry::fold(&cards);
    assert!(
        reg.providers.len() >= 13,
        "real registry carries its 13 providers, got {}",
        reg.providers.len()
    );
    assert!(
        reg.seats.len() >= 70,
        "real registry carries its ~75 seats, got {}",
        reg.seats.len()
    );
    // NO credential may cross: any substantial key-shaped value under
    // "apiKey" (real tokens: long, no path separators) must be absent
    // from the seed bytes. Trivial placeholders (short values like a
    // provider's own name) collide with provider ids by construction —
    // they carry no secret and the collector drops them anyway.
    let parsed = crate::json::parse(&text).unwrap();
    if let Some(provs) = parsed.get("providers").and_then(|p| p.as_obj()) {
        for (pid, prow) in provs {
            if let Some(k) = prow.get("apiKey").and_then(|v| v.as_str()) {
                let k = k.trim();
                let placeholder = k.len() < 12
                    || pid.to_lowercase().contains(&k.to_lowercase())
                    || prow
                        .get("baseUrl")
                        .and_then(|b| b.as_str())
                        .map(|b| b.to_lowercase().contains(&k.to_lowercase()))
                        .unwrap_or(false);
                let pathlike = k.contains('/') || k.contains('\\') || k.starts_with("file:");
                if !k.is_empty() && !placeholder && !pathlike {
                    assert!(
                        !seed_a.contains(k),
                        "a raw credential leaked into the seed stream (provider {pid})"
                    );
                }
            }
        }
    }
    // every collected seat seeds `probing` (never probed => selection closed).
    assert!(reg
        .seats
        .values()
        .all(|s| s.state == crate::SeatState::Probing));
    assert!(
        report.skipped.is_empty(),
        "real catalog: no skips expected: {:?}",
        report.skipped
    );
}

#[test]
fn malformed_catalog_is_an_error_never_a_partial_seed() {
    assert!(collect("{\"providers\":").is_err());
    assert!(collect("{}").is_err());
    assert!(collect("not json").is_err());
}

#[test]
fn openai_codex_base_url_is_an_honest_blank() {
    let report = collect(&fixture()).unwrap();
    let codex = report
        .cards
        .iter()
        .find_map(|c| match c {
            Card::Provider(p) if p.id == "openai-codex" => Some(p),
            _ => None,
        })
        .unwrap();
    assert_eq!(codex.base_url, "");
    assert_eq!(
        codex.lane_type,
        crate::LaneType::Http,
        "per-model api counts"
    );
}

// ---------------------------------------------------------------------------
// P4 slice 1: seed_once — the one-time organ home bootstrap
// ---------------------------------------------------------------------------

use std::fs;

fn seed_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("caddis-dlib-seed-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn view_digest_matches(stream: &std::path::Path, view: &std::path::Path) -> bool {
    let text = fs::read_to_string(stream).unwrap();
    let v = crate::json::parse(&fs::read_to_string(view).unwrap()).unwrap();
    v.get("stream_sha256").and_then(|d| d.as_str())
        == Some(crate::registry::stream_digest(&text).as_str())
}

#[test]
fn seed_once_creates_home_and_proves_the_view() {
    let dir = seed_dir("create");
    let (stream, view) = (dir.join("seats.jsonl"), dir.join("seats-view.json"));
    let out = seed_once(&fixture(), &stream, &view).expect("seed creates");
    match out {
        SeedOutcome::Created {
            rows,
            skipped,
            view_synced,
        } => {
            assert!(rows > 0, "fixture yields cards");
            assert_eq!(rows, 9, "4 providers + 5 seats (fixture fact)");
            assert!(skipped >= 2, "ghost + no-ctx are reported skips");
            assert!(view_synced, "first sync always writes the view");
        }
        other => panic!("expected Created, got {other:?}"),
    }
    assert!(
        view_digest_matches(&stream, &view),
        "F2: view digest == stream truth"
    );
}

#[test]
fn seed_once_is_idempotent_on_identical_bytes() {
    let dir = seed_dir("idem");
    let (stream, view) = (dir.join("seats.jsonl"), dir.join("seats-view.json"));
    seed_once(&fixture(), &stream, &view).unwrap();
    let before = fs::read_to_string(&stream).unwrap();
    let out = seed_once(&fixture(), &stream, &view).expect("re-seed is not an error");
    match out {
        SeedOutcome::AlreadySeeded { rows, view_synced } => {
            assert_eq!(rows, 9);
            assert!(!view_synced, "identical bytes => view untouched");
        }
        other => panic!("expected AlreadySeeded, got {other:?}"),
    }
    assert_eq!(
        fs::read_to_string(&stream).unwrap(),
        before,
        "stream never rewritten"
    );
}

#[test]
fn seed_once_refuses_to_clobber_a_diverged_stream() {
    let dir = seed_dir("refuse");
    let (stream, view) = (dir.join("seats.jsonl"), dir.join("seats-view.json"));
    seed_once(&fixture(), &stream, &view).unwrap();
    // The edit path appended a row: the stream moved on from the seed.
    let mut evolved = fs::read_to_string(&stream).unwrap();
    let first_seat = evolved
        .lines()
        .find(|l| l.contains("\"class\":\"seat\""))
        .unwrap()
        .to_string();
    let bumped = first_seat.replace("\"state\":\"probing\"", "\"state\":\"live\"");
    evolved.push_str(&bumped);
    evolved.push('\n');
    fs::write(&stream, &evolved).unwrap();

    let err = seed_once(&fixture(), &stream, &view)
        .expect_err("a diverged stream must never be re-seeded");
    assert!(err.contains("refused"), "the refusal names its law: {err}");
    assert_eq!(
        fs::read_to_string(&stream).unwrap(),
        evolved,
        "diverged stream untouched"
    );
}

#[test]
fn seed_once_never_writes_on_malformed_catalog() {
    let dir = seed_dir("malformed");
    let (stream, view) = (dir.join("seats.jsonl"), dir.join("seats-view.json"));
    assert!(seed_once("{\"providers\":", &stream, &view).is_err());
    assert!(!stream.exists(), "no stream on a refused catalog");
    assert!(!view.exists(), "no view on a refused catalog");
}

#[test]
fn seed_once_two_fresh_homes_are_byte_identical() {
    let a = seed_dir("det-a");
    let b = seed_dir("det-b");
    seed_once(
        &fixture(),
        &a.join("seats.jsonl"),
        &a.join("seats-view.json"),
    )
    .unwrap();
    seed_once(
        &fixture(),
        &b.join("seats.jsonl"),
        &b.join("seats-view.json"),
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(a.join("seats.jsonl")).unwrap(),
        fs::read_to_string(b.join("seats.jsonl")).unwrap(),
        "deterministic bytes across homes"
    );
}

#[test]
fn seed_once_keeps_the_no_secrets_law_on_disk() {
    let dir = seed_dir("secrets");
    let (stream, view) = (dir.join("seats.jsonl"), dir.join("seats-view.json"));
    seed_once(&fixture(), &stream, &view).unwrap();
    let rendered = format!(
        "{}{}",
        fs::read_to_string(&stream).unwrap(),
        fs::read_to_string(&view).unwrap()
    );
    assert!(!rendered.contains(&raw_key()), "raw key never reaches disk");
    assert!(
        rendered.contains("C:/Users/alice/vault/keys/zai.key"),
        "vault PATH is carried"
    );
}
