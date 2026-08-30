//! drain_lineage.rs — CARD-0133. Hermetic HOME. Never ~/.herdr or ~/.claude.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);
const KEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

fn tmp(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("caddis-lin-{tag}-{n}"));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

struct World {
    home: PathBuf,
    snap: PathBuf,
}

impl World {
    fn new(tag: &str) -> Self {
        let root = tmp(tag);
        let home = root.join("home");
        fs::create_dir_all(&home).unwrap();
        let snap = root.join("snapshot.json");
        fs::write(&snap, "{}").unwrap();
        Self { home, snap }
    }

    fn run(&self, args: &[&str]) -> (String, String, i32) {
        let mut argv: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
        let sub = argv.get(1).map(|s| s.as_str()).unwrap_or("");
        if matches!(sub, "ready" | "arm" | "verify") && !argv.iter().any(|s| s == "--lineage") {
            argv.push("--lineage".into());
            argv.push("lin-t".into());
        }
        let out = Command::new(env!("CARGO_BIN_EXE_caddis"))
            .args(&argv)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("CADDIS_HMAC_KEY", KEY)
            .env("CADDIS_HERDR_SNAPSHOT", &self.snap)
            .output()
            .expect("caddis must spawn");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    }

    fn run_env(&self, args: &[&str], extra: &[(&str, &str)]) -> (String, String, i32) {
        let argv: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
        let sub = argv.get(1).map(|s| s.as_str()).unwrap_or("");
        let argv = if matches!(sub, "ready" | "arm" | "verify")
            && !argv.iter().any(|s| s == "--lineage")
        {
            let mut v = argv;
            v.push("--lineage".into());
            v.push("lin-t".into());
            v
        } else {
            argv
        };
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_caddis"));
        cmd.args(&argv)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("CADDIS_HMAC_KEY", KEY)
            .env("CADDIS_HERDR_SNAPSHOT", &self.snap);
        for (k, v) in extra {
            cmd.env(k, v);
        }
        let out = cmd.output().expect("caddis must spawn");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    }

    fn arm_pane(&self, pane: &str) {
        let (o, e, c) = self.run(&[
            "rotate", "ready", "--kind", "omp", "--model", "m1", "--pane", pane,
        ]);
        assert_eq!(c, 0, "ready: {o}{e}");
        let (o, e, c) = self.run(&["rotate", "arm"]);
        assert_eq!(c, 0, "arm: {o}{e}");
    }
}

fn snap_working(pane: &str) -> String {
    format!(r#"{{"agents":[{{"pane_id":"{pane}","agent_status":"working"}}]}}"#)
}

fn snap_idle(pane: &str) -> String {
    format!(r#"{{"agents":[{{"pane_id":"{pane}","agent_status":"idle"}}]}}"#)
}

/// CARD-0300: an idle-but-live predecessor is LIVE. Today this fails the
/// assertion (verify exits 0 and restamps) — the RED proof for E1.
#[test]
fn arm_pane_idle_fails_drain() {
    let w = World::new("idle");
    fs::write(&w.snap, snap_idle("w3J:p1")).unwrap();
    w.arm_pane("w3J:p1");
    let (o, e, c) = w.run_env(&["rotate", "verify"], &[("HERDR_PANE_ID", "w3J:p9")]);
    assert_ne!(
        c, 0,
        "idle arm pane is live: verify must fail closed: {o}{e}"
    );
    let arm = fs::read_to_string(w.home.join(".caddis/rotation/lines/lin-t/arm.receipt"))
        .expect("arm receipt readable");
    assert!(
        !arm.contains("pane=w3J:p9"),
        "no restamp may occur while the predecessor lives: {arm}"
    );
}

/// CARD-0300: a stale state source is Unknown, never Clean — E1 must not
/// be reborn one layer up via a frozen session.json.
#[test]
fn stale_snapshot_is_unknown_not_clean() {
    let w = World::new("stale");
    fs::write(&w.snap, "{}").unwrap();
    w.arm_pane("w3J:p1");
    let (o, e, c) = w.run_env(
        &["rotate", "verify"],
        &[("CADDIS_DRAIN_FRESHNESS_SECS", "0")],
    );
    assert_ne!(c, 0, "stale snapshot must be Unknown: {o}{e}");
    assert!(
        o.contains("stale") || e.contains("stale"),
        "must name staleness: {o}{e}"
    );
}

/// CARD-0300 regression guard: pane absent from a FRESH source stays
/// Clean — the presence fix must not over-fire.
#[test]
fn arm_pane_absent_still_clean() {
    let w = World::new("gone");
    fs::write(&w.snap, "{}").unwrap();
    w.arm_pane("w3J:p1");
    let (o, e, c) = w.run(&["rotate", "verify"]);
    assert_eq!(c, 0, "absent pane + fresh source must promote: {o}{e}");
}

#[test]
fn other_pane_working_does_not_fail_this_rotation() {
    let w = World::new("other");
    fs::write(&w.snap, snap_working("w36:pP")).unwrap();
    w.arm_pane("w3J:p1");
    let (o, e, c) = w.run(&["rotate", "verify"]);
    assert_eq!(c, 0, "other pane must not fail this lineage: {o}{e}");
}

#[test]
fn arm_pane_working_still_fails_drain() {
    let w = World::new("mine");
    fs::write(&w.snap, snap_working("w3J:p1")).unwrap();
    w.arm_pane("w3J:p1");
    let (o, e, c) = w.run(&["rotate", "verify"]);
    assert_ne!(c, 0, "this pane working must fail drain: {o}{e}");
}

// --- CARD-0310: the production drain is a live daemon query ---

impl World {
    /// Production-path run: NO CADDIS_HERDR_SNAPSHOT — the drain must
    /// consult the herdr the organs would (CADDIS_HERDR_BIN fakes it).
    fn run_live(&self, args: &[&str], bin: &str) -> (String, String, i32) {
        let mut argv: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
        let sub = argv.get(1).map(|s| s.as_str()).unwrap_or("");
        if matches!(sub, "ready" | "arm" | "verify") && !argv.iter().any(|s| s == "--lineage") {
            argv.push("--lineage".into());
            argv.push("lin-t".into());
        }
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_caddis"));
        cmd.args(&argv)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("CADDIS_HMAC_KEY", KEY)
            .env("CADDIS_HERDR_BIN", bin);
        let out = cmd.output().expect("caddis must spawn");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    }

    fn line(&self, name: &str) -> PathBuf {
        self.home.join(".caddis/rotation/lines/lin-t").join(name)
    }
}

/// A fake herdr .cmd answering `pane list` with the exact JSON shape
/// the daemon prints (pane_id tokens included).
#[cfg(windows)]
fn write_fake_pane_list(root: &std::path::Path, pane: Option<&str>) -> PathBuf {
    let bin = root.join("fake-herdr-list.cmd");
    let body = match pane {
        Some(p) => format!(
            "@echo off\r\necho {{\"id\":\"cli:pane:list\",\"result\":{{\"panes\":[{{\"pane_id\":\"{p}\",\"agent_status\":\"idle\",\"workspace_id\":\"w1\"}}]}}}}\r\n"
        ),
        None => "@echo off\r\necho {\"id\":\"cli:pane:list\",\"result\":{\"panes\":[]}}\r\n".into(),
    };
    fs::write(&bin, body).unwrap();
    bin
}

/// CARD-0310: production truth is the live daemon pane list — the
/// armed pane PRESENT at idle must fail verify (the watch3 rotation
/// proved session.json reads Clean over a live predecessor).
#[cfg(windows)]
#[test]
fn live_daemon_presence_fails_verify() {
    let w = World::new("live-presence");
    w.arm_pane("w1:p1");
    let root = w.snap.parent().unwrap().to_path_buf();
    let bin = write_fake_pane_list(&root, Some("w1:p1"));
    let (o, e, c) = w.run_live(&["rotate", "verify"], bin.to_str().unwrap());
    assert_ne!(c, 0, "verify must fail on live predecessor: {o}");
    assert!(e.contains("drain fail"), "stderr: {e}");
    assert!(
        w.line("linger.lease").is_file(),
        "linger.lease must be written"
    );
}

/// CARD-0310: an unreachable herdr is Unknown, never Clean — and never
/// a linger (no live agent was seen). Portable: caddis answers an
/// unknown `pane` subcommand with exit 2 and empty stdout.
#[test]
fn live_daemon_unreachable_is_unknown() {
    let w = World::new("live-unreachable");
    w.arm_pane("w1:p1");
    let (o, e, c) = w.run_live(&["rotate", "verify"], env!("CARGO_BIN_EXE_caddis"));
    assert_ne!(c, 0, "verify must fail closed: {o}");
    assert!(e.contains("drain unknown"), "stderr: {e}");
    assert!(
        !w.line("linger.lease").is_file(),
        "no linger without a live agent"
    );
}

/// CARD-0310 regression guard: predecessor absent from the daemon pane
/// list -> Clean, verify proceeds.
#[cfg(windows)]
#[test]
fn live_daemon_absent_stays_clean() {
    let w = World::new("live-absent");
    w.arm_pane("w1:p1");
    let root = w.snap.parent().unwrap().to_path_buf();
    let bin = write_fake_pane_list(&root, None);
    let (o, e, c) = w.run_live(&["rotate", "verify"], bin.to_str().unwrap());
    assert_eq!(c, 0, "verify: {o}{e}");
    assert!(o.contains("lease:"), "{o}");
}
