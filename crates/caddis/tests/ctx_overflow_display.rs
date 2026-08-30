//! ctx_overflow_display.rs — CARD-0247 RED. The ctx indicator must MEAN
//! something: real ratio, named subject, stated action. The operator
//! ruling: a clamp is a lie; a red badge without a next step is noise.

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
    let p = std::env::temp_dir().join(format!(
        "caddis-ctxoverflow-{}-{n}-{tag}",
        std::process::id()
    ));
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
            "lin-a",
            "--pane",
            "w3J:pY",
        ]);
        assert!(c == 0, "ready: {o}{e}");
        let (o, e, c) = self.run(&["rotate", "arm", "--lineage", "lin-a"]);
        assert!(c == 0, "arm: {o}{e}");
    }

    fn observe(&self, session: &str, body: &str) {
        let dir = self.home.join(".caddis").join("pager");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(format!("{session}.observe.jsonl")), body).unwrap();
        fs::create_dir_all(dir.join(session)).unwrap();
    }
}

#[test]
fn over_capacity_renders_true_ratio_subject_and_remedy() {
    let w = World::new("overflow");
    w.arm();
    let body = r#"{"kind":"context","parse_ok":true,"n_messages":1,"chars":4,"largest_tool_result_chars":0,"stored_tokens":33730,"sent_est_tokens":8000,"stored_window":16384,"stored_pct":100,"stored_over":true,"stored_ratio_milli":2060,"n_stubbed":0,"user_chars":0,"assistant_chars":0,"toolResult_chars":0,"page_mode":false}
"#;
    w.observe("lin-a", body);
    let (o, e, c) = w.run(&["worker", "board", "--lineage", "lin-a"]);
    assert_eq!(c, 0, "board: {o}{e}");
    // VALUE: the TRUE number — 206% OVER (33730/16384) — never the clamped 100%.
    assert!(o.contains("206%"), "must show true ratio percent: {o}");
    assert!(o.contains("OVER"), "must say OVER: {o}");
    assert!(
        o.contains("33730") && o.contains("16384"),
        "must show raw tokens/window: {o}"
    );
    // SUBJECT: a labeled obs=<session> source row.
    assert!(o.contains("obs="), "must name the subject: {o}");
    // ACTION: the row appends a remedy hint.
    assert!(
        o.contains("compact") || o.contains("switch to a larger-window model"),
        "must append a remedy: {o}"
    );
}

#[test]
fn healthy_capacity_renders_green_percent_no_over_no_action() {
    let w = World::new("healthy");
    w.arm();
    let body = r#"{"kind":"context","parse_ok":true,"n_messages":1,"chars":4,"largest_tool_result_chars":0,"stored_tokens":5000,"sent_est_tokens":1000,"stored_window":16384,"stored_pct":31,"n_stubbed":0,"user_chars":0,"assistant_chars":0,"toolResult_chars":0,"page_mode":false}
"#;
    w.observe("lin-a", body);
    let (o, e, c) = w.run(&["worker", "board", "--lineage", "lin-a"]);
    assert_eq!(c, 0, "board: {o}{e}");
    assert!(o.contains("31%"), "healthy green percent: {o}");
    assert!(!o.contains("OVER ("), "healthy must not render OVER: {o}");
    assert!(
        !o.contains("compact or switch"),
        "healthy must not append the remedy: {o}"
    );
}
