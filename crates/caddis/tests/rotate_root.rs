//! rotate_root.rs — CARD-0303/0304. Work-root stamping + single-flight
//! reservation. Own hermetic World (cwd + snapshot aware).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);
const KEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

struct World {
    home: PathBuf,
    line: PathBuf,
    snap: PathBuf,
}

impl World {
    fn new(tag: &str) -> Self {
        let n = SEQ.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!("caddis-root-{tag}-{n}"));
        let _ = fs::remove_dir_all(&root); // swallow: best-effort-cleanup — stale temp dir from a prior run
        let home = root.join("home");
        fs::create_dir_all(&home).unwrap();
        let snap = root.join("snapshot.json");
        fs::write(&snap, "{}").unwrap();
        Self {
            line: home.join(".caddis/rotation/lines/lin-t"),
            home,
            snap,
        }
    }

    fn run(&self, args: &[&str], cwd: Option<&Path>) -> (String, String, i32) {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_caddis"));
        cmd.args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("CADDIS_HMAC_KEY", KEY)
            .env("CADDIS_SKIP_WARDEN", "1")
            .env("CADDIS_HERDR_SNAPSHOT", &self.snap)
            .env_remove("HERDR_PANE_ID");
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }
        let out = cmd.output().expect("caddis must spawn");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    }

    fn ready(&self, cwd: Option<&Path>) -> (String, String, i32) {
        self.run(
            &[
                "rotate",
                "ready",
                "--kind",
                "omp",
                "--model",
                "m1",
                "--lineage",
                "lin-t",
            ],
            cwd,
        )
    }

    fn ready_pane(&self, pane: &str) -> (String, String, i32) {
        self.run(
            &[
                "rotate",
                "ready",
                "--kind",
                "omp",
                "--model",
                "m1",
                "--pane",
                pane,
                "--lineage",
                "lin-t",
            ],
            None,
        )
    }

    fn arm(&self) -> (String, String, i32) {
        self.run(&["rotate", "arm", "--lineage", "lin-t"], None)
    }
}

/// CARD-0303: ready stamps the work root the successor must spawn at —
/// the canonical physical cwd, no \\?\ device prefix.
#[test]
fn ready_stamps_work_root() {
    let w = World::new("stamp");
    let work = std::env::temp_dir().join("caddis-root-work");
    fs::create_dir_all(&work).unwrap();
    let (o, e, c) = w.ready(Some(&work));
    assert_eq!(c, 0, "ready: {o}{e}");
    let stamp = fs::read_to_string(w.line.join("ready.root")).expect("ready.root written");
    let norm = stamp.trim_end();
    assert!(!norm.starts_with(r"\\?\"), "no device prefix: {norm}");
    let canon = work.canonicalize().unwrap();
    let s = canon.to_string_lossy();
    let s = s
        .strip_prefix(r"\\?\UNC")
        .map(|r| format!(r"\\{r}"))
        .unwrap_or_else(|| s.strip_prefix(r"\\?\").unwrap_or(&s).to_string());
    assert_eq!(norm, s, "canonical physical path");
}

/// CARD-0303: arm names the stamped root as the spawn target.
#[test]
fn arm_prints_spawn_root() {
    let w = World::new("armroot");
    let (o, e, c) = w.ready(None);
    assert_eq!(c, 0, "ready: {o}{e}");
    let (o, e, c) = w.arm();
    assert_eq!(c, 0, "arm: {o}{e}");
    assert!(o.contains("root: "), "spawn root named: {o}");
}

/// CARD-0304: a live armed pane refuses a second ready — one rotation
/// in flight per lineage (the double-arm hole closes).
#[test]
fn ready_refused_while_arm_pane_live() {
    let w = World::new("reserve");
    let (o, e, c) = w.ready_pane("w1:p1");
    assert_eq!(c, 0, "first ready with pane: {o}{e}");
    let (o, e, c) = w.arm();
    assert_eq!(c, 0, "arm: {o}{e}");
    fs::write(
        &w.snap,
        r#"{"agents":[{"pane_id":"w1:p1","agent_status":"idle"}]}"#,
    )
    .unwrap();
    let (o, e, c) = w.ready(None);
    assert_ne!(c, 0, "second ready must refuse: {o}{e}");
    assert!(
        o.contains("refused") || e.contains("refused"),
        "names the refusal: {o}{e}"
    );
}

/// CARD-0304: pane gone -> re-ready is allowed (rotation completed).
#[test]
fn ready_allowed_after_pane_gone() {
    let w = World::new("regone");
    let (o, e, c) = w.ready_pane("w1:p1");
    assert_eq!(c, 0, "first ready: {o}{e}");
    let (_o, _e, _c) = w.arm();
    fs::write(&w.snap, "{}").unwrap();
    let (o, e, c) = w.ready(None);
    assert_eq!(c, 0, "re-ready after completion: {o}{e}");
}

/// CARD-0308: a LANDED claim spends the arm's reservation — the next
/// rotation may ready even while the old armed pane still lives (it is
/// the CURRENT owner, not an in-flight successor).
#[test]
fn ready_allowed_after_landed_claim_with_live_arm_pane() {
    let w = World::new("spent");
    let (o, e, c) = w.ready_pane("w1:p1");
    assert_eq!(c, 0, "first ready: {o}{e}");
    let (_o, _e, _c) = w.arm();
    fs::write(&w.snap, "{}").unwrap(); // predecessor pane gone -> verify promotes
    let (o, e, c) = w.run(&["rotate", "verify", "--lineage", "lin-t"], None);
    assert_eq!(c, 0, "verify claims: {o}{e}");
    fs::write(
        &w.snap,
        r#"{"agents":[{"pane_id":"w1:p1","agent_status":"idle"},{"pane_id":"w1:p2","agent_status":"idle"}]}"#,
    )
    .unwrap();
    let (o, e, c) = w.ready(None);
    assert_eq!(c, 0, "landed claim spends the reservation: {o}{e}");
}
