//! card_bee.rs — CARD-0131. Bee lanes refuse v1 (no EXECUTION) cards.

use std::process::{Command, Stdio};

const V1: &str = "---\nid: CARD-TEST-1\nclass: fix\nowner: t\n---\n\
# a test card\n\n# Done-When\n- the test passes\n\n# RED-TEST\nit failed before\n";

const STRICT: &str = "---\nid: CARD-TEST-2\nclass: fix\nowner: t\n---\n\
# a test card\n\n# Done-When\n- the test passes\n\n# RED-TEST\nit failed before\n\n\
# EXECUTION\n\nlevel: L1\nblast: 1\nclaims-forbidden: true\n\
anchors:\n  - path: a\n    content: |\n      x\nallowlist:\n  - edit a\n";

fn tmp(tag: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!(
        "caddis-bee-{}-{:?}",
        tag,
        std::thread::current().id()
    ));
    p
}

fn card(ledger: &std::path::Path, from: &str, args: &[&str]) -> (String, String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_caddis-warden"))
        .arg("card")
        .args(args)
        .env("CADDIS_WARDEN_LEDGER", ledger)
        .env("CADDIS_WARDEN_FROM", from)
        .stdin(Stdio::null())
        .output()
        .expect("binary must spawn");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

fn write_pair(tag: &str, text: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let ledger = tmp(&format!("led-{tag}")).with_extension("jsonl");
    let card_path = tmp(&format!("card-{tag}")).with_extension("md");
    let _ = std::fs::remove_file(&ledger);
    std::fs::write(&card_path, text).unwrap();
    (ledger, card_path)
}

#[test]
fn bee_cannot_open_v1_card() {
    let (led, path) = write_pair("v1", V1);
    let arg = path.to_string_lossy().into_owned();
    let (o, e, c) = card(&led, "little-coder.aaaaaaaa", &["open", &arg]);
    assert_ne!(c, 0, "bee v1 must refuse: {o}{e}");
    assert!(e.contains("EXECUTION"), "name the missing contract: {e}");
    let _ = std::fs::remove_file(&led);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn omp_can_still_open_v1_card() {
    let (led, path) = write_pair("omp", V1);
    let arg = path.to_string_lossy().into_owned();
    let (o, e, c) = card(&led, "peleda.aaaaaaaa", &["open", &arg]);
    assert_eq!(c, 0, "omp v1 must still open: {o}{e}");
    assert!(o.contains("NOT BOUNDED"), "{o}");
    let _ = std::fs::remove_file(&led);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn bee_can_open_strict_card() {
    let (led, path) = write_pair("st", STRICT);
    let arg = path.to_string_lossy().into_owned();
    let (o, e, c) = card(&led, "droid.aaaaaaaa", &["open", &arg]);
    assert_eq!(c, 0, "bee strict must open: {o}{e}");
    let _ = std::fs::remove_file(&led);
    let _ = std::fs::remove_file(&path);
}
