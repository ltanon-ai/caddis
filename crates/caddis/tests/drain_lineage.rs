//! drain_lineage.rs — CARD-0133. Hermetic HOME. Never ~/.herdr or ~/.claude.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);
const KEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

fn tmp(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("caddis-lin-{tag}-{n}"));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

struct World {
    home: PathBuf,
    snap: PathBuf,
}

impl World {
    fn new(tag: &str) -> Self {
        let root = tmp(tag);
        let home = root.join("home");
        fs::create_dir_all(&home).unwrap();
        let snap = root.join("snapshot.json");
        fs::write(&snap, "{}").unwrap();
        Self { home, snap }
    }

    fn run(&self, args: &[&str]) -> (String, String, i32) {
        let mut argv: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
        let sub = argv.get(1).map(|s| s.as_str()).unwrap_or("");
        if matches!(sub, "ready" | "arm" | "verify") && !argv.iter().any(|s| s == "--lineage") {
            argv.push("--lineage".into());
            argv.push("lin-t".into());
        }
        let out = Command::new(env!("CARGO_BIN_EXE_caddis"))
            .args(&argv)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("CADDIS_HMAC_KEY", KEY)
            .env("CADDIS_HERDR_SNAPSHOT", &self.snap)
            .output()
            .expect("caddis must spawn");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    }

    fn arm_pane(&self, pane: &str) {
        let (o, e, c) = self.run(&[
            "rotate",
            "ready",
            "--kind",
            "omp",
            "--model",
            "m1",
            "--pane",
            pane,
        ]);
        assert_eq!(c, 0, "ready: {o}{e}");
        let (o, e, c) = self.run(&["rotate", "arm"]);
        assert_eq!(c, 0, "arm: {o}{e}");
    }
}

fn snap_working(pane: &str) -> String {
    format!(
        r#"{{"agents":[{{"pane_id":"{pane}","agent_status":"working"}}]}}"#
    )
}

#[test]
fn other_pane_working_does_not_fail_this_rotation() {
    let w = World::new("other");
    fs::write(&w.snap, snap_working("w36:pP")).unwrap();
    w.arm_pane("w3J:p1");
    let (o, e, c) = w.run(&["rotate", "verify"]);
    assert_eq!(c, 0, "other pane must not fail this lineage: {o}{e}");
}

#[test]
fn arm_pane_working_still_fails_drain() {
    let w = World::new("mine");
    fs::write(&w.snap, snap_working("w3J:p1")).unwrap();
    w.arm_pane("w3J:p1");
    let (o, e, c) = w.run(&["rotate", "verify"]);
    assert_ne!(c, 0, "this pane working must fail drain: {o}{e}");
}
