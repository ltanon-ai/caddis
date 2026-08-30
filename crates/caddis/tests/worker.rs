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
fn tick_without_lineage_is_usage() {
    let w = World::new("nousage");
    let (o, e, c) = w.run(&["worker", "tick"]);
    assert_eq!(c, 2, "tick requires --lineage: {o}{e}");
}

#[test]
fn missing_arm_is_fail_no_child() {
    let w = World::new("noarm");
    let marker = w.marker();
    w.queue(&format!(
        "CARD-0216 python -c \"open(r'{}','w').write('x')\"\n",
        marker.display()
    ));
    let (o, e, c) = w.run(&["worker", "tick", "--lineage", "line-a"]);
    assert_eq!(c, 1, "no arm: {o}{e}");
    assert!(!marker.exists(), "must not spawn without ARM");
}

#[test]
fn empty_queue_is_idle_ok() {
    let w = World::new("empty");
    w.arm();
    let marker = w.marker();
    let (o, e, c) = w.run(&["worker", "tick", "--lineage", "line-a"]);
    assert_eq!(c, 0, "empty: {o}{e}");
    assert!(o.contains("PACE IDLE-OK"), "{o}{e}");
    assert!(!marker.exists());
}

#[test]
fn work_spawns_bee_with_arm_harness() {
    let w = World::new("work");
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
        "import os,sys\nopen(sys.argv[1],'w').write(os.environ.get('CADDIS_HARNESS',''))\n",
    )
    .unwrap();
    w.queue(&format!(
        "CARD-0216 python {} {}\n",
        script.display(),
        marker.display()
    ));
    let (o, e, c) = w.run(&["worker", "tick", "--lineage", "line-a"]);
    assert_eq!(c, 0, "work: {o}{e}");
    assert!(o.contains("PACE WORK CARD-0216"), "{o}{e}");
    let got = fs::read_to_string(&marker).unwrap();
    assert_eq!(got, "omp", "harness from ARM: {got}");
    let log = fs::read_to_string(w.line_dir().join("bee.log")).unwrap();
    assert!(
        log.contains("\"card\":\"CARD-0216\"")
            && log.contains("\"argv0\":\"python\"")
            && log.contains("\"exit\":0"),
        "FR-11 journal after spawn: {log}"
    );
}

#[test]
fn unknown_drain_still_works() {
    let w = World::new("unknown");
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
        "import os,sys\nopen(sys.argv[1],'w').write('ok')\n",
    )
    .unwrap();
    w.queue(&format!(
        "CARD-0216 python {} {}\n",
        script.display(),
        marker.display()
    ));
    let (o, e, c) = w.run_drain(&["worker", "tick", "--lineage", "line-a"], false);
    assert_eq!(c, 0, "unknown: {o}{e}");
    assert!(o.contains("PACE WORK CARD-0216"), "must not freeze: {o}{e}");
    assert!(marker.exists(), "bee must run under Unknown drain");
}

#[test]
fn live_chair_is_busy_no_spawn() {
    let w = World::new("busy");
    w.arm();
    let marker = w.marker();
    let script = w.root.join("bee.py");
    fs::write(&script, "open('should-not','w').write('x')\n").unwrap();
    w.queue(&format!(
        "CARD-0216 python {} {}\n",
        script.display(),
        marker.display()
    ));
    fs::write(&w.herdr_fixture, r#"{"status": "live"}"#).unwrap();
    let (o, e, c) = w.run(&["worker", "tick", "--lineage", "line-a"]);
    assert_eq!(c, 0, "busy: {o}{e}");
    assert!(o.contains("PACE BUSY"), "{o}{e}");
    assert!(!o.contains("PACE WORK"), "{o}");
    assert!(!marker.exists());
}

#[test]
fn work_without_argv_does_not_spawn() {
    let w = World::new("noargv");
    w.arm();
    w.queue("CARD-0216\n");
    let (o, e, c) = w.run(&["worker", "tick", "--lineage", "line-a"]);
    assert_eq!(c, 0, "no argv: {o}{e}");
    assert!(o.contains("PACE WORK CARD-0216"), "{o}{e}");
}

#[test]
fn harness_flag_is_usage() {
    let w = World::new("flag");
    w.arm();
    let (o, e, c) = w.run(&[
        "worker",
        "tick",
        "--lineage",
        "line-a",
        "--harness",
        "claude",
    ]);
    assert_eq!(c, 2, "no --harness: {o}{e}");
}
