//! beekeeper_heartbeat.rs — CARD-0262. Make in-band work visible to the
//! overnight watch via a heartbeat file in the lineage dir.
//!
//! RED choreography: a hermetic line dir with a queued head, silent
//! bee.log, and a fresh keeper.heartbeat shows bee=DEAD under the current
//! detection. After: fresh heartbeat reads WORKING; stale heartbeat
//! (>2 intervals) with queued head still reads DEAD truthfully.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static SEQ: AtomicU64 = AtomicU64::new(0);
const TEST_KEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

fn tmp(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!("caddis-bh-{}-{n}-{tag}", std::process::id()));
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

/// Locate python or python3 in PATH. On Windows, falls back to `py -3`.
fn python_cmd() -> Command {
    for bin in ["python", "python3"] {
        if let Ok(o) = Command::new(bin).arg("-c").arg("1").output() {
            if o.status.success() {
                return Command::new(bin);
            }
        }
    }
    let mut c = Command::new("py");
    c.arg("-3");
    c
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// Set a file's mtime to `secs` seconds since epoch, portably.
fn set_mtime(path: &Path, secs: i64) {
    let script = format!(
        "import os; os.utime(r'{}', ({}, {}))",
        path.to_string_lossy(),
        secs,
        secs
    );
    let out = python_cmd()
        .arg("-c")
        .arg(&script)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("python utime must spawn");
    assert!(
        out.status.success(),
        "utime failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

struct World {
    home: PathBuf,
    root: PathBuf,
    herdr_fixture: PathBuf,
    warden_bin: PathBuf,
    watch_src: PathBuf,
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
        let manifest = env!("CARGO_MANIFEST_DIR");
        let watch_src = Path::new(manifest)
            .join("..")
            .join("..")
            .join("tools")
            .join("overnight_watch.py");
        Self {
            home,
            root,
            herdr_fixture,
            warden_bin,
            watch_src,
        }
    }

    fn run(&self, args: &[&str]) -> (String, String, i32) {
        let suite = self.root.join("scan-suite.txt");
        fs::write(&suite, "noop python -c pass\n").unwrap();
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_caddis"));
        cmd.args(args)
            .current_dir(&self.root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("CADDIS_HMAC_KEY", TEST_KEY)
            .env("CADDIS_DRAIN_HERDR", &self.herdr_fixture)
            .env("CADDIS_DASH_NO_ENSURE", "1")
            .env("CADDIS_SCAN_SUITE", &suite)
            .env("PATH", prepend_path(&self.warden_bin));
        let out = cmd.output().expect("caddis must spawn");
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

    fn queue(&self, body: &str) {
        let dir = self.line_dir();
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("queue"), body).unwrap();
    }

    fn line_dir(&self) -> PathBuf {
        self.home
            .join(".caddis")
            .join("rotation")
            .join("lines")
            .join("line-a")
    }

    /// Call overnight_watch.bee_alive with HOME pointed at our hermetic dir.
    fn bee_alive(&self, lineage: &str) -> bool {
        let script = "\
import importlib.util, os, sys\n\
home = os.environ['CADDIS_TEST_HOME']\n\
os.environ['USERPROFILE'] = home\n\
os.environ['HOME'] = home\n\
src = os.environ['CADDIS_TEST_WATCH']\n\
lin = os.environ['CADDIS_TEST_LINE']\n\
spec = importlib.util.spec_from_file_location('ow', src)\n\
mod = importlib.util.module_from_spec(spec)\n\
spec.loader.exec_module(mod)\n\
print('WORKING' if mod.bee_alive(lin) else 'DEAD')\n";
        let out = python_cmd()
            .arg("-c")
            .arg(script)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("CADDIS_TEST_HOME", &self.home)
            .env("CADDIS_TEST_WATCH", &self.watch_src)
            .env("CADDIS_TEST_LINE", lineage)
            .output()
            .expect("python must spawn");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "bee_alive python failed: {stderr}");
        stdout.trim().contains("WORKING")
    }
    /// Touch keeper.heartbeat with an mtime `age_secs` in the past.
    fn touch_heartbeat(&self, age_secs: u64) {
        let dir = self.line_dir();
        fs::create_dir_all(&dir).unwrap();
        let hb = dir.join("keeper.heartbeat");
        fs::write(&hb, b"").unwrap();
        let past = unix_now().saturating_sub(age_secs as i64);
        set_mtime(&hb, past);
    }
}

/// A queued head with a fresh keeper.heartbeat (bumped by the
/// beekeeper cycle) reads WORKING even when bee.log is silent.
#[test]
fn fresh_heartbeat_reads_working() {
    let w = World::new("fresh");
    w.arm();
    w.queue("CARD-9701 python -c pass\n");
    let (o, e, c) = w.run(&["beekeeper", "--once", "--lineage", "line-a"]);
    assert_eq!(c, 0, "beekeeper --once: {o}{e}");
    let hb = w.line_dir().join("keeper.heartbeat");
    assert!(hb.exists(), "keeper.heartbeat must exist after a cycle");
    assert!(w.bee_alive("line-a"), "fresh heartbeat = WORKING");
}

/// A queued head with a STALE heartbeat (>2 intervals) and silent
/// bee.log still reads DEAD — the heartbeat does not lie.
#[test]
fn stale_heartbeat_reads_dead() {
    let w = World::new("stale");
    w.arm();
    w.queue("CARD-9702 python -c pass\n");
    w.touch_heartbeat(120);
    assert!(!w.bee_alive("line-a"), "stale heartbeat = DEAD");
}

/// Absent heartbeat (old keeper binary) degrades to today's behavior:
/// queued head + silent bee.log = DEAD, never worse.
#[test]
fn absent_heartbeat_degrades_gracefully() {
    let w = World::new("absent");
    w.arm();
    w.queue("CARD-9703 python -c pass\n");
    assert!(
        !w.bee_alive("line-a"),
        "absent heartbeat = DEAD (today's behavior)"
    );
}
