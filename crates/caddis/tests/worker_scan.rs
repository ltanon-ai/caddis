//! worker_scan.rs tests — CARD-0219. Hermetic: fixture suite via
//! CADDIS_SCAN_SUITE, census via CADDIS_SCAN_ROOT. Never live cargo.

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
    let p = std::env::temp_dir().join(format!("caddis-scan-{}-{n}-{tag}", std::process::id()));
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

    fn line_dir(&self) -> PathBuf {
        self.home
            .join(".caddis")
            .join("rotation")
            .join("lines")
            .join("line-a")
    }
}

#[test]
fn scan_without_lineage_is_usage() {
    let w = World::new("nousage");
    let (o, e, c) = w.run(&["worker", "scan"]);
    assert_eq!(c, 2, "scan requires --lineage: {o}{e}");
}

#[test]
fn scan_suite_green_writes_log() {
    let w = World::new("green");
    // fixture suite: two passing commands
    let suite = w.root.join("suite.txt");
    fs::write(&suite, "check1 python -c pass\ncheck2 python -c pass\n").unwrap();
    // census root: one small file
    let cr = w.root.join("crates");
    fs::create_dir_all(&cr).unwrap();
    fs::write(cr.join("a.rs"), "fn main() {}\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_caddis"))
        .args(["worker", "scan", "--lineage", "line-a"])
        .current_dir(&w.root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("HOME", &w.home)
        .env("USERPROFILE", &w.home)
        .env("CADDIS_HMAC_KEY", TEST_KEY)
        .env("CADDIS_SCAN_SUITE", &suite)
        .env("CADDIS_SCAN_ROOT", &cr)
        .output()
        .expect("spawn");
    let o = String::from_utf8_lossy(&out.stdout).into_owned();
    let e = String::from_utf8_lossy(&out.stderr).into_owned();
    let c = out.status.code().unwrap_or(-1);
    assert_eq!(c, 0, "scan green: {o}{e}");
    assert!(
        o.lines()
            .any(|l| l.starts_with("check1") && l.ends_with("pass")),
        "{o}"
    );
    assert!(
        o.lines()
            .any(|l| l.starts_with("census") && l.ends_with("pass")),
        "{o}"
    );
    assert!(o.contains("SUMMARY pass"), "{o}");
    let log = fs::read_to_string(w.line_dir().join("scan.log")).unwrap();
    assert!(log.contains("\"kind\":\"scan\""), "{log}");
    assert!(log.contains("\"check1\":\"pass\""), "{log}");
}

#[test]
fn scan_suite_red_fails() {
    let w = World::new("red");
    let suite = w.root.join("suite.txt");
    fs::write(
        &suite,
        "check1 python -c pass\ncheck2 python -c \"import sys;sys.exit(2)\"\n",
    )
    .unwrap();
    let cr = w.root.join("crates");
    fs::create_dir_all(&cr).unwrap();
    fs::write(cr.join("a.rs"), "fn main() {}\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_caddis"))
        .args(["worker", "scan", "--lineage", "line-a"])
        .current_dir(&w.root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("HOME", &w.home)
        .env("USERPROFILE", &w.home)
        .env("CADDIS_HMAC_KEY", TEST_KEY)
        .env("CADDIS_SCAN_SUITE", &suite)
        .env("CADDIS_SCAN_ROOT", &cr)
        .output()
        .expect("spawn");
    let o = String::from_utf8_lossy(&out.stdout).into_owned();
    let c = out.status.code().unwrap_or(-1);
    assert_eq!(c, 1, "scan red exits 1: {o}");
    assert!(o.contains("check2   FAIL"), "{o}");
    assert!(o.contains("SUMMARY fail"), "{o}");
}

#[test]
fn census_over_280_fails() {
    let w = World::new("fat");
    let suite = w.root.join("suite.txt");
    fs::write(&suite, "check1 python -c pass\n").unwrap();
    let cr = w.root.join("crates");
    fs::create_dir_all(&cr).unwrap();
    let fat = "x\n".repeat(281);
    fs::write(cr.join("fat.rs"), fat).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_caddis"))
        .args(["worker", "scan", "--lineage", "line-a"])
        .current_dir(&w.root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("HOME", &w.home)
        .env("USERPROFILE", &w.home)
        .env("CADDIS_HMAC_KEY", TEST_KEY)
        .env("CADDIS_SCAN_SUITE", &suite)
        .env("CADDIS_SCAN_ROOT", &cr)
        .output()
        .expect("spawn");
    let o = String::from_utf8_lossy(&out.stdout).into_owned();
    let c = out.status.code().unwrap_or(-1);
    assert_eq!(c, 1, "fat file fails scan: {o}");
    assert!(o.contains("census   FAIL"), "{o}");
    assert!(o.contains("fat.rs:281"), "{o}");
}
