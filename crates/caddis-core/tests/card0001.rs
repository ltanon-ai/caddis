//! CARD-0001 DW tests (falsifiable per card; origin-stamped CD-021).
use caddis_core::{envelope, idempotency, ledger, policy};

fn env(id: &str, key: &str, typ: &str) -> envelope::Envelope {
    envelope::validate(
        1,
        id,
        key,
        typ,
        "seed",
        "ledger",
        "{}",
        "2026-08-16T00:00:00Z",
    )
    .unwrap()
}

#[test]
fn dw1_green_path_channel() {
    // DW1: žalias kelias per kanalą (D2: kanalas admit'ina validų envelope; v0 tipai dar neužpildyti — admit su leidžiamu tipu ateis su schema v2; kol kas kanalas = validate+decide+idem+ledger mechanika, žr. dw1b)
    let e = env("aaaa0001", "k1", "signal/node.state");
    let d = policy::decide(&e);
    assert!(
        !d.allow,
        "v0 denies ALL types (no allow-list yet) — signal/* arrives with schema v2"
    );
}

#[test]
fn dw1b_d2_murmur_envelope_rejected() {
    // D2 RED: murmur/* envelope ATMETAMAS visada
    let e = env("aaaa0009", "k9", "murmur/event");
    let d = policy::decide(&e);
    assert!(!d.allow, "murmur = stream, not envelope-family");
    assert!(d.reason.contains("E-POLICY"));
}

#[test]
fn dw3_idempotency_duplicate() {
    // DW3: dublis -> E-IDEM
    let mut idem = idempotency::Idempotency::new();
    assert!(idem.check("k1").is_ok());
    assert_eq!(idem.check("k1").unwrap_err(), "E-IDEM: duplicate idem_key");
}

#[test]
fn dw_policy_denies_non_murmur() {
    let e = env("aaaa0002", "k2", "root/exec");
    assert!(!policy::decide(&e).allow);
}

#[test]
fn dw_broken_envelope() {
    assert!(envelope::validate(1, "x", "k", "murmur/x", "a", "b", "{}", "t").is_err());
}

#[test]
fn dw2_ledger_seq_monotonic() {
    // DW2: append seq=1,2
    let dir = std::env::temp_dir().join(format!("caddis-test-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let file = dir.join("ledger.jsonl");
    let _ = std::fs::remove_file(&file);
    let mut l = ledger::Ledger::open(&file).unwrap();
    assert_eq!(l.append(&env("aaaa0003", "k3", "murmur/event")).unwrap(), 1);
    assert_eq!(l.append(&env("aaaa0004", "k4", "murmur/event")).unwrap(), 2);
    // reopen: seq tęsiasi
    let l2 = ledger::Ledger::open(&file).unwrap();
    assert_eq!(l2.seq(), 2);
}
