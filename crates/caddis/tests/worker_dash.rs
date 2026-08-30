//! worker_dash.rs — CARD-0243 RED-first: the FIXED live worker view
//! (no scroll, last-5 events, mechanisms) + the herdr-split guarantee.

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
    let p = std::env::temp_dir().join(format!("caddis-dash-{}-{n}-{tag}", std::process::id()));
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
            .env("CADDIS_DASH_NO_ENSURE", "1") // tests NEVER split real panes
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
            "grok-x",
            "--pane",
            "w3J:pY",
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

    fn seed(&self, world: usize) {
        let d = self.line_dir();
        fs::create_dir_all(&d).unwrap();
        if world == 0 {
            return;
        } // EMPTY world: nothing
        fs::write(
            d.join("queue"),
            "done CARD-0001 cargo test x\nCARD-0239\nCARD-0240\nCARD-0241\nCARD-0242\nCARD-0243\n",
        )
        .unwrap();
        // A real pace sentence (queue first, so the verdict is WORK):
        let (o, e, c) = self.run(&["check", "--lineage", "line-a", "--pace", "run"]);
        assert_eq!(c, 0, "check: {o}{e}");
        fs::write(
            d.join("bee.log"),
            "{\"card\":\"CARD-0001\",\"argv0\":\"cargo\",\"exit\":0,\"ts\":\"2026-08-28T16:00:04Z\"}\n",
        )
        .unwrap();
        fs::write(
            d.join("phases.log"),
            "{\"card\":\"CARD-0239\",\"phase\":\"task\",\"ts\":\"2026-08-28T16:00:09Z\"}\n",
        )
        .unwrap();
        fs::write(
            d.join("scan.live"),
            "{\"check\":\"test\",\"state\":\"pass\",\"ts\":\"2026-08-28T16:00:14Z\"}\n",
        )
        .unwrap();
        let trail: String = (0..3u64)
            .map(|i| format!("{{\"run_id\":\"line-a\",\"seq\":{i},\"payload_hash\":\"{:016x}\",\"status_class\":\"unprovable\",\"outcome_hash\":\"{:016x}\",\"cache_read\":0,\"cache_write\":0,\"latency_ms\":0,\"ts_ms\":1787932840000,\"resume_after\":null}}\n", 5, 7 + i))
            .collect();
        fs::write(d.join("eddy.jsonl"), trail).unwrap();
        fs::write(self.home.join(".caddis").join("fold.at"), "78").unwrap();
        fs::write(d.join("fold.state"), "").unwrap(); // present => warned
        let pager = self.home.join(".caddis").join("pager");
        let sdir = pager.join("line-a");
        fs::create_dir_all(&sdir).unwrap();
        fs::write(sdir.join("mode"), "page").unwrap();
        fs::write(sdir.join("mark"), "41").unwrap();
        fs::write(
            pager.join("line-a.observe.jsonl"),
            "{\"kind\":\"context\",\"stored_tokens\":410000,\"sent_est_tokens\":500000,\"stored_pct\":41,\"n_stubbed\":2}\n",
        )
        .unwrap();
    }
}

// --watch + --frames: read-only, deterministic.
#[test]
fn watch_flag_renders_fixed_frames() {
    let w = World::new("watch");
    w.arm();
    w.seed(1);
    let (o, e, c) = w.run(&[
        "worker",
        "board",
        "--lineage",
        "line-a",
        "--watch",
        "--frames",
        "2",
        "--interval-ms",
        "10",
    ]);
    assert_eq!(c, 0, "{o}{e}");
    let clears = o.matches("\x1b[2J").count();
    let homes = o.matches("\x1b[H").count();
    assert_eq!(clears, 1, "one clear, then in-place redraw: {e}");
    assert!(homes >= 2, "each frame homes the cursor: {e}");
}

/// THE no-scroll law: two different worlds, IDENTICAL frame height —
/// and every watch frame is that same height.
#[test]
fn frame_height_is_constant_across_worlds() {
    let a = World::new("empty");
    a.arm();
    a.seed(0);
    let b = World::new("full");
    b.arm();
    b.seed(1);
    let (oa, _, ca) = a.run(&["worker", "board", "--lineage", "line-a"]);
    assert_eq!(ca, 0);
    let (ob, _, cb) = b.run(&["worker", "board", "--lineage", "line-a"]);
    assert_eq!(cb, 0);
    let la = oa.trim_end().lines().count();
    let lb = ob.trim_end().lines().count();
    assert_eq!(la, lb, "fixed height: empty={la} full={lb}");
    let (ow, _, cw) = b.run(&[
        "worker",
        "board",
        "--lineage",
        "line-a",
        "--watch",
        "--frames",
        "3",
        "--interval-ms",
        "5",
    ]);
    assert_eq!(cw, 0);
    let mut heights: Vec<usize> = ow
        .split("\x1b[H")
        .skip(1)
        .map(|fr| fr.trim_end().lines().count())
        .filter(|h| *h > 0) // the initial clear's empty piece
        .collect();
    heights.dedup();
    assert_eq!(
        heights.len(),
        1,
        "every watch frame is the SAME height: {heights:?}"
    );
    assert!(
        heights[0] >= lb,
        "padded up to the fixed height ({} >= {lb})",
        heights[0]
    );
}

/// Last 5 events, merged across the lineage's journals, newest first.
#[test]
fn event_feed_shows_last_five_merged() {
    let w = World::new("feed");
    w.arm();
    w.seed(1);
    let (o, e, c) = w.run(&["worker", "board", "--lineage", "line-a"]);
    assert_eq!(c, 0, "{o}{e}");
    assert!(o.contains("EVENTS"), "the feed section exists: {o}{e}");
    let idx = |needle: &str| o.find(needle).unwrap_or(usize::MAX);
    assert!(
        idx("unprovable") < idx("CARD-0001"),
        "newest first: eddy tick before the older bee run: {o}{e}"
    );
    let a = World::new("feedempty");
    a.arm();
    a.seed(0);
    let (oa, _, ca) = a.run(&["worker", "board", "--lineage", "line-a"]);
    assert_eq!(ca, 0);
    assert!(
        oa.contains("EVENTS"),
        "feed section renders even empty: {oa}"
    );
}

/// The unique mechanisms, live on the frame.
#[test]
fn mechanisms_are_visible() {
    let w = World::new("mech");
    w.arm();
    w.seed(1);
    let (o, e, c) = w.run(&["worker", "board", "--lineage", "line-a"]);
    assert_eq!(c, 0, "{o}{e}");
    assert!(o.contains("FOLD"), "fold section: {o}{e}");
    assert!(o.contains("78"), "fold threshold visible: {o}{e}");
    assert!(o.contains("warned"), "fold state visible: {o}{e}");
    assert!(o.contains("41%"), "context pct visible: {o}{e}");
    assert!(o.to_uppercase().contains("EDDY"), "eddy section: {o}{e}");
    assert!(
        o.contains("unprovable"),
        "the trail's statuses visible: {o}{e}"
    );
    assert!(o.contains("PACE WORK CARD-0239"), "pace verdict: {o}{e}");
}
