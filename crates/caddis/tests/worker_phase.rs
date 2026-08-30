//! worker_phase.rs tests — CARD-0221. Hermetic HOME. Never live bag.

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
    let p = std::env::temp_dir().join(format!("caddis-phase-{}-{n}-{tag}", std::process::id()));
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

    fn queue(&self, body: &str) {
        let dir = self.line_dir();
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("queue"), body).unwrap();
    }
}

#[test]
fn task_line_mode_logs_phase_and_board_shows_it() {
    let w = World::new("taskmode");
    w.arm();
    fs::write(w.line_dir().join("worker.cfg"), "workflow=tasks\n").unwrap();
    w.queue("CARD-0221 add retry backoff to flap client\n");
    let (o, e, c) = w.run(&["worker", "tick", "--lineage", "line-a"]);
    assert_eq!(c, 0, "tick: {o}{e}");
    let log = fs::read_to_string(w.line_dir().join("phases.log")).unwrap();
    assert!(log.contains("\"card\":\"CARD-0221\""), "{log}");
    assert!(
        log.contains("\"phase\":\"task\""),
        "task phase journaled: {log}"
    );
    let (o, e, c) = w.run(&["worker", "board", "--lineage", "line-a"]);
    assert_eq!(c, 0, "board: {o}{e}");
    assert!(o.contains("PHASE"), "{o}{e}");
    assert!(o.contains("CARD-0221"), "{o}{e}");
}

#[test]
fn argv_mode_default_no_phase_log() {
    let w = World::new("argvmode");
    w.arm();
    fs::write(
        w.root.join("_card_0216.md"),
        "# Done-When\n- $ python -c pass\n",
    )
    .unwrap();
    w.queue("CARD-0216 python -c pass\n");
    let (o, e, c) = w.run(&["worker", "tick", "--lineage", "line-a"]);
    assert_eq!(c, 0, "tick: {o}{e}");
    assert!(
        !w.line_dir().join("phases.log").exists(),
        "argv mode stays legacy"
    );
}
