//! fold.rs — CARD-0135. Hermetic HOME. Never ~/.claude.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn tmp(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("caddis-fold-{tag}-{n}"));
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
        let out = Command::new(env!("CARGO_BIN_EXE_caddis"))
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
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
fn first_tick_at_threshold_warns_second_denies() {
    let w = World::new("tick");
    let (o, e, c) = w.run(&["fold", "threshold", "--at", "40"]);
    assert_eq!(c, 0, "threshold: {o}{e}");
    let at = fs::read_to_string(w.home.join(".caddis").join("fold.at")).unwrap();
    assert_eq!(at.trim(), "40", "fold.at: {at}");
    let tick = [
        "fold", "tick", "--lineage", "line-a", "--used-pct", "40",
    ];
    let (o, e, c) = w.run(&tick);
    assert_eq!(c, 0, "first tick warn: {o}{e}");
    assert!(o.contains("FOLD warn"), "warn stdout: {o}");
    let (o, e, c) = w.run(&tick);
    assert_eq!(c, 1, "second tick deny: {o}{e}");
    assert!(o.contains("FOLD deny"), "deny stdout: {o}");
}

#[test]
fn below_threshold_is_quiet_without_threshold_file() {
    let w = World::new("quiet");
    let (o, e, c) = w.run(&[
        "fold", "tick", "--lineage", "line-a", "--used-pct", "49",
    ]);
    assert_eq!(c, 0, "quiet: {o}{e}");
    assert!(o.contains("FOLD quiet"), "quiet stdout: {o}");
}
