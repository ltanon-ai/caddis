//! packet_organs.rs — CARD-0257 RED-first. The orientation packet carries
//! the soul HEAD + the valence TAIL around the unchanged arm-receipt body.
//!
//! RED today: `caddis lineage packet` prints only the arm-receipt echo —
//! neither the soul HEAD (`I learned`) nor any valence tail marker appears.
//! After: the HEAD block precedes `LINEAGE <id>`, the arm lines stay
//! byte-identical, and a rendered tail block follows `fold=`; a lineage
//! with no telemetry still exits 0 with a zeroed tail.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);
const KEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

fn tmp(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("caddis-pkt-organs-{tag}-{n}"));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

struct World {
    home: PathBuf,
    line_dir: PathBuf,
}

impl World {
    fn new(tag: &str, lineage: &str) -> Self {
        let root = tmp(tag);
        let home = root.join("home");
        let line_dir = home
            .join(".caddis")
            .join("rotation")
            .join("lines")
            .join(lineage);
        fs::create_dir_all(&line_dir).unwrap();
        fs::write(home.join("empty-drain"), "").unwrap();
        Self { home, line_dir }
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

    /// Stamp an arm.receipt the same way the existing packet testkit does.
    fn arm(&self, lineage: &str) {
        let (o, e, c) = self.run(&[
            "rotate",
            "ready",
            "--lineage",
            lineage,
            "--kind",
            "omp",
            "--model",
            "ma",
            "--pane",
            "w3J:p1",
        ]);
        assert_eq!(c, 0, "ready: {o}{e}");
        let (o, e, c) = self.run(&["rotate", "arm", "--lineage", lineage]);
        assert_eq!(c, 0, "arm: {o}{e}");
    }

    /// Seed the soul fixture: one Pain (blocker b-900, emotion 1, epoch 0)
    /// that composts to a Lesson "caution", plus one Joy.
    fn seed_soul(&self) {
        let soul = self.line_dir.join("soul.jsonl");
        let blockers = self.line_dir.join("blockers.jsonl");
        let pain = "{\"kind\":\"Pain\",\"cause\":\"open pain\",\"blocker_id\":\"b-900\",\
                   \"lesson\":\"caution\",\"emotion\":1,\"epoch\":0,\"created_by_model\":\"m\"}\n";
        fs::write(&soul, pain).unwrap();
        let joy = "{\"kind\":\"Joy\",\"source\":\"green pipeline\",\"lesson\":\"\",\
                  \"emotion\":10,\"epoch\":0,\"created_by_model\":\"m\"}\n";
        let mut f = fs::OpenOptions::new().append(true).open(&soul).unwrap();
        f.write_all(joy.as_bytes()).unwrap();
        let blocker = "{\"source\":\"b-900\",\"reason\":\"broken build\",\
                      \"ts\":\"2026-08-28T00:00:00Z\"}\n";
        fs::write(&blockers, blocker).unwrap();
    }

    /// Seed minimal eddy + bee + scan telemetry.
    fn seed_telemetry(&self) {
        let tick = "{\"run_id\":\"r\",\"seq\":0,\"payload_hash\":\"0000000000000005\",\
                   \"status_class\":\"ok\",\"outcome_hash\":\"0000000000000007\",\
                   \"cache_read\":100,\"cache_write\":10,\"latency_ms\":50,\
                   \"ts_ms\":10000,\"resume_after\":null}\n";
        fs::write(self.line_dir.join("eddy.jsonl"), tick).unwrap();
        let bee = "{\"card\":\"CARD-0001\",\"argv0\":\"cargo\",\"exit\":0,\
                  \"ts\":\"2026-08-28T16:00:04Z\"}\n";
        fs::write(self.line_dir.join("bee.log"), bee).unwrap();
        let scan = "{\"check\":\"test\",\"state\":\"pass\",\"ts\":\"2026-08-28T16:00:14Z\"}\n";
        fs::write(self.line_dir.join("scan.live"), scan).unwrap();
    }
}

/// RED: the packet output carries the soul HEAD before the arm body, the
/// arm lines unchanged, and a valence tail after `fold=`.
#[test]
fn packet_carries_soul_head_and_valence_tail() {
    let w = World::new("organs", "line-a");
    w.arm("line-a");
    w.seed_soul();
    w.seed_telemetry();

    let (out, err, code) = w.run(&["lineage", "packet", "--lineage", "line-a"]);
    assert_eq!(code, 0, "packet: {out}{err}");

    // HEAD: soul identity block with the composted lesson.
    assert!(out.contains("- I learned"), "soul HEAD present: {out}");
    let head_idx = out.find("- I learned").unwrap();
    let lineage_idx = out.find("LINEAGE line-a").unwrap();
    assert!(head_idx < lineage_idx, "HEAD precedes arm body: {out}");

    // ARM body: unchanged, byte-identical order.
    assert!(out.contains("LINEAGE line-a"), "arm lineage: {out}");
    assert!(out.contains("kind=omp"), "arm kind: {out}");
    assert!(out.contains("model=ma"), "arm model: {out}");
    assert!(out.contains("pane=w3J:p1"), "arm pane: {out}");
    assert!(out.contains("fold_at=50"), "arm fold_at: {out}");
    assert!(out.contains("fold=quiet"), "arm fold: {out}");

    // TAIL: a rendered valence block AFTER `fold=`.
    let fold_idx = out.find("fold=quiet").unwrap();
    let tail_idx = out
        .find("tail |")
        .unwrap_or_else(|| out.find("tail|").unwrap_or(usize::MAX));
    assert!(
        tail_idx != usize::MAX && tail_idx > fold_idx,
        "valence tail after fold: {out}"
    );
}

/// RED: a lineage with no telemetry (no soul, no eddy) still exits 0 and
/// renders a zeroed tail — a fresh lineage is a normal state, never an error.
#[test]
fn packet_empty_telemetry_exits_zero_with_zeroed_tail() {
    let w = World::new("empty", "line-b");
    w.arm("line-b");

    let (out, err, code) = w.run(&["lineage", "packet", "--lineage", "line-b"]);
    assert_eq!(code, 0, "empty packet: {out}{err}");
    // No HEAD (no soul.jsonl) — fail-soft: no "I learned" line.
    assert!(!out.contains("- I learned"), "no soul head: {out}");
    // Zeroed tail still present after fold.
    let fold_idx = out.find("fold=quiet").unwrap();
    let tail_idx = out
        .find("tail |")
        .unwrap_or_else(|| out.find("tail|").unwrap_or(usize::MAX));
    assert!(
        tail_idx != usize::MAX && tail_idx > fold_idx,
        "zeroed tail after fold: {out}"
    );
}
