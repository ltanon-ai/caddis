//! worker_board_idle.rs — CARD-0256. The PHASE panel must be
//! queue-aware: an empty queue with no live bee run shows IDLE, not
//! the stale phases.log tail. Hermetic HOME. Never ~/.caddis live bag.

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
    let p = std::env::temp_dir().join(format!("caddis-idle-{}-{n}-{tag}", std::process::id()));
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
}

/// Extract the PHASE section body (between the PHASE header and the
/// next section header QUEUE), so assertions don't match the EVENTS
/// feed or the BEE section which truthfully show history.
fn phase_section(o: &str) -> String {
    let start = o.find("PHASE").unwrap_or(0);
    let end = o[start..]
        .find("QUEUE")
        .map(|n| n + start)
        .unwrap_or(o.len());
    o[start..end].to_string()
}

/// A phases.log whose tail is a completed card, plus an EMPTY queue
/// (all lines done) and no live worker.lock → the board must show
/// IDLE, never the dead card/phase.
#[test]
fn empty_queue_shows_idle_not_stale_phase() {
    let w = World::new("idle");
    w.arm();
    let dir = w.line_dir();
    // The queue is drained: every line is done, remaining=0.
    fs::write(dir.join("queue"), "done CARD-0001 cargo test\n").unwrap();
    // Stale phases.log tail — the ghost the board must NOT show.
    fs::write(
        dir.join("phases.log"),
        "{\"card\":\"CARD-0001\",\"phase\":\"task\",\"ts\":\"2026-08-28T16:00:09Z\"}\n",
    )
    .unwrap();
    let (o, e, c) = w.run(&["check", "--lineage", "line-a"]);
    assert_eq!(c, 0, "check seeds pace.line: {o}{e}");
    let (o, e, c) = w.run(&["worker", "board", "--lineage", "line-a"]);
    assert_eq!(c, 0, "board: {o}{e}");
    let p = phase_section(&o);
    assert!(p.contains("PHASE"), "phase section renders: {o}{e}");
    // The dead card must NOT leak into the phase panel.
    assert!(
        !p.contains("CARD-0001"),
        "stale card must not show when queue is empty: {p}"
    );
    assert!(
        !p.contains("task"),
        "stale phase must not show when queue is empty: {p}"
    );
    // The idle marker is present.
    assert!(
        p.contains("idle"),
        "idle marker shown when queue empty: {p}"
    );
}

/// With a queued CARD head present, the phase panel behaves exactly
/// as before — the phases.log tail is truthful (the card is live).
#[test]
fn queued_head_shows_phase_as_before() {
    let w = World::new("queued");
    w.arm();
    let dir = w.line_dir();
    fs::write(
        dir.join("queue"),
        "done CARD-0001 cargo test\nCARD-0256 cargo build\n",
    )
    .unwrap();
    fs::write(
        dir.join("phases.log"),
        "{\"card\":\"CARD-0256\",\"phase\":\"task\",\"ts\":\"2026-08-28T16:00:09Z\"}\n",
    )
    .unwrap();
    let (o, e, c) = w.run(&["check", "--lineage", "line-a"]);
    assert_eq!(c, 0, "check: {o}{e}");
    let (o, e, c) = w.run(&["worker", "board", "--lineage", "line-a"]);
    assert_eq!(c, 0, "board: {o}{e}");
    let p = phase_section(&o);
    assert!(p.contains("PHASE"), "phase section renders: {o}{e}");
    assert!(
        p.contains("CARD-0256"),
        "queued card shows in phase panel: {p}"
    );
    assert!(
        p.contains("task"),
        "queued card phase shows in phase panel: {p}"
    );
}

/// An empty queue but a LIVE bee run (fresh worker.lock) keeps the
/// phase panel showing the card — the bee is mid-flight, not idle.
#[test]
fn empty_queue_with_live_bee_shows_phase() {
    let w = World::new("livebee");
    w.arm();
    let dir = w.line_dir();
    fs::write(dir.join("queue"), "done CARD-0001 cargo test\n").unwrap();
    fs::write(
        dir.join("phases.log"),
        "{\"card\":\"CARD-0001\",\"phase\":\"task\",\"ts\":\"2026-08-28T16:00:09Z\"}\n",
    )
    .unwrap();
    // A FRESH worker.lock — the bee is running right now.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    fs::write(dir.join("worker.lock"), format!("pid=1\nts={now}\n")).unwrap();
    let (o, e, c) = w.run(&["check", "--lineage", "line-a"]);
    assert_eq!(c, 0, "check: {o}{e}");
    let (o, e, c) = w.run(&["worker", "board", "--lineage", "line-a"]);
    assert_eq!(c, 0, "board: {o}{e}");
    let p = phase_section(&o);
    assert!(p.contains("PHASE"), "phase section renders: {o}{e}");
    // Live bee: the phase panel shows the card (not idle).
    assert!(
        p.contains("CARD-0001"),
        "live bee keeps card in phase panel: {p}"
    );
}
