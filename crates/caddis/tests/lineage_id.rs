//! lineage_id.rs — CARD-0134. Hermetic HOME. Never ~/.claude.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);
const KEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

fn tmp(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("caddis-lid-{tag}-{n}"));
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
        Self { home }
    }

    fn run(&self, args: &[&str]) -> (String, String, i32) {
        let drain = self.home.join("empty-drain");
        if !drain.is_file() {
            fs::write(&drain, "").unwrap();
        }
        let out = Command::new(env!("CARGO_BIN_EXE_caddis"))
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("CADDIS_HMAC_KEY", KEY)
            .env("CADDIS_DRAIN_HERDR", &drain)
            .output()
            .expect("caddis must spawn");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    }

    fn lines(&self, id: &str) -> PathBuf {
        self.home
            .join(".caddis")
            .join("rotation")
            .join("lines")
            .join(id)
    }
}

#[test]
fn ready_without_lineage_is_usage() {
    let w = World::new("no-lin");
    fs::write(w.home.join("empty-drain"), "").unwrap();
    let (o, e, c) = w.run(&["rotate", "ready", "--kind", "omp", "--model", "m1"]);
    assert_eq!(c, 2, "ready without --lineage is usage: {o}{e}");
}

#[test]
fn two_lineages_cannot_clobber_each_others_arm() {
    let w = World::new("two");
    fs::write(w.home.join("empty-drain"), "").unwrap();
    let ready_a = [
        "rotate",
        "ready",
        "--kind",
        "omp",
        "--model",
        "ma",
        "--lineage",
        "line-a",
    ];
    let ready_b = [
        "rotate",
        "ready",
        "--kind",
        "omp",
        "--model",
        "mb",
        "--lineage",
        "line-b",
    ];
    let (o, e, c) = w.run(&ready_a);
    assert_eq!(c, 0, "ready a: {o}{e}");
    let (o, e, c) = w.run(&["rotate", "arm", "--lineage", "line-a"]);
    assert_eq!(c, 0, "arm a: {o}{e}");
    let (o, e, c) = w.run(&ready_b);
    assert_eq!(c, 0, "ready b: {o}{e}");
    let (o, e, c) = w.run(&["rotate", "arm", "--lineage", "line-b"]);
    assert_eq!(c, 0, "arm b: {o}{e}");
    let a_arm = fs::read_to_string(w.lines("line-a").join("arm.receipt")).unwrap();
    assert!(
        a_arm.contains("model=ma") && a_arm.contains("lineage=line-a"),
        "A's ARM must survive B: {a_arm}"
    );
    let (o, e, c) = w.run(&["rotate", "verify", "--lineage", "line-a"]);
    assert_eq!(c, 0, "verify A after B armed: {o}{e}");
    assert!(
        o.contains("LINEAGE line-a"),
        "verify must print lineage: {o}"
    );
}
