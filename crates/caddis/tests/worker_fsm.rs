//! worker_fsm.rs tests — CARD-0225. Hermetic HOME. Never live bag.

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
    let p = std::env::temp_dir().join(format!("caddis-fsm-{}-{n}-{tag}", std::process::id()));
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
            herdr_fixture,
            warden_bin,
        }
    }

    fn run(&self, args: &[&str]) -> (String, String, i32) {
        let out = Command::new(env!("CARGO_BIN_EXE_caddis"))
            .args(args)
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
            "m1",
            "--lineage",
            "line-a",
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

    fn phase(&self, args: &[&str]) -> (String, String, i32) {
        let mut v = vec!["worker", "phase", "--lineage", "line-a"];
        v.extend_from_slice(args);
        self.run(&v)
    }
}

#[test]
fn phase_without_lineage_is_usage() {
    let w = World::new("nousage");
    let (o, e, c) = w.run(&["worker", "phase"]);
    assert_eq!(c, 2, "phase requires --lineage: {o}{e}");
}

#[test]
fn lifecycle_advances_and_reports_state() {
    let w = World::new("life");
    w.arm();
    for (i, ph) in ["task", "scout", "build", "scan"].iter().enumerate() {
        let (o, e, c) = w.phase(&["--card", "CARD-9", "--advance", ph]);
        assert_eq!(c, 0, "advance {ph}: {o}{e}");
        assert!(o.contains(ph), "{o}{e}");
        let _ = i;
    }
    let (o, e, c) = w.phase(&[]);
    assert_eq!(c, 0, "read: {o}{e}");
    assert!(o.contains("CARD-9"), "{o}{e}");
    assert!(o.contains("scan"), "{o}{e}");
    assert!(o.contains("r1"), "attempt identity: {o}{e}");
    let log = fs::read_to_string(w.line_dir().join("phases.log")).unwrap();
    assert!(log.contains("\"phase\":\"scan\""), "{log}");
}

#[test]
fn repair_cap_three_then_fail_journaled() {
    let w = World::new("cap");
    w.arm();
    let (o, e, c) = w.phase(&["--card", "CARD-9", "--advance", "scan"]);
    assert_eq!(c, 0, "scan: {o}{e}");
    for k in 1..=3 {
        let (o, e, c) = w.phase(&["--card", "CARD-9", "--advance", "repair"]);
        assert_eq!(c, 0, "repair {k}: {o}{e}");
        assert!(o.contains(&format!("r{k}")), "{o}{e}");
        w.phase(&["--card", "CARD-9", "--advance", "scan"]);
    }
    let (o, e, c) = w.phase(&["--card", "CARD-9", "--advance", "repair"]);
    assert_eq!(c, 1, "4th repair denied: {o}{e}");
    let log = fs::read_to_string(w.line_dir().join("phases.log")).unwrap();
    assert!(
        log.contains("\"phase\":\"fail\""),
        "cap fail journaled: {log}"
    );
    let (o, _e, c) = w.phase(&[]);
    assert_eq!(c, 0, "read after fail: {o}");
    assert!(o.contains("fail"), "{o}");
}

#[test]
fn unknown_phase_is_usage() {
    let w = World::new("bogus");
    w.arm();
    let (o, e, c) = w.phase(&["--card", "CARD-9", "--advance", "banana"]);
    assert_eq!(c, 2, "enum enforced: {o}{e}");
}
