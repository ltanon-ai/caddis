//! packet.rs — CARD-0136. Hermetic HOME. Never ~/.claude.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);
const KEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

fn tmp(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("caddis-pkt-{tag}-{n}"));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

struct World {
    home: PathBuf,
}

impl World {
    fn new(tag: &str) -> Self {
        let root = tmp(tag);
        let home = root.join("home");
        fs::create_dir_all(&home).unwrap();
        fs::write(home.join("empty-drain"), "").unwrap();
        Self { home }
    }

    fn run(&self, args: &[&str]) -> (String, String, i32) {
        let out = Command::new(env!("CARGO_BIN_EXE_caddis"))
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("CADDIS_HMAC_KEY", KEY)
            .env("CADDIS_DRAIN_HERDR", self.home.join("empty-drain"))
            .output()
            .expect("caddis must spawn");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    }
}

#[test]
fn packet_prints_this_lineage_arm() {
    let w = World::new("pkt");
    let (o, e, c) = w.run(&[
        "rotate",
        "ready",
        "--lineage",
        "line-a",
        "--kind",
        "omp",
        "--model",
        "ma",
        "--pane",
        "w3J:p1",
    ]);
    assert_eq!(c, 0, "ready: {o}{e}");
    let (o, e, c) = w.run(&["rotate", "arm", "--lineage", "line-a"]);
    assert_eq!(c, 0, "arm: {o}{e}");
    let (o, e, c) = w.run(&["lineage", "packet", "--lineage", "line-a"]);
    assert_eq!(c, 0, "packet: {o}{e}");
    assert!(o.contains("LINEAGE line-a"), "{o}");
    assert!(o.contains("kind=omp"), "{o}");
    assert!(o.contains("model=ma"), "{o}");
    assert!(o.contains("pane=w3J:p1"), "{o}");
    assert!(o.contains("fold_at=50"), "{o}");
    assert!(o.contains("fold=quiet"), "{o}");
}

#[test]
fn packet_without_arm_fails() {
    let w = World::new("noarm");
    let (o, e, c) = w.run(&["lineage", "packet", "--lineage", "line-a"]);
    assert_ne!(c, 0, "missing ARM must fail: {o}{e}");
}
