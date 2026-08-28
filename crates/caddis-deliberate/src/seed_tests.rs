//! seed_tests.rs — P4 slice 3 gates: the F13 signed-seed verify-gate.
//!
//! The Done-When this file proves: tampered seed = REFUSED (verify names
//! the broken law; restore constructs NOTHING), clean seed = byte-exact
//! reconstruction with the view re-proven through the real loader.

use super::*;
use std::fs;
use std::path::PathBuf;

fn provider(id: &str) -> registry::Card {
    registry::Card::Provider(registry::ProviderCard {
        id: id.into(),
        lane_type: crate::LaneType::Http,
        base_url: format!("https://{id}.example/v1"),
        auth_path: String::new(),
        probe_path: String::new(),
        caps: 1,
        source: "models.json#deadbeef".into(),
    })
}

fn seat(
    id: &str,
    provider: &str,
    state: crate::SeatState,
    cost: crate::CostClass,
) -> registry::Card {
    registry::Card::Seat(registry::SeatCard {
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

fn home_fixture(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("caddis-seed-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let cards = vec![
        provider("groq"),
        provider("zai"),
        seat(
            "groq/llama-4",
            "groq",
            crate::SeatState::Live,
            crate::CostClass::Free,
        ),
        seat(
            "zai/glm-4.6",
            "zai",
            crate::SeatState::Probing,
            crate::CostClass::Free,
        ),
    ];
    fs::write(dir.join("seats.jsonl"), registry::render_seed(&cards)).unwrap();
    dir
}

// --- primitives against OUTSIDE vectors (a hash nobody has checked
// --- against the standard is a random function with good marketing).

#[test]
fn hmac_matches_rfc_4231_case_1() {
    // RFC 4231 Test Case 1: key = 0x0b x20, data = "Hi There".
    let key = [0x0bu8; 20];
    let mac = hmac_sha256(&key, b"Hi There");
    assert_eq!(
        hex64(&mac),
        "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
    );
}

#[test]
fn base64_known_vectors_and_round_trip() {
    assert_eq!(b64_encode(b""), "");
    assert_eq!(b64_encode(b"f"), "Zg==");
    assert_eq!(b64_encode(b"fo"), "Zm8=");
    assert_eq!(b64_encode(b"foo"), "Zm9v");
    assert_eq!(b64_encode(b"foobar"), "Zm9vYmFy");
    for len in 0..64usize {
        let data: Vec<u8> = (0..len).map(|i| (i * 7 + 3) as u8).collect();
        assert_eq!(b64_decode(&b64_encode(&data)).unwrap(), data, "len {len}");
    }
    assert!(b64_decode("A").is_none(), "len%4");
    assert!(b64_decode("A===").is_none(), "pad in wrong position");
    assert!(b64_decode("Zm9v!g==").is_none(), "bad alphabet char");
    assert!(b64_decode("Zg==Zg==").is_none(), "data after padding");
}

// --- export → verify → restore: the clean round trip.

#[test]
fn export_verify_restore_round_trip() {
    let home = home_fixture("rt");
    let ex = export_seed(&home).unwrap();
    assert!(ex.key_minted, "first export mints the key");
    assert_eq!(ex.rows, 4);

    let slot = SeedKeySlot::load(&home.join(KEY_FILE));
    let v = verify_seed_text(&ex.artifact, &slot);
    assert!(v.clean, "findings: {:?}", v.findings);
    assert_eq!(v.rows, Some(4));

    let target = std::env::temp_dir().join(format!("caddis-seed-rt-target-{}", std::process::id()));
    let _ = fs::remove_dir_all(&target);
    match restore_seed(&ex.artifact, &slot, &target).unwrap() {
        RestoreOutcome::Constructed { rows } => assert_eq!(rows, 4),
        other => panic!("expected Constructed, got {other:?}"),
    }
    let src = fs::read_to_string(home.join("seats.jsonl")).unwrap();
    let dst = fs::read_to_string(target.join("seats.jsonl")).unwrap();
    assert_eq!(src, dst, "byte-exact reconstruction");
    // The view was re-derived through the REAL loader, never trusted
    // from the artifact (the artifact does not even carry it).
    let view = fs::read_to_string(target.join("seats-view.json")).unwrap();
    assert!(
        view.contains(&ex.stream_sha256),
        "view proves the signed stream digest"
    );
    // Idempotent re-restore over identical bytes.
    match restore_seed(&ex.artifact, &slot, &target).unwrap() {
        RestoreOutcome::AlreadyIdentical { rows } => assert_eq!(rows, 4),
        other => panic!("expected AlreadyIdentical, got {other:?}"),
    }
    fs::remove_dir_all(&home).ok();
    fs::remove_dir_all(&target).ok();
}

#[test]
fn export_is_deterministic_for_same_stream_and_key() {
    let home = home_fixture("det");
    let a = export_seed(&home).unwrap();
    let b = export_seed(&home).unwrap();
    assert!(!b.key_minted, "second export reuses the born-once key");
    assert_eq!(a.artifact, b.artifact);
    fs::remove_dir_all(&home).ok();
}

#[test]
fn mint_is_born_once_and_refuses_overwrite() {
    let home = home_fixture("mint");
    let dir = home.join("nested");
    let k1 = mint_seed_key(&dir, 7).unwrap();
    assert_eq!(k1.born_rows(), 7);
    assert!(k1.fingerprint().len() == 16);
    let err = mint_seed_key(&dir, 7).unwrap_err();
    assert!(err.contains("refusing"), "{err}");
    let k2 = mint_seed_key(&home.join("other"), 7).unwrap();
    assert_ne!(k1.fingerprint(), k2.fingerprint(), "keys are fresh");
    fs::remove_dir_all(&home).ok();
}

// --- the F13 GATE: tampering is visible and construction refuses.

/// Splice a DIFFERENT stream into the artifact text, keeping the original
/// signature and digest (the attacker's exact move).
fn tamper_stream(artifact: &str, new_stream: &str) -> String {
    let start = artifact.find("\"stream_b64\":\"").unwrap() + "\"stream_b64\":\"".len();
    let end = artifact[start..].find('"').unwrap() + start;
    format!(
        "{}{}{}",
        &artifact[..start],
        b64_encode(new_stream.as_bytes()),
        &artifact[end..]
    )
}

#[test]
fn tampered_payload_refused_and_restore_writes_nothing() {
    let home = home_fixture("tamper");
    let ex = export_seed(&home).unwrap();
    let slot = SeedKeySlot::load(&home.join(KEY_FILE));

    // Same row COUNT, different VALUES — only the crypto + digest laws
    // can catch this one (the row-count check alone would pass).
    let cards = vec![
        provider("groq"),
        provider("EVIL"),
        seat(
            "groq/llama-4",
            "groq",
            crate::SeatState::Live,
            crate::CostClass::Free,
        ),
        seat(
            "zai/glm-4.6",
            "zai",
            crate::SeatState::Live,
            crate::CostClass::Premium,
        ),
    ];
    let evil = tamper_stream(&ex.artifact, &registry::render_seed(&cards));
    let v = verify_seed_text(&evil, &slot);
    assert!(!v.clean);
    assert!(
        v.findings
            .iter()
            .any(|f| f.starts_with("STREAM_DIGEST_MISMATCH")),
        "{:?}",
        v.findings
    );
    assert!(
        v.findings.iter().any(|f| f.starts_with("SIG_MISMATCH")),
        "{:?}",
        v.findings
    );

    let target =
        std::env::temp_dir().join(format!("caddis-seed-tamper-target-{}", std::process::id()));
    let _ = fs::remove_dir_all(&target);
    fs::create_dir_all(&target).unwrap();
    let err = restore_seed(&evil, &slot, &target).unwrap_err();
    assert!(err.contains("REFUSED"), "{err}");
    assert!(
        !target.join("seats.jsonl").exists(),
        "construction refused — NOTHING written"
    );
    fs::remove_dir_all(&home).ok();
    fs::remove_dir_all(&target).ok();
}
#[test]
fn tampered_sig_refused() {
    let home = home_fixture("sig");
    let ex = export_seed(&home).unwrap();
    let slot = SeedKeySlot::load(&home.join(KEY_FILE));
    // Keep the payload, forge the signature (64 hex of zeros).
    let canonical_end = ex.artifact.rfind(",\"sig\":\"").unwrap();
    let forged = format!(
        "{},\"sig\":\"{}\"}}\n",
        &ex.artifact[..canonical_end],
        "0".repeat(64)
    );
    let v = verify_seed_text(&forged, &slot);
    assert!(!v.clean);
    assert!(
        v.findings.iter().any(|f| f.starts_with("SIG_MISMATCH")),
        "{:?}",
        v.findings
    );
    fs::remove_dir_all(&home).ok();
}

#[test]
fn absent_and_broken_keys_fail_closed() {
    let home = home_fixture("keylaw");
    let ex = export_seed(&home).unwrap();

    let absent = verify_seed_text(&ex.artifact, &SeedKeySlot::Absent);
    assert!(!absent.clean);
    assert!(absent.findings[0].starts_with("KEY_ABSENT"));

    fs::write(home.join(KEY_FILE), "garbage\nnot-a-shape\n").unwrap();
    let broken = verify_seed_text(&ex.artifact, &SeedKeySlot::load(&home.join(KEY_FILE)));
    assert!(!broken.clean);
    assert!(broken.findings[0].starts_with("KEY_BROKEN"));
    // Export under a broken key refuses — fail closed, never unsigned.
    let err = export_seed(&home).unwrap_err();
    assert!(err.contains("fail closed"), "{err}");
    fs::remove_dir_all(&home).ok();
}

// --- shape law: exact fields, exact kind, exact version.

#[test]
fn shape_law_refuses_forged_envelopes() {
    let home = home_fixture("shape");
    let ex = export_seed(&home).unwrap();
    let slot = SeedKeySlot::load(&home.join(KEY_FILE));

    // Unknown extra field.
    let extra = ex.artifact.replace("\"sig\":", "\"rogue\":1,\"sig\":");
    let v = verify_seed_text(&extra, &slot);
    assert!(
        !v.clean && v.findings[0].starts_with("BAD_SHAPE"),
        "{:?}",
        v.findings
    );

    // Missing member.
    let start = ex.artifact.find(",\"fingerprint\":\"").unwrap();
    let end = ex.artifact[start + 1..].find('"').unwrap() + start + 1;
    let missing = format!("{}{}", &ex.artifact[..start], &ex.artifact[end + 1..]);
    let v = verify_seed_text(&missing, &slot);
    assert!(
        !v.clean && v.findings[0].starts_with("BAD_SHAPE"),
        "{:?}",
        v.findings
    );

    // Wrong kind.
    let wrong_kind = ex.artifact.replace(SEED_KIND, "caddis-evil-seed");
    let v = verify_seed_text(&wrong_kind, &slot);
    assert!(
        !v.clean && v.findings[0].starts_with("BAD_SHAPE"),
        "{:?}",
        v.findings
    );

    // Wrong version.
    let wrong_v = ex.artifact.replace("\"v\":1,", "\"v\":2,");
    let v = verify_seed_text(&wrong_v, &slot);
    assert!(
        !v.clean && v.findings[0].starts_with("VERSION"),
        "{:?}",
        v.findings
    );
    fs::remove_dir_all(&home).ok();
}

#[test]
fn restore_never_clobbers_a_diverged_home() {
    let home = home_fixture("clobber");
    let ex = export_seed(&home).unwrap();
    let slot = SeedKeySlot::load(&home.join(KEY_FILE));

    let target = home.join("diverged");
    fs::create_dir_all(&target).unwrap();
    let other_cards = vec![provider("someone-else")];
    fs::write(
        target.join("seats.jsonl"),
        registry::render_seed(&other_cards),
    )
    .unwrap();

    let err = restore_seed(&ex.artifact, &slot, &target).unwrap_err();
    assert!(err.contains("refusing to clobber"), "{err}");
    // The diverged home is untouched.
    assert_eq!(
        fs::read_to_string(target.join("seats.jsonl")).unwrap(),
        registry::render_seed(&other_cards)
    );
    fs::remove_dir_all(&home).ok();
}

#[test]
fn key_material_never_appears_in_artifact_or_report() {
    let home = home_fixture("secrecy");
    let ex = export_seed(&home).unwrap();
    let key_line = fs::read_to_string(home.join(KEY_FILE))
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .to_string();
    assert!(!ex.artifact.contains(&key_line), "raw key must never ship");
    assert!(!ex.fingerprint.contains(&key_line[..8]));
    // Fingerprint is the ONLY key-derived surface: 16 hex of sha256(key).
    let v = verify_seed_text(&ex.artifact, &SeedKeySlot::load(&home.join(KEY_FILE)));
    assert!(v.clean);
    assert_eq!(v.fingerprint.as_deref(), Some(ex.fingerprint.as_str()));
    fs::remove_dir_all(&home).ok();
}
