//! fault_matrix.rs — CARD-0318/0319. The quorum-S8 fault-injection
//! matrix, restart + lease rows: every organ fails CLOSED on torn or
//! tampered state — no empty-key turn minting, no mac-deaf counting,
//! no spawn into a ghost root, no silent generation re-fence. Hermetic
//! HOME; the live bag is never touched.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);
const KEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

fn tmp(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("caddis-fm-{tag}-{n}"));
    let _ = fs::remove_dir_all(&p); // swallow: best-effort-cleanup — stale temp dir from a prior run
    fs::create_dir_all(&p).unwrap();
    p
}

struct World {
    home: PathBuf,
    line: PathBuf,
}

impl World {
    fn new(tag: &str) -> Self {
        let root = tmp(tag);
        let home = root.join("home");
        fs::create_dir_all(&home).unwrap();
        Self {
            line: home.join(".caddis/rotation/lines/lin-f"),
            home,
        }
    }

    fn run(&self, args: &[&str], envs: &[(&str, &str)], drop_key: bool) -> (String, String, i32) {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_caddis"));
        cmd.args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("CADDIS_SKIP_WARDEN", "1")
            .env_remove("HERDR_PANE_ID")
            .env_remove("CADDIS_HERDR_BIN");
        if drop_key {
            cmd.env_remove("CADDIS_HMAC_KEY");
        } else {
            cmd.env("CADDIS_HMAC_KEY", KEY);
        }
        for (k, v) in envs {
            cmd.env(k, v);
        }
        let out = cmd.output().expect("caddis must spawn");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    }

    fn seed(&self) {
        let (o, e, c) = self.run(
            &[
                "rotate",
                "ready",
                "--kind",
                "omp",
                "--model",
                "m1",
                "--lineage",
                "lin-f",
            ],
            &[],
            false,
        );
        assert_eq!(c, 0, "ready: {o}{e}");
        let (o, e, c) = self.run(&["rotate", "arm", "--lineage", "lin-f"], &[], false);
        assert_eq!(c, 0, "arm: {o}{e}");
    }
}

/// A fake herdr .cmd shim: answers `pane split` with a pane id (the
/// restart_spawn.rs pattern) — spawn rows only, windows-gated.
#[cfg(windows)]
fn write_fake_herdr(root: &Path) -> PathBuf {
    let bin = root.join("fake-herdr.cmd");
    let body = "@echo off\r\nif \"%1\"==\"pane\" if \"%2\"==\"split\" echo {\"result\":{\"pane\":{\"pane_id\":\"wS:p9\"}}}\r\nif \"%1\"==\"pane\" if \"%2\"==\"list\" echo {\"result\":{\"panes\":[]}}\r\n";
    fs::write(&bin, body).unwrap();
    bin
}

/// Row 1: no hmac.key and no env key -> the talk organ FAILS CLOSED;
/// an empty-key turn (mac of the zero key) must never be minted.
#[test]
fn missing_key_talk_post_fails_closed() {
    let w = World::new("nokey");
    w.seed();
    let _ = fs::remove_file(w.line.join("hmac.key")); // swallow: absent-when-env-key-was-used — the drop_key run is the subject
    let (o, e, c) = w.run(
        &[
            "restart",
            "talk",
            "--lineage",
            "lin-f",
            "--post",
            "finding",
            "gate probe E:/evidence.md",
        ],
        &[],
        true,
    );
    assert_ne!(c, 0, "missing key must fail the post: {o}{e}");
    assert!(
        !w.line.join("talk/turns.jsonl").exists(),
        "no turn appended under an empty key"
    );
}

/// Row 2: a mac-tampered turn is EXCLUDED from the retire-gate count
/// and NAMED — tamper-evidence bites where the gate reads it.
#[test]
fn tampered_turn_excluded_and_named() {
    let w = World::new("tamper");
    w.seed();
    let (o, e, c) = w.run(
        &[
            "restart",
            "talk",
            "--lineage",
            "lin-f",
            "--post",
            "finding",
            "gate probe E:/evidence.md",
        ],
        &[],
        false,
    );
    assert_eq!(c, 0, "post: {o}{e}");
    let turns = w.line.join("talk/turns.jsonl");
    let raw = fs::read_to_string(&turns).unwrap();
    let last = raw.lines().last().unwrap().to_string();
    let at = last.rfind("\"mac\":\"").expect("mac field");
    let mut line = last.clone();
    let mac_char = if line.ends_with('0') { '1' } else { '0' };
    line.pop(); // closing quote
    line.pop(); // last mac hex char
    line.push(mac_char);
    line.push('"');
    let _ = at;
    fs::write(&turns, line + "\n").unwrap();
    let (o, e, c) = w.run(&["restart", "enter", "--lineage", "lin-f"], &[], false);
    assert_eq!(c, 0, "enter: {o}{e}");
    assert!(o.contains("unverified"), "tampered turn is named: {o}{e}");
    assert!(
        !o.contains("unanswered finding"),
        "a tampered turn never holds the gate: {o}{e}"
    );
}

/// Row 3 (fence): a torn turns tail is skipped, counts unchanged.
#[test]
fn torn_turns_tail_skipped() {
    let w = World::new("torn");
    w.seed();
    let (o, e, c) = w.run(
        &[
            "restart",
            "talk",
            "--lineage",
            "lin-f",
            "--post",
            "finding",
            "gate probe E:/evidence.md",
        ],
        &[],
        false,
    );
    assert_eq!(c, 0, "post: {o}{e}");
    let turns = w.line.join("talk/turns.jsonl");
    let mut raw = fs::read_to_string(&turns).unwrap();
    raw.push_str("{\"role\":\"pre");
    fs::write(&turns, raw).unwrap();
    let (o, e, c) = w.run(&["restart", "enter", "--lineage", "lin-f"], &[], false);
    assert_eq!(c, 0, "torn tail never panics: {o}{e}");
    assert!(
        o.contains("1 unanswered finding"),
        "the valid finding still counts: {o}{e}"
    );
}

/// Row 4: a ready.root pointing at a nonexistent dir — spawn REFUSES
/// before splitting; a stub herdr "success" must not boot a ghost seat.
#[cfg(windows)]
#[test]
fn bad_ready_root_spawn_refuses() {
    let w = World::new("ghost");
    w.seed();
    let root = w.home.parent().unwrap().to_path_buf();
    fs::write(w.line.join("ready.root"), "Z:\\no\\such\\dir\\ghost").unwrap();
    let bin = write_fake_herdr(&root);
    let (o, e, c) = w.run(
        &["restart", "spawn", "--lineage", "lin-f"],
        &[("CADDIS_HERDR_BIN", bin.to_str().unwrap())],
        false,
    );
    assert_ne!(c, 0, "ghost root must refuse: {o}{e}");
    assert!(
        format!("{o}{e}").contains("ready.root"),
        "names the root: {o}{e}"
    );
}

/// Row 5 (fence): a torn heartbeat proves nothing — the split pane's
/// wake is absent and armed-never-woke is written for the doctor.
#[cfg(windows)]
#[test]
fn torn_heartbeat_wakes_nobody() {
    let w = World::new("wake");
    w.seed();
    let root = w.home.parent().unwrap().to_path_buf();
    fs::write(w.line.join("heartbeat.receipt"), "pane=wS:p").unwrap();
    let bin = write_fake_herdr(&root);
    let (o, e, c) = w.run(
        &["restart", "spawn", "--lineage", "lin-f"],
        &[("CADDIS_HERDR_BIN", bin.to_str().unwrap())],
        false,
    );
    assert_eq!(c, 0, "spawn still succeeds: {o}{e}");
    assert!(
        o.contains("armed-never-woke"),
        "torn heartbeat is not a wake: {o}{e}"
    );
    assert!(w.line.join("armed-never-woke.lease").is_file());
}
