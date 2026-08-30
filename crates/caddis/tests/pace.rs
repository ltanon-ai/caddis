//! pace.rs — CARD-0214. Hermetic HOME. Never ~/.caddis live bag.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

const TEST_KEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

fn tmp(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!("caddis-pace-{}-{n}-{tag}", std::process::id()));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

struct World {
    home: PathBuf,
    rot: PathBuf,
    herdr_fixture: PathBuf,
    claude_fixture: PathBuf,
    qpi_fixture: PathBuf,
}

impl World {
    fn new(tag: &str) -> Self {
        let root = tmp(tag);
        let home = root.join("home");
        fs::create_dir_all(&home).unwrap();
        let rot = home.join(".caddis").join("rotation");
        let herdr_fixture = root.join("herdr.json");
        let claude_fixture = root.join("claude-reg.json");
        let qpi_fixture = root.join("qpi.json");
        for f in [&herdr_fixture, &claude_fixture, &qpi_fixture] {
            fs::write(f, "").unwrap();
        }
        Self {
            home,
            rot,
            herdr_fixture,
            claude_fixture,
            qpi_fixture,
        }
    }

    fn run(&self, args: &[&str]) -> (String, String, i32) {
        self.run_with(args, true)
    }

    fn run_unknown(&self, args: &[&str]) -> (String, String, i32) {
        self.run_with(args, false)
    }

    fn run_with(&self, args: &[&str], set_drain: bool) -> (String, String, i32) {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_caddis"));
        cmd.args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("CADDIS_HMAC_KEY", TEST_KEY);
        if set_drain {
            cmd.env("CADDIS_DRAIN_HERDR", &self.herdr_fixture)
                .env("CADDIS_DRAIN_CLAUDE_REGISTRY", &self.claude_fixture)
                .env("CADDIS_DRAIN_QPI", &self.qpi_fixture);
        } else {
            cmd.env(
                "CADDIS_HERDR_SNAPSHOT",
                self.home.join("missing-herdr.json"),
            );
        }
        let out = cmd.output().expect("caddis must spawn");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    }

    fn arm(&self) {
        let ready = [
            "rotate",
            "ready",
            "--kind",
            "omp",
            "--model",
            "m1",
            "--lineage",
            "line-a",
        ];
        let (o, e, c) = self.run(&ready);
        assert_eq!(c, 0, "ready: {o}{e}");
        let (o, e, c) = self.run(&["rotate", "arm", "--lineage", "line-a"]);
        assert_eq!(c, 0, "arm: {o}{e}");
    }

    fn queue(&self, body: &str) {
        let dir = self.rot.join("lines").join("line-a");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("queue"), body).unwrap();
    }

    fn arm_file(&self) -> PathBuf {
        self.rot.join("lines").join("line-a").join("arm.receipt")
    }

    fn pace_line(&self) -> PathBuf {
        self.rot.join("lines").join("line-a").join("pace.line")
    }
}

#[test]
fn check_without_lineage_is_usage() {
    let w = World::new("nousage");
    let (o, e, c) = w.run(&["check"]);
    assert_eq!(c, 2, "check requires --lineage: {o}{e}");
}

#[test]
fn empty_queue_is_idle_ok_no_force() {
    let w = World::new("empty");
    w.arm();
    let (o, e, c) = w.run(&["check", "--lineage", "line-a"]);
    assert_eq!(c, 0, "check empty: {o}{e}");
    assert!(o.contains("PACE IDLE-OK"), "empty must be IDLE-OK: {o}{e}");
    assert!(!o.contains("PACE WORK"), "empty queue is no force: {o}");
}

#[test]
fn named_card_and_idle_chair_is_work() {
    let w = World::new("work");
    w.arm();
    w.queue("CARD-0214\n");
    let (o, e, c) = w.run(&["check", "--lineage", "line-a"]);
    assert_eq!(c, 0, "check work: {o}{e}");
    assert!(
        o.contains("PACE WORK CARD-0214"),
        "idle + card licenses WORK: {o}{e}"
    );
    let line = fs::read_to_string(w.pace_line()).unwrap();
    assert!(line.contains("sentence=PACE WORK CARD-0214"), "{line}");
    assert!(line.contains("---\n"), "HMAC-stamped: {line}");
}

#[test]
fn live_chair_is_busy_not_work() {
    let w = World::new("busy");
    w.arm();
    w.queue("CARD-0214\n");
    fs::write(&w.herdr_fixture, r#"{"status": "live"}"#).unwrap();
    let (o, e, c) = w.run(&["check", "--lineage", "line-a"]);
    assert_eq!(c, 0, "check busy: {o}{e}");
    assert!(o.contains("PACE BUSY"), "live chair is BUSY: {o}{e}");
    assert!(!o.contains("PACE WORK"), "must not force a live chair: {o}");
}

#[test]
fn pace_stop_never_works() {
    let w = World::new("stop");
    w.arm();
    w.queue("CARD-0214\n");
    let (o, e, c) = w.run(&["check", "--lineage", "line-a", "--pace", "stop"]);
    assert_eq!(c, 0, "check stop: {o}{e}");
    assert!(o.contains("PACE STOP"), "stop: {o}{e}");
    assert!(!o.contains("PACE WORK"), "stop is no force: {o}");
    let arm = fs::read_to_string(w.arm_file()).unwrap();
    assert!(arm.contains("pace=stop"), "ARM field: {arm}");
}

#[test]
fn ready_receipt_carries_pace_run() {
    let w = World::new("runfield");
    w.arm();
    let arm = fs::read_to_string(w.arm_file()).unwrap();
    assert!(arm.contains("pace=run"), "default ARM pace=run: {arm}");
}

#[test]
fn done_card_is_skipped() {
    let w = World::new("done");
    w.arm();
    w.queue("done CARD-0214\nCARD-0215\n");
    let (o, e, c) = w.run(&["check", "--lineage", "line-a"]);
    assert_eq!(c, 0, "check done-skip: {o}{e}");
    assert!(o.contains("PACE WORK CARD-0215"), "next named card: {o}{e}");
}

#[test]
fn fold_tick_feeds_only_frozen_sentence() {
    let w = World::new("foldfeed");
    w.arm();
    w.queue("CARD-0214\n");
    let (o, e, c) = w.run(&["check", "--lineage", "line-a"]);
    assert_eq!(c, 0, "check: {o}{e}");
    let tick = ["fold", "tick", "--lineage", "line-a", "--used-pct", "21"];
    let (o, e, c) = w.run(&tick);
    assert_eq!(c, 0, "tick: {o}{e}");
    assert!(o.contains("FOLD quiet"), "tick quiet: {o}");
    assert!(
        o.contains("PACE WORK CARD-0214"),
        "fold feeds frozen sentence: {o}"
    );
}

#[test]
fn unknown_drain_with_card_is_work() {
    let w = World::new("unknown");
    w.arm();
    w.queue("CARD-0215\n");
    let (o, e, c) = w.run_unknown(&["check", "--lineage", "line-a"]);
    assert_eq!(c, 0, "unknown check: {o}{e}");
    assert!(
        o.contains("PACE WORK CARD-0215"),
        "Unknown drain must not freeze: {o}{e}"
    );
    assert!(!o.contains("PACE BUSY"), "Unknown is not BUSY: {o}");
}

#[test]
fn fold_tick_beats_without_prior_check() {
    let w = World::new("autobeat");
    w.arm();
    w.queue("CARD-0215\n");
    let tick = ["fold", "tick", "--lineage", "line-a", "--used-pct", "21"];
    let (o, e, c) = w.run(&tick);
    assert_eq!(c, 0, "tick: {o}{e}");
    assert!(
        o.contains("PACE WORK CARD-0215"),
        "fold tick must beat pace: {o}{e}"
    );
}

#[test]
fn tampered_pace_line_without_arm_is_not_fed() {
    let w = World::new("tamper");
    let dir = w.rot.join("lines").join("line-a");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("pace.line"),
        "sentence=PACE WORK FORGED\n---\n00\n",
    )
    .unwrap();
    let tick = ["fold", "tick", "--lineage", "line-a", "--used-pct", "21"];
    let (o, e, c) = w.run(&tick);
    assert_eq!(c, 0, "tick: {o}{e}");
    assert!(!o.contains("PACE WORK"), "forged line must not feed: {o}");
}

#[test]
fn fold_empty_queue_prints_no_work() {
    let w = World::new("nofeed");
    w.arm();
    let tick = ["fold", "tick", "--lineage", "line-a", "--used-pct", "21"];
    let (o, e, c) = w.run(&tick);
    assert_eq!(c, 0, "tick: {o}{e}");
    assert!(!o.contains("PACE WORK"), "empty queue is no force: {o}");
}
