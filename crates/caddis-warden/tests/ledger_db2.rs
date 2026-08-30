//! ledger_db2.rs — CARD-LEDGER-DB-2. Hermetic. Never ~/.caddis live bag.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_caddis-warden"))
}

const CARD: &str = "---\nid: CARD-DB2-1\nclass: fix\nowner: t\n---\n\
# db2\n\n# Done-When\n- x\n\n# RED-TEST\nit failed before\n";

#[test]
fn production_card_open_lands_both_stores() {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let home = std::env::temp_dir().join(format!("caddis-db2-{stamp}"));
    fs::create_dir_all(home.join(".caddis")).unwrap();
    let jsonl = home.join(".caddis").join("warden-ledger.jsonl");
    let sqlite = home.join(".caddis").join("ledger.sqlite");
    fs::write(&jsonl, "{\"seq\":1}\n").unwrap();
    let before = fs::read(&jsonl).unwrap();
    let card = home.join("t.md");
    fs::write(&card, CARD).unwrap();

    let out = Command::new(bin())
        .args(["card", "open"])
        .arg(&card)
        .env("USERPROFILE", &home)
        .env("HOME", &home)
        .env_remove("CADDIS_WARDEN_LEDGER")
        .env("CADDIS_WARDEN_FROM", "peleda.aaaaaaaa")
        .stdin(Stdio::null())
        .output()
        .expect("spawn");
    assert_eq!(
        out.status.code(),
        Some(0),
        "open failed: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let after = fs::read_to_string(&jsonl).unwrap();
    assert!(
        after.len() > before.len(),
        "CARD-0321: production JSONL takes the row (readers' store)"
    );
    assert!(
        after.contains("card.open"),
        "the open row is in JSONL: {after}"
    );
    assert!(sqlite.exists(), "sqlite missing after production card open");
    let n = Command::new("python")
        .args([
            "-c",
            "import sqlite3,sys; print(sqlite3.connect(sys.argv[1]).execute('select count(*) from verdicts').fetchone()[0])",
        ])
        .arg(&sqlite)
        .output()
        .expect("python count");
    assert!(n.status.success(), "{}", String::from_utf8_lossy(&n.stderr));
    let count: i64 = String::from_utf8_lossy(&n.stdout).trim().parse().unwrap();
    assert!(count >= 1, "sqlite COUNT was {count}");
}
