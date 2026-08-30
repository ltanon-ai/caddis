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
fn first_tick_at_threshold_warns_second_holds_without_era() {
    let w = World::new("tick");
    let (o, e, c) = w.run(&["fold", "threshold", "--at", "40"]);
    assert_eq!(c, 0, "threshold: {o}{e}");
    let tick = ["fold", "tick", "--lineage", "line-a", "--used-pct", "40"];
    let (o, e, c) = w.run(&tick);
    assert_eq!(c, 0, "first tick warn: {o}{e}");
    assert!(o.contains("FOLD warn"), "warn stdout: {o}");
    let (o, e, c) = w.run(&tick);
    assert_eq!(c, 0, "second tick hold without era: {o}{e}");
    assert!(o.contains("FOLD hold"), "hold stdout: {o}");
}

#[test]
fn second_tick_denies_only_after_era_open() {
    let w = World::new("era");
    let _ = w.run(&["fold", "threshold", "--at", "40"]);
    let tick = ["fold", "tick", "--lineage", "line-a", "--used-pct", "40"];
    let (o, e, c) = w.run(&tick);
    assert_eq!(c, 0, "warn: {o}{e}");
    let era = w
        .home
        .join(".caddis")
        .join("pager")
        .join("line-a")
        .join("era");
    fs::create_dir_all(era.parent().unwrap()).unwrap();
    fs::write(&era, "open=1\nlast_task=CARD-0210\n").unwrap();
    let (o, e, c) = w.run(&tick);
    assert_eq!(c, 1, "deny after era: {o}{e}");
    assert!(o.contains("FOLD deny"), "deny stdout: {o}");
}

#[test]
fn tokens_at_cap_are_over_below_percent() {
    let w = World::new("tok");
    let _ = w.run(&["fold", "threshold", "--at", "90"]);
    let (o, e, c) = w.run(&["fold", "cap", "--lineage", "line-a", "--tokens", "100000"]);
    assert_eq!(c, 0, "cap: {o}{e}");
    let (o, e, c) = w.run(&[
        "fold",
        "tick",
        "--lineage",
        "line-a",
        "--used-pct",
        "21",
        "--used-tokens",
        "105000",
    ]);
    assert_eq!(c, 0, "token cap warns: {o}{e}");
    assert!(o.contains("FOLD warn"), "warn stdout: {o}");
}

#[test]
fn missing_cap_file_defaults_to_170k() {
    let w = World::new("defcap");
    let _ = w.run(&["fold", "threshold", "--at", "90"]);
    let (o, e, c) = w.run(&[
        "fold",
        "tick",
        "--lineage",
        "line-a",
        "--used-pct",
        "21",
        "--used-tokens",
        "170000",
    ]);
    assert_eq!(c, 0, "default cap warns: {o}{e}");
    assert!(o.contains("FOLD warn"), "warn stdout: {o}");
}

#[test]
fn zero_cap_is_uncapped() {
    let w = World::new("uncap");
    let _ = w.run(&["fold", "threshold", "--at", "90"]);
    let (o, e, c) = w.run(&["fold", "cap", "--lineage", "line-a", "--tokens", "0"]);
    assert_eq!(c, 0, "cap 0: {o}{e}");
    let (o, e, c) = w.run(&[
        "fold",
        "tick",
        "--lineage",
        "line-a",
        "--used-pct",
        "21",
        "--used-tokens",
        "999999",
    ]);
    assert_eq!(c, 0, "uncapped quiet: {o}{e}");
    assert!(o.contains("FOLD quiet"), "quiet stdout: {o}");
}

#[test]
fn claude_kind_warns_at_30_percent() {
    let w = World::new("claude30");
    let _ = w.run(&["fold", "threshold", "--at", "90"]);
    let dir = w
        .home
        .join(".caddis")
        .join("rotation")
        .join("lines")
        .join("line-a");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("arm.receipt"),
        "kind=claude\nmodel=x\nlineage=line-a\n---\nxx\n",
    )
    .unwrap();
    let (o, e, c) = w.run(&["fold", "tick", "--lineage", "line-a", "--used-pct", "31"]);
    assert_eq!(c, 0, "claude 31 warns: {o}{e}");
    assert!(o.contains("FOLD warn"), "warn stdout: {o}");
}

#[test]
fn claude_kind_has_no_default_token_cap() {
    let w = World::new("claudetok");
    let _ = w.run(&["fold", "threshold", "--at", "90"]);
    let dir = w
        .home
        .join(".caddis")
        .join("rotation")
        .join("lines")
        .join("line-a");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("arm.receipt"),
        "kind=claude\nmodel=x\nlineage=line-a\n---\nxx\n",
    )
    .unwrap();
    let (o, e, c) = w.run(&[
        "fold",
        "tick",
        "--lineage",
        "line-a",
        "--used-pct",
        "29",
        "--used-tokens",
        "999999",
    ]);
    assert_eq!(c, 0, "claude under 30 quiet: {o}{e}");
    assert!(o.contains("FOLD quiet"), "quiet stdout: {o}");
}

#[test]
fn below_threshold_is_quiet_without_threshold_file() {
    let w = World::new("quiet");
    let (o, e, c) = w.run(&["fold", "tick", "--lineage", "line-a", "--used-pct", "49"]);
    assert_eq!(c, 0, "quiet: {o}{e}");
    assert!(o.contains("FOLD quiet"), "quiet stdout: {o}");
}
