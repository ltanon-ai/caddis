//! worker_board.rs — CARD-0217. Hermetic HOME. Never ~/.caddis live bag.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

const TEST_KEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

fn tmp(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!("caddis-board-{}-{n}-{tag}", std::process::id()));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

fn prepend_path(first: &Path) -> OsString {
    let mut out = first.as_os_str().to_os_string();
    if let Some(rest) = env::var_os("PATH") {
        out.push(if cfg!(windows) { ";" } else { ":" });
        out.push(rest);
    }
    out
}

struct World {
    home: PathBuf,
    root: PathBuf,
    herdr_fixture: PathBuf,
    warden_bin: PathBuf,
}

impl World {
    fn new(tag: &str) -> Self {
        let root = tmp(tag);
        let home = root.join("home");
        fs::create_dir_all(&home).unwrap();
        let herdr_fixture = root.join("herdr.json");
        fs::write(&herdr_fixture, "").unwrap();
        let warden_bin = root.join("bin");
        fs::create_dir_all(&warden_bin).unwrap();
        #[cfg(windows)]
        fs::write(
            warden_bin.join("caddis-warden.cmd"),
            "@echo off\r\nexit /b 0\r\n",
        )
        .unwrap();
        #[cfg(not(windows))]
        {
            use std::os::unix::fs::PermissionsExt;
            let p = warden_bin.join("caddis-warden");
            fs::write(&p, "#!/bin/sh\nexit 0\n").unwrap();
            fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
        }
        Self {
            home,
            root,
            herdr_fixture,
            warden_bin,
        }
    }

    fn run(&self, args: &[&str]) -> (String, String, i32) {
        let out = Command::new(env!("CARGO_BIN_EXE_caddis"))
            .args(args)
            .current_dir(&self.root)
            .env("CADDIS_BOARD_WIDTH", "200")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("CADDIS_HMAC_KEY", TEST_KEY)
            .env("PATH", prepend_path(&self.warden_bin))
            .env("CADDIS_DRAIN_HERDR", &self.herdr_fixture)
            .output()
            .expect("caddis must spawn");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    }

    fn arm(&self) {
        let (o, e, c) = self.run(&[
            "rotate",
            "ready",
            "--kind",
            "omp",
            "--model",
            "grok-4.6",
            "--lineage",
            "line-a",
            "--pane",
            "w3J:pY",
        ]);
        assert_eq!(c, 0, "ready: {o}{e}");
        let (o, e, c) = self.run(&["rotate", "arm", "--lineage", "line-a"]);
        assert_eq!(c, 0, "arm: {o}{e}");
    }

    fn line_dir(&self) -> PathBuf {
        self.home
            .join(".caddis")
            .join("rotation")
            .join("lines")
            .join("line-a")
    }

    fn observe(&self, session: &str, body: &str) {
        let dir = self.home.join(".caddis").join("pager");
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join(format!("{session}.observe.jsonl"));
        fs::write(p, body).unwrap();
    }

    fn cold(&self, session: &str, records: usize) {
        let dir = self.home.join(".caddis").join("pager").join(session);
        fs::create_dir_all(&dir).unwrap();
        let mut text = String::new();
        for seq in 1..=records {
            text.push_str(&format!(
                "seq={seq}\nrole=toolResult\nchars=900\nturn={seq}\nsha=ab\ntext=ff\n---\n"
            ));
        }
        fs::write(dir.join("cold.store"), text).unwrap();
        fs::write(dir.join("mode"), "page\n").unwrap();
        fs::write(dir.join("mark"), "100000\n").unwrap();
    }
}

const OBSERVE: &str = concat!(
    r#"{"kind":"context","stored_tokens":81000,"sent_est_tokens":71000,"stored_pct":31,"n_stubbed":2}"#,
    "\n",
    r#"{"kind":"project","n_evicted":5}"#,
    "\n",
    r#"{"kind":"message_end","usage":{"input":1200,"cacheRead":3400,"cacheWrite":800,"output":900,"reasoningTokens":1500}}"#,
    "\n",
);

#[test]
fn board_without_lineage_is_usage() {
    let w = World::new("nousage");
    let (o, e, c) = w.run(&["worker", "board"]);
    assert_eq!(c, 2, "board requires --lineage: {o}{e}");
}

#[test]
fn board_frame_shows_all_organs() {
    let w = World::new("frame");
    w.arm();
    w.observe("line-a", OBSERVE);
    w.cold("line-a", 3);
    let dir = w.line_dir();
    fs::write(
        dir.join("queue"),
        "done CARD-0900 cargo test\nCARD-0217 cargo build\n",
    )
    .unwrap();
    fs::write(
        dir.join("bee.log"),
        r#"{"card":"CARD-0900","argv0":"cargo","exit":0,"ts":1787890000}"#,
    )
    .unwrap();
    fs::write(
        dir.join("scan.log"),
        "{\"kind\":\"scan\",\"fmt\":\"pass\",\"clippy\":\"pass\",\"test\":\"pass\",\"census\":\"pass\",\"ts\":\"1\"}\n",
    )
    .unwrap();
    let (o, e, c) = w.run(&["check", "--lineage", "line-a"]);
    assert_eq!(c, 0, "check seeds pace.line: {o}{e}");
    let (o, e, c) = w.run(&["worker", "board", "--lineage", "line-a"]);
    assert_eq!(c, 0, "board: {o}{e}");
    for needle in [
        "lineage line-a",
        "kind=omp",
        "model=grok-4.6",
        "pane=w3J:pY",
        "PACE WORK CARD-0217",
        "remaining=1",
        "done=1",
        "CARD-0217 cargo build",
        "argv0=cargo",
        "exit=0",
        "cold=3",
        "mode=page",
        "mark=100000",
        "stored=81000",
        "sent=71000",
        "pct=31",
        "stubbed=2",
        "evicted=5",
        "input=1200",
        "cacheRead=3400",
        "reasoningTokens=1500",
        "verdict=pass",
    ] {
        assert!(o.contains(needle), "frame must contain {needle}: {o}{e}");
    }
    assert!(!o.contains("fault="), "fault is not a real nerve kind: {o}");
}

#[test]
fn tampered_pace_line_shows_unverified() {
    let w = World::new("tamper");
    w.arm();
    w.observe("line-a", OBSERVE);
    let (o, e, c) = w.run(&["check", "--lineage", "line-a"]);
    assert_eq!(c, 0, "check: {o}{e}");
    let p = w.line_dir().join("pace.line");
    fs::write(p, "sentence=PACE WORK FORGED\n---\n00\n").unwrap();
    let (o, e, c) = w.run(&["worker", "board", "--lineage", "line-a"]);
    assert_eq!(c, 0, "board: {o}{e}");
    assert!(o.contains("PACE unverified"), "{o}{e}");
    assert!(!o.contains("FORGED"), "must not print forged: {o}");
}

#[test]
fn missing_session_degrades_not_fails() {
    let w = World::new("nopage");
    w.arm();
    let (o, e, c) = w.run(&["check", "--lineage", "line-a"]);
    assert_eq!(c, 0, "check: {o}{e}");
    let (o, e, c) = w.run(&["worker", "board", "--lineage", "line-a"]);
    assert_eq!(c, 0, "board without page data: {o}{e}");
    assert!(
        o.contains("session=line-a"),
        "default session is lineage: {o}{e}"
    );
    assert!(o.contains("cold=0"), "absent page is honest zeros: {o}{e}");
}
