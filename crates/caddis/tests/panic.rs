//! panic.rs tests — CARD-0311. The operator's halt: stop the bee,
//! pause the queue, snapshot the state, report. Hermetic fake HOME.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static SEQ: AtomicU64 = AtomicU64::new(0);
const KEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

fn tmp(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("caddis-panic-{tag}-{n}"));
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
        let line = home.join(".caddis/rotation/lines/lin-p");
        fs::create_dir_all(&line).unwrap();
        Self { home, line }
    }

    /// Seed a quiet line: one queued card + a pace line.
    fn seed(&self) {
        fs::write(self.line.join("queue"), "CARD-9999\n").unwrap();
        fs::write(self.line.join("pace.line"), "pace=run\nts=1\n").unwrap();
    }

    fn panic(&self) -> (String, String, i32) {
        let out = Command::new(env!("CARGO_BIN_EXE_caddis"))
            .args(["panic", "--lineage", "lin-p"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("CADDIS_HMAC_KEY", KEY)
            .env("CADDIS_SKIP_WARDEN", "1")
            .output()
            .expect("caddis must spawn");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    }

    fn snap_dir(&self) -> PathBuf {
        let panic_root = self.home.join(".caddis/rotation/panic");
        let mut entries: Vec<_> = fs::read_dir(&panic_root)
            .expect("panic snapshot dir must exist")
            .map(|e| e.unwrap().path())
            .collect();
        entries.sort();
        entries.pop().expect("at least one snapshot")
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// CARD-0311: the halt reports every act, writes the pause marker and
/// the snapshot with the queue preserved.
#[test]
fn panic_pauses_snapshots_reports() {
    let w = World::new("quiet");
    w.seed();
    let (o, e, c) = w.panic();
    assert_eq!(c, 0, "panic: {o}{e}");
    assert!(w.line.join("panic.pause").is_file(), "pause marker missing");
    assert!(
        fs::read_to_string(w.line.join("panic.pause"))
            .unwrap()
            .starts_with("ts="),
        "pause marker must carry ts"
    );
    let snap = w.snap_dir();
    assert!(
        snap.join("queue").is_file(),
        "snapshot must preserve the queue"
    );
    assert!(o.contains("queue: paused"), "{o}");
    assert!(o.contains("snapshot:"), "{o}");
    assert!(o.contains("bee: none"), "quiet line has no bee: {o}");
    assert!(
        o.contains("resume:"),
        "must tell the operator how to resume: {o}"
    );
}

/// A real sleeper standing in for the bee: its pid goes into a FRESH
/// worker.lock — panic must kill it (the halt is real).
#[cfg(windows)]
fn spawn_sleeper() -> std::process::Child {
    Command::new("python")
        .args(["-c", "import time; time.sleep(120)"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("python sleeper must spawn")
}

#[cfg(windows)]
fn alive(child: &mut std::process::Child) -> bool {
    matches!(child.try_wait(), Ok(None))
}

/// CARD-0311: a fresh worker.lock names a live bee — panic stops it.
#[cfg(windows)]
#[test]
fn panic_stops_a_fresh_bee() {
    let w = World::new("fresh-bee");
    w.seed();
    let mut bee = spawn_sleeper();
    fs::write(
        w.line.join("worker.lock"),
        format!("pid={}\nts={}\n", bee.id(), now_secs()),
    )
    .unwrap();
    let (o, e, c) = w.panic();
    assert_eq!(c, 0, "panic: {o}{e}");
    assert!(
        o.contains("bee: stopped"),
        "{o}{e}\nlock was: {:?}",
        fs::read_to_string(w.line.join("worker.lock")).unwrap_or_else(|e| e.to_string())
    );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while alive(&mut bee) && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    assert!(!alive(&mut bee), "the bee must be dead after panic");
    assert!(
        !w.line.join("worker.lock").is_file(),
        "the killed lock is spent"
    );
    let _ = bee.kill();
}

/// CARD-0311 safety: a STALE lock pid is never killed — pids recycle;
/// "bee: none" and the sleeper survives.
#[cfg(windows)]
#[test]
fn panic_never_kills_a_stale_lock_pid() {
    let w = World::new("stale-lock");
    w.seed();
    let mut sleeper = spawn_sleeper();
    fs::write(
        w.line.join("worker.lock"),
        format!("pid={}\nts={}\n", sleeper.id(), now_secs() - 120),
    )
    .unwrap();
    let (o, e, c) = w.panic();
    assert_eq!(c, 0, "panic: {o}{e}");
    assert!(o.contains("bee: none"), "stale lock is not a bee: {o}");
    std::thread::sleep(std::time::Duration::from_secs(1));
    assert!(alive(&mut sleeper), "a stale pid must NEVER be killed");
    let _ = sleeper.kill();
    let _ = sleeper.wait();
}

/// CARD-0312: the pause marker freezes the work gate — the tick (and
/// every keeper cycle behind it) refuses a card while it stands.
#[test]
fn panic_pause_freezes_the_worker_tick() {
    let w = World::new("paused-tick");
    w.seed();
    fs::write(w.line.join("panic.pause"), "ts=1\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_caddis"))
        .args(["worker", "tick", "--lineage", "lin-p"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("HOME", &w.home)
        .env("USERPROFILE", &w.home)
        .env("CADDIS_HMAC_KEY", KEY)
        .env("CADDIS_SKIP_WARDEN", "1")
        .output()
        .expect("caddis must spawn");
    let o = String::from_utf8_lossy(&out.stdout).into_owned();
    let c = out.status.code().unwrap_or(-1);
    assert_eq!(c, 0, "a paused tick exits quiet: {o}");
    assert!(o.contains("PANIC PAUSED"), "the refusal must be said: {o}");
    assert_eq!(
        fs::read_to_string(w.line.join("queue")).unwrap(),
        "CARD-9999\n",
        "the pause must not touch the queue"
    );
}
