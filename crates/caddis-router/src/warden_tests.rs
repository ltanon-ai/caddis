//! warden.rs tests — R5. The HMAC is pinned to RFC 4231 vectors (a MAC
//! nobody has checked against the standard is a random function with good
//! marketing), the key file law to the exact mint shape.

use super::*;

fn tmpdir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("rtr-warden-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn hmac_rfc4231_case1_short_key() {
    let key = [0x0bu8; 20];
    let mac = hmac_sha256(&key, b"Hi There");
    assert_eq!(
        hex64(&mac),
        "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
    );
}

#[test]
fn hmac_rfc4231_case2_jefe() {
    let mac = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
    assert_eq!(
        hex64(&mac),
        "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
    );
}

#[test]
fn hmac_rfc4231_case6_long_key_is_hashed() {
    // 131-byte key: longer than the 64-byte block, so the RFC hashes it first.
    let key = [0xaau8; 131];
    let mac = hmac_sha256(
        &key,
        b"Test Using Larger Than Block-Size Key - Hash Key First",
    );
    assert_eq!(
        hex64(&mac),
        "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
    );
}

#[test]
fn sign_check_roundtrip_and_tamper() {
    let k = WardenKey {
        key: [7u8; 32],
        activated_seq: 5,
    };
    let canonical = r#"{"seq":6,"ts":"2026-08-28T00:00:00Z","kind":"outcome"}"#;
    let sig = k.sign(canonical);
    assert_eq!(sig.len(), 64);
    assert!(k.check(canonical, &sig));
    // One flipped byte in the claimed canonical row -> signature fails.
    let tampered = canonical.replace('6', "7");
    assert!(!k.check(&tampered, &sig), "tampered row must fail");
    // Malformed sig strings are failed checks, never panics.
    assert!(!k.check(canonical, "zz"));
    assert!(!k.check(canonical, ""));
    assert!(!k.check(canonical, &sig[..63]));
    assert!(!k.check(canonical, &format!("{sig}00")));
}

#[test]
fn fingerprint_is_stable_and_not_the_key() {
    let k = WardenKey {
        key: [1u8; 32],
        activated_seq: 0,
    };
    let fp = k.fingerprint();
    assert_eq!(fp.len(), 16);
    assert_eq!(fp, k.fingerprint(), "fingerprint is deterministic");
    assert!(!hex64(&k.key).starts_with(&fp), "not the key itself");
    let other = WardenKey {
        key: [2u8; 32],
        activated_seq: 0,
    };
    assert_ne!(fp, other.fingerprint(), "different key, different print");
}

#[test]
fn mint_writes_the_two_line_shape_and_loads_back() {
    let dir = tmpdir("mint");
    let k = mint(&dir, 2440).unwrap();
    assert_eq!(k.activated_seq(), 2440);
    let text = std::fs::read_to_string(dir.join("warden.key")).unwrap();
    let mut lines = text.lines();
    let l1 = lines.next().unwrap();
    assert_eq!(l1.len(), 64, "hex key on line 1: {text}");
    assert!(
        l1.chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "lowercase hex"
    );
    assert_eq!(lines.next().unwrap(), "activated_seq=2440");
    assert!(lines.next().is_none(), "exactly two lines");
    // Loads back as the same key.
    match WardenSlot::load(&dir.join("ledger.jsonl")) {
        WardenSlot::Key(loaded) => {
            assert_eq!(loaded, k);
            assert_eq!(loaded.fingerprint(), k.fingerprint());
        }
        other => panic!("expected Key, got {other:?}"),
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn mint_refuses_overwrite_and_two_mints_differ() {
    let dir = tmpdir("mint2");
    let k1 = mint(&dir, 0).unwrap();
    let err = mint(&dir, 0).unwrap_err();
    assert!(err.contains("refusing"), "{err}");
    let k2 = mint(&tmpdir("mint3"), 0).unwrap();
    assert_ne!(
        k1.fingerprint(),
        k2.fingerprint(),
        "keys are fresh, not derived"
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn slot_states_absent_and_broken_shapes() {
    // Missing file beside the ledger -> Absent (not activated).
    let dir = tmpdir("slot");
    assert_eq!(
        WardenSlot::load(&dir.join("ledger.jsonl")),
        WardenSlot::Absent
    );
    // Each malformed shape is Broken with a reason, never a silent Absent.
    // Reuse a valid-length key for the seq/extra cases.
    let good_key = "ab".repeat(32);
    let cases: Vec<String> = vec![
        "".into(),
        format!("zzzz\nactivated_seq=1\n"),
        format!("{good_key}\n"),                       // missing line 2
        format!("{good_key}\nactivated_seq=x\n"),      // seq not u64
        format!("{good_key}\nactivated_seq=1\nextra"), // extra line
    ];
    for (i, text) in cases.iter().enumerate() {
        std::fs::write(dir.join("warden.key"), text).unwrap();
        match WardenSlot::load(&dir.join("ledger.jsonl")) {
            WardenSlot::Broken(why) => assert!(!why.is_empty(), "case {i}: {text:?}"),
            other => panic!("case {i} ({text:?}) must be Broken, got {other:?}"),
        }
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn broken_slot_fails_closed_on_sign() {
    let slot = WardenSlot::Broken("line 1 is not 64 hex".into());
    assert!(slot.sign("{}").is_err());
    assert_eq!(WardenSlot::Absent.sign("{}"), Ok(None));
    let k = WardenKey {
        key: [9u8; 32],
        activated_seq: 0,
    };
    assert!(matches!(
        WardenSlot::Key(k).sign("{}"),
        Ok(Some(sig)) if sig.len() == 64
    ));
}
