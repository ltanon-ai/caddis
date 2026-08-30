//! doctor_scan.rs — CARD-0307. The doctor: find, fix the safe set,
//! escalate the rest. Hermetic World.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);
const KEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

fn tmp(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("caddis-doc-{tag}-{n}"));
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

    fn run(&self, args: &[&str], extra: &[(&str, &str)]) -> (String, String, i32) {
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
        for (k, v) in extra {
            cmd.env(k, v);
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
            &[],
        );
        assert_eq!(c, 0, "ready: {o}{e}");
        let (o, e, c) = self.run(&["rotate", "arm", "--lineage", "lin-t"], &[]);
        assert_eq!(c, 0, "arm: {o}{e}");
    }

    fn turns(&self) -> String {
        fs::read_to_string(self.line.join("talk/turns.jsonl")).unwrap_or_default()
    }

    fn post_finding(&self) {
        let (o, e, c) = self.run(
            &[
                "restart",
                "talk",
                "--lineage",
                "lin-t",
                "--post",
                "finding",
                "keeper dead E:/tmp/bee.log",
            ],
            &[],
        );
        assert_eq!(c, 0, "post: {o}{e}");
    }
}

/// S1: a stale marker WITH a heartbeat present is fixed, evidence-logged.
#[test]
fn stale_marker_with_heartbeat_is_fixed() {
    let w = World::new("fix");
    w.seed();
    fs::write(w.line.join("armed-never-woke.lease"), "pane=w1:p9\nts=1\n").unwrap();
    fs::write(w.line.join("heartbeat.receipt"), "pane=w1:p9\nts=2\n").unwrap();
    let (o, e, c) = w.run(&["doctor", "--lineage", "lin-t"], &[]);
    assert_eq!(c, 0, "doctor: {o}{e}");
    assert!(
        !w.line.join("armed-never-woke.lease").exists(),
        "marker removed"
    );
    let turns = w.turns();
    assert!(
        turns.contains("\"kind\":\"fix\""),
        "fix turn logged: {turns}"
    );
    assert!(
        turns.contains("armed-never-woke.lease"),
        "fix turn carries the evidence path: {turns}"
    );
}

/// S2: unanswered findings escalate — the doctor never answers them.
#[test]
fn unanswered_finding_escalated_not_answered() {
    let w = World::new("esc");
    w.seed();
    w.post_finding();
    let (o, e, c) = w.run(&["doctor", "--lineage", "lin-t"], &[]);
    assert_eq!(c, 0, "doctor: {o}{e}");
    assert!(o.contains("escalate"), "names the escalation: {o}");
    let turns = w.turns();
    assert!(
        turns.contains("\"kind\":\"escalate\""),
        "escalate turn logged: {turns}"
    );
    assert!(
        !turns.contains("\"kind\":\"answer\""),
        "the doctor never answers findings: {turns}"
    );
    assert!(
        turns.contains("\"kind\":\"finding\""),
        "the finding stands: {turns}"
    );
}

/// A clean lineage is a no-op: no turns, exit 0.
#[test]
fn clean_lineage_is_noop() {
    let w = World::new("noop");
    w.seed();
    let (o, e, c) = w.run(&["doctor", "--lineage", "lin-t"], &[]);
    assert_eq!(c, 0, "doctor: {o}{e}");
    assert_eq!(w.turns(), "", "no turns added");
}

/// S3: a stale bee.log is reported via escalate (restart stays operator's).
#[test]
fn dead_keeper_reported() {
    let w = World::new("keeper");
    w.seed();
    fs::write(w.line.join("bee.log"), "{\"card\":\"CARD-1\",\"exit\":1}\n").unwrap();
    // Force the file mtime into the past via the bound: 0 secs = always stale.
    let (o, e, c) = w.run(
        &["doctor", "--lineage", "lin-t"],
        &[("CADDIS_DOCTOR_KEEPER_STALE_SECS", "0")],
    );
    assert_eq!(c, 0, "doctor: {o}{e}");
    assert!(o.contains("keeper"), "names the keeper: {o}");
    assert!(
        w.turns().contains("\"kind\":\"escalate\""),
        "escalated: {}",
        w.turns()
    );
}
