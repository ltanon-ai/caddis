//! worker_guard.rs — lock/chair/tamper guard tests split from worker.rs (280 cap).

//! worker.rs — CARD-0216. Hermetic HOME. Never ~/.caddis live bag.

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
    let p = std::env::temp_dir().join(format!("caddis-worker-{}-{n}-{tag}", std::process::id()));
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

fn install_warden(bin: &Path) {
    fs::create_dir_all(bin).unwrap();
    #[cfg(windows)]
    fs::write(bin.join("caddis-warden.cmd"), "@echo off\r\nexit /b 0\r\n").unwrap();
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;
        let p = bin.join("caddis-warden");
        fs::write(&p, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

struct World {
    home: PathBuf,
    rot: PathBuf,
    root: PathBuf,
    herdr_fixture: PathBuf,
    warden_bin: PathBuf,
}

impl World {
    fn new(tag: &str) -> Self {
        let root = tmp(tag);
        let home = root.join("home");
        fs::create_dir_all(&home).unwrap();
        let rot = home.join(".caddis").join("rotation");
        let herdr_fixture = root.join("herdr.json");
        fs::write(&herdr_fixture, "").unwrap();
        let warden_bin = root.join("bin");
        install_warden(&warden_bin);
        Self {
            home,
            rot,
            root,
            herdr_fixture,
            warden_bin,
        }
    }

    fn run(&self, args: &[&str]) -> (String, String, i32) {
        self.run_drain(args, true)
    }

    fn run_drain(&self, args: &[&str], drain: bool) -> (String, String, i32) {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_caddis"));
        cmd.args(args)
            .current_dir(&self.root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("CADDIS_HMAC_KEY", TEST_KEY)
            .env("PATH", prepend_path(&self.warden_bin));
        if drain {
            cmd.env("CADDIS_DRAIN_HERDR", &self.herdr_fixture);
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

    fn line_dir(&self) -> PathBuf {
        self.rot.join("lines").join("line-a")
    }

    fn marker(&self) -> PathBuf {
        self.root.join("marker.txt")
    }
}

#[test]
fn held_lock_is_worker_busy_not_forged_pace() {
    let w = World::new("lock");
    w.arm();
    let marker = w.marker();
    let script = w.root.join("bee.py");
    fs::write(
        &script,
        "import os,sys\nopen(sys.argv[1],'w').write('ran')\n",
    )
    .unwrap();
    w.queue(&format!(
        "CARD-0216 python {} {}\n",
        script.display(),
        marker.display()
    ));
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    fs::write(
        w.line_dir().join("worker.lock"),
        format!("pid=1\nts={now}\n"),
    )
    .unwrap();
    let (o, e, c) = w.run(&["worker", "tick", "--lineage", "line-a"]);
    assert_eq!(c, 0, "lock: {o}{e}");
    assert!(o.contains("PACE WORK CARD-0216"), "organ pace: {o}");
    assert!(o.contains("WORKER BUSY"), "{o}{e}");
    assert!(!o.contains("PACE BUSY"), "must not forge PACE BUSY: {o}");
    assert!(!marker.exists());
}

#[test]
fn stale_lock_is_stolen_and_work_proceeds() {
    let w = World::new("stale");
    w.arm();
    fs::write(
        w.root.join("_card_0216.md"),
        "# Done-When\n- $ python -c pass\n",
    )
    .unwrap();
    let marker = w.marker();
    let script = w.root.join("bee.py");
    fs::write(
        &script,
        "import os,sys\nopen(sys.argv[1],'w').write('ran')\n",
    )
    .unwrap();
    w.queue(&format!(
        "CARD-0216 python {} {}\n",
        script.display(),
        marker.display()
    ));
    fs::write(w.line_dir().join("worker.lock"), "pid=1\nts=1\n").unwrap();
    let (o, e, c) = w.run(&["worker", "tick", "--lineage", "line-a"]);
    assert_eq!(c, 0, "stale: {o}{e}");
    assert!(o.contains("PACE WORK CARD-0216"), "{o}{e}");
    assert!(!o.contains("WORKER BUSY"), "stale lock must be stolen: {o}");
    assert!(marker.exists(), "bee must run after steal");
}

#[test]
fn chair_argv_is_fail() {
    let w = World::new("chair");
    w.arm();
    w.queue("CARD-0216 claude --help\n");
    let (o, e, c) = w.run(&["worker", "tick", "--lineage", "line-a"]);
    assert_eq!(c, 1, "chair argv: {o}{e}");
}

#[test]
fn tampered_arm_fails() {
    let w = World::new("tamper");
    w.arm();
    w.queue("CARD-0216 python -c pass\n");
    let arm = w.line_dir().join("arm.receipt");
    let mut raw = fs::read(&arm).unwrap();
    raw[0] ^= 0xff;
    fs::write(&arm, raw).unwrap();
    let (o, e, c) = w.run(&["worker", "tick", "--lineage", "line-a"]);
    assert_eq!(c, 1, "tamper: {o}{e}");
}

#[test]
fn source_has_no_forbidden_calls() {
    let src = include_str!("../src/worker.rs");
    let lock = include_str!("../src/worker_lock.rs");
    let all = format!("{src}\n{lock}");
    assert!(!all.contains("herdr::"));
    assert!(!all.to_ascii_lowercase().contains("tinyagi"));
    assert!(!all.contains("EXECUTION"));
    assert!(!all.contains("drain::"));
    assert!(!all.contains("process::exit"));
}
