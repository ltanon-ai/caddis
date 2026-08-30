//! beekeeper_host.rs — CARD-0236 RED-first. The waking-session loop
//! (`_worker_loop.py`, untracked scratch) ported to Rust as a HOST:
//! read queue head, call the worker tick, scan when a card ran,
//! redraw the board, sleep. It holds NO law — every halt decision
//! comes from the organ. Never a `worker` subcommand (locked spec:
//! the worker is one-shot by design).

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
    let p = std::env::temp_dir().join(format!("caddis-bk-{}-{n}-{tag}", std::process::id()));
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
            fs::set_permissions(&p, fs::PermissionsExt::from_mode(0o755)).unwrap();
        }
        Self {
            home,
            root,
            herdr_fixture,
            warden_bin,
        }
    }

    fn run(&self, args: &[&str]) -> (String, String, i32) {
        // A tiny scan suite so `worker scan` is hermetic and fast.
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
        let dir = self
            .home
            .join(".caddis")
            .join("rotation")
            .join("lines")
            .join("line-a");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("queue"), body).unwrap();
    }
}

/// One cycle with a runnable head card: tick dispatches the bee, the
/// card EARNS done, scan runs (fixture suite), the board renders, and
/// the cycle exits 0 under --once.
#[test]
fn one_cycle_ticks_scans_boards_and_earns_done() {
    let w = World::new("cycle");
    w.arm();
    let marker = w.root.join("marker.txt");
    let bee = w.root.join("bee.py");
    fs::write(&bee, "import sys\nopen(sys.argv[1],'a').write('run\\n')\n").unwrap();
    fs::write(
        w.root.join("_card_9301.md"),
        "# Done-When\n\n- $ python -c pass\n",
    )
    .unwrap();
    w.queue(&format!(
        "CARD-9301 python {} {}\n",
        bee.display(),
        marker.display()
    ));

    let (o, e, c) = w.run(&["beekeeper", "--lineage", "line-a", "--once"]);
    assert_eq!(c, 0, "cycle: {o}{e}");
    assert!(
        o.contains("PACE WORK CARD-9301"),
        "the tick dispatched: {o}{e}"
    );
    assert!(o.contains("DW-OK"), "done earned through the gate: {o}{e}");
    assert!(
        o.contains("SUMMARY pass"),
        "scan ran (fixture suite): {o}{e}"
    );
    assert!(o.contains("worker board"), "the board rendered: {o}{e}");
    let q = fs::read_to_string(
        w.home
            .join(".caddis")
            .join("rotation")
            .join("lines")
            .join("line-a")
            .join("queue"),
    )
    .unwrap();
    assert!(q.contains("done CARD-9301"), "the line left rotation: {q}");
    assert!(marker.exists(), "the bee ran");
}

/// Empty queue: one tick (IDLE-OK), NO scan, board renders, exit 0.
#[test]
fn empty_queue_cycle_is_idle_without_scan() {
    let w = World::new("idle");
    w.arm();
    w.queue("");
    let (o, e, c) = w.run(&["beekeeper", "--lineage", "line-a", "--once"]);
    assert_eq!(c, 0, "{o}{e}");
    assert!(o.contains("PACE IDLE-OK"), "{o}{e}");
    assert!(!o.contains("SUMMARY"), "no card ran, so no scan: {o}{e}");
    assert!(o.contains("worker board"), "{o}{e}");
}

#[test]
fn missing_lineage_is_usage() {
    let w = World::new("nousage");
    let (o, e, c) = w.run(&["beekeeper", "--once"]);
    assert_eq!(c, 2, "{o}{e}");
}

/// The host holds no law of its own: a withheld line is halted by the
/// ORGAN-side gate (CARD-0235), and the beekeeper just reports it —
/// the next cycle sees an empty remaining queue (IDLE-OK), never a
/// re-fire of the halted line.
#[test]
fn halted_line_is_not_refired_next_cycle() {
    let w = World::new("halted");
    w.arm();
    let marker = w.root.join("marker.txt");
    let bee = w.root.join("bee.py");
    fs::write(&bee, "import sys\nopen(sys.argv[1],'a').write('run\\n')\n").unwrap();
    fs::write(
        w.root.join("_card_9302.md"),
        "# Withheld\n\n# Done-When\n\n- $ python -c \"import sys;sys.exit(1)\"\n",
    )
    .unwrap();
    w.queue(&format!(
        "CARD-9302 python {} {}\n",
        bee.display(),
        marker.display()
    ));

    let threshold = caddis_organs::watchdog::DEFAULT_MAX_FAILURES;
    let mut last = String::new();
    for _ in 0..threshold {
        let (o, e, c) = w.run(&["beekeeper", "--lineage", "line-a", "--once"]);
        assert_eq!(c, 0, "{o}{e}");
        last = o;
    }
    assert!(
        last.contains("WITHHELD-HALT CARD-9302"),
        "the halt surfaced on the threshold cycle: {last}"
    );
    // The cycle AFTER the halt: nothing remaining — the line is gone
    // from rotation, no re-fire, no marker growth.
    let before = fs::read_to_string(&marker).unwrap().lines().count();
    let (o, _, c) = w.run(&["beekeeper", "--lineage", "line-a", "--once"]);
    assert_eq!(c, 0, "{o}");
    assert!(
        o.contains("PACE IDLE-OK"),
        "halted line is not re-fired: {o}"
    );
    let after = fs::read_to_string(&marker).unwrap().lines().count();
    assert_eq!(before, after, "no dispatch after the halt");
}
