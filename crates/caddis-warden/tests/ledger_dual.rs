//! ledger_dual.rs — CARD-0321. The stores are never forked: a
//! production-path write lands in BOTH the readers' JSONL and the
//! sqlite query store; either store failing refuses the row. Hermetic
//! temp HOME — the operator's real stores are never touched.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn tmp(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "caddis-dual-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = fs::remove_dir_all(&p); // swallow: best-effort-cleanup
    fs::create_dir_all(&p).unwrap();
    p
}

struct World {
    home: PathBuf,
}

impl World {
    fn new(tag: &str) -> Self {
        let home = tmp(tag);
        Self { home }
    }

    /// Production-path card open: NO CADDIS_WARDEN_LEDGER — the writer
    /// must resolve the real default JSONL under this temp HOME.
    fn card_open(&self, card: &str) -> (String, String, i32) {
        let out = Command::new(env!("CARGO_BIN_EXE_caddis-warden"))
            .args(["card", "open", card])
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env_remove("CADDIS_WARDEN_LEDGER")
            .env("CADDIS_WARDEN_FROM", "dual.rs")
            .stdin(Stdio::null())
            .output()
            .expect("the binary must spawn");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    }

    fn card(&self, args: &[&str], ledger: &str) -> (String, String, i32) {
        let out = Command::new(env!("CARGO_BIN_EXE_caddis-warden"))
            .arg("card")
            .args(args)
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("CADDIS_WARDEN_LEDGER", ledger)
            .env("CADDIS_WARDEN_FROM", "dual.rs")
            .stdin(Stdio::null())
            .output()
            .expect("the binary must spawn");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    }

    fn jsonl(&self) -> PathBuf {
        self.home.join(".caddis/warden-ledger.jsonl")
    }

    fn sqlite(&self) -> PathBuf {
        self.home.join(".caddis/ledger.sqlite")
    }
}

const CARD: &str = "---\nid: CARD-DUAL-1\nclass: fix\nowner: t\n---\n\
# dual-store probe\n\n# Done-When\n- the test passes\n\n# RED-TEST\nit failed before\n";

/// Production-path `card open` lands in BOTH stores: the JSONL (every
/// reader's store) grows by the open row AND sqlite takes the INSERT.
#[test]
fn dual_write_lands_both_stores() {
    let w = World::new("both");
    let card = w.home.join("card.md");
    fs::write(&card, CARD).unwrap();
    let (o, e, c) = w.card_open(card.to_str().unwrap());
    assert_eq!(c, 0, "card open: {o}{e}");
    let jsonl = fs::read_to_string(w.jsonl()).expect("JSONL grew (readers' store)");
    assert!(jsonl.contains("card.open"), "open row in JSONL: {jsonl}");
    assert!(w.sqlite().is_file(), "sqlite store exists");
}

/// A JSONL append that cannot happen refuses the row — never a
/// sqlite-only success that every reader is blind to.
#[test]
fn jsonl_append_failure_refuses_the_row() {
    let w = World::new("fail");
    let card = w.home.join("card.md");
    fs::write(&card, CARD).unwrap();
    // the "ledger path" is a DIRECTORY: open/append must fail loudly
    let dir_ledger = w.home.join("not-a-file.jsonl");
    fs::create_dir_all(&dir_ledger).unwrap();
    let (o, e, c) = w.card(
        &["open", card.to_str().unwrap()],
        dir_ledger.to_str().unwrap(),
    );
    assert_ne!(c, 0, "a refused store refuses the row: {o}{e}");
    assert!(
        e.to_lowercase().contains("cannot read")
            || e.to_lowercase().contains("ledger")
            || e.to_lowercase().contains("append"),
        "names the failing store: {e}"
    );
}
