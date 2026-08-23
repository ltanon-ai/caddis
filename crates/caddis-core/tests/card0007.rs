//! CARD-0007 (L-02): FOOTER-STATE testai.
#[test]
fn dw1_snapshot_reads_ledger_seq() {
    let dir = std::env::temp_dir().join(format!("fs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("ledger.jsonl");
    let mut l = caddis_core::ledger::Ledger::open(&p).unwrap();
    let env = caddis_core::envelope::validate(
        1,
        "aaaa0007x",
        "k7",
        "signal/node.state",
        "test",
        "ledger",
        "{}",
        "t",
    )
    .unwrap();
    l.append(&env).unwrap();
    let fs = caddis_core::footer_state::FooterState::snapshot(&p);
    assert_eq!(fs.ledger_seq, 1);
    assert_eq!(fs.organs.len(), 5);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dw2_render_plain() {
    let fs = caddis_core::footer_state::FooterState::default();
    let s = fs.render_plain();
    assert!(
        s.contains("warden") && s.contains("organs") && s.contains("ledger"),
        "plain form: {s}"
    );
}
