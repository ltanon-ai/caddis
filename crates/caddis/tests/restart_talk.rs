//! restart_talk.rs — CARD-0306. The talk organ + heartbeat + promote
//! hygiene (drill gaps G1-G3). Hermetic World (restart_enter harness).

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);
const KEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

fn tmp(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("caddis-talk-{tag}-{n}"));
    let _ = fs::remove_dir_all(&p); // swallow: best-effort-cleanup — stale temp dir from a prior run
    fs::create_dir_all(&p).unwrap();
    p
}

struct World {
    home: PathBuf,
    line: PathBuf,
}

impl World {
    fn new(tag: &str) -> Self {
        let root = tmp(tag);
        let home = root.join("home");
        fs::create_dir_all(&home).unwrap();
        Self {
            line: home.join(".caddis/rotation/lines/lin-t"),
            home,
        }
    }

    fn run(&self, args: &[&str], pane: Option<&str>) -> (String, String, i32) {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_caddis"));
        cmd.args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("CADDIS_HMAC_KEY", KEY)
            .env("CADDIS_SKIP_WARDEN", "1")
            .env_remove("HERDR_PANE_ID");
        if let Some(p) = pane {
            cmd.env("HERDR_PANE_ID", p);
        }
        let out = cmd.output().expect("caddis must spawn");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    }

    fn seed(&self) {
        let (o, e, c) = self.run(
            &[
                "rotate",
                "ready",
                "--kind",
                "omp",
                "--model",
                "m1",
                "--lineage",
                "lin-t",
            ],
            None,
        );
        assert_eq!(c, 0, "ready: {o}{e}");
        let (o, e, c) = self.run(&["rotate", "arm", "--lineage", "lin-t"], None);
        assert_eq!(c, 0, "arm: {o}{e}");
    }

    fn talk_path(&self) -> PathBuf {
        self.line.join("talk").join("turns.jsonl")
    }
}

/// A stamped turn round-trips through post -> read (G3: mechanical).
#[test]
fn post_and_read_roundtrip() {
    let w = World::new("rt");
    w.seed();
    let (o, e, c) = w.run(
        &[
            "restart",
            "talk",
            "--lineage",
            "lin-t",
            "--post",
            "finding",
            "drain treats idle as gone E:/tmp/log.txt",
        ],
        Some("w1:p2"),
    );
    assert_eq!(c, 0, "post: {o}{e}");
    let raw = fs::read_to_string(w.talk_path()).expect("turns.jsonl written");
    assert!(raw.contains("\"kind\":\"finding\""), "kind recorded: {raw}");
    assert!(raw.contains("w1:p2"), "role pane recorded: {raw}");
    let (o, _e, _c) = w.run(&["restart", "talk", "--lineage", "lin-t", "--read"], None);
    assert!(o.contains("finding"), "read shows the turn: {o}");
}

/// answer|fix without an evidence path is refused (E6: receipts, not prose).
#[test]
fn answer_without_evidence_path_fails() {
    let w = World::new("noev");
    w.seed();
    let (o, e, c) = w.run(
        &[
            "restart",
            "talk",
            "--lineage",
            "lin-t",
            "--post",
            "answer",
            "it works now trust me",
        ],
        Some("w1:p1"),
    );
    assert_ne!(c, 0, "answer without evidence must fail: {o}{e}");
}

/// heartbeat writes the receipt and clears the armed-never-woke marker (G1+G2).
#[test]
fn heartbeat_writes_and_clears_armed_never_woke() {
    let w = World::new("hb");
    w.seed();
    fs::create_dir_all(w.line.join("talk")).unwrap();
    fs::write(w.line.join("armed-never-woke.lease"), "pane=w1:p9\nts=1\n").unwrap();
    let (o, e, c) = w.run(
        &["restart", "heartbeat", "--lineage", "lin-t"],
        Some("w1:p9"),
    );
    assert_eq!(c, 0, "heartbeat: {o}{e}");
    assert!(
        w.line.join("heartbeat.receipt").is_file(),
        "receipt written"
    );
    assert!(
        !w.line.join("armed-never-woke.lease").exists(),
        "marker cleared on heartbeat"
    );
}

/// enter names an unanswered finding (retire-gate convention, read-only).
#[test]
fn unanswered_finding_named_by_enter() {
    let w = World::new("gate");
    w.seed();
    let (o, e, c) = w.run(
        &[
            "restart",
            "talk",
            "--lineage",
            "lin-t",
            "--post",
            "finding",
            "keeper dead E:/tmp/bee.log",
        ],
        Some("w1:p2"),
    );
    assert_eq!(c, 0, "post: {o}{e}");
    let (o, e, c) = w.run(&["restart", "enter", "--lineage", "lin-t"], None);
    assert_eq!(c, 0, "enter: {o}{e}");
    assert!(o.contains("unanswered"), "names the open finding: {o}");
}
