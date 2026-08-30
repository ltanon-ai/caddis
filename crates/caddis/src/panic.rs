//! panic.rs — CARD-0311. The operator's halt: stop the bee, pause the
//! queue, snapshot the state, report. Hosts decide and halt — this is
//! the manual halt the beekeeper doctrine names. The pause marker is
//! enforced by take_task/keeper in CARD-0312; this organ is the halt
//! and the evidence.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::lineage;
use crate::worker_lock;

pub enum Error {
    Usage(String),
    Fail(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Usage(s) | Error::Fail(s) => write!(f, "{s}"),
        }
    }
}

/// The small state a post-mortem needs: receipts + queue + journals.
const SNAPSHOT_FILES: &[&str] = &[
    "ready.receipt",
    "arm.receipt",
    "handover.receipt",
    "claimed.receipt",
    "claimed.gen",
    "heartbeat.receipt",
    "ready.root",
    "queue",
    "pace.line",
    "worker.lock",
    "linger.lease",
    "armed-never-woke.lease",
    "bee.log",
    "phases.log",
    "scan.live",
    "talk/turns.jsonl",
];

pub fn run(args: &[String]) -> Result<(), Error> {
    let (id, rest) = lineage::take(args).map_err(Error::Usage)?;
    if !rest.is_empty() {
        return Err(Error::Usage(format!(
            "panic takes only --lineage (unknown: {})",
            rest.join(" ")
        )));
    }
    let dir = lineage::dir(&id).map_err(Error::Fail)?;
    if !dir.is_dir() {
        return Err(Error::Fail(format!("lineage {id} not found")));
    }
    let ts = now_secs();
    // Evidence first: the snapshot freezes the live state — including
    // the worker.lock that stop_bee is about to consume.
    let (snap, count) = snapshot(&dir, &id, ts).map_err(Error::Fail)?;
    let bee = stop_bee(&dir);
    fs::write(dir.join("panic.pause"), format!("ts={ts}\n"))
        .map_err(|e| Error::Fail(format!("write panic.pause: {e}")))?;
    println!("LINEAGE {id}");
    match bee {
        Some(pid) => println!("bee: stopped pid={pid}"),
        None => println!("bee: none (no fresh worker.lock)"),
    }
    println!("queue: paused (panic.pause)");
    println!("snapshot: {} ({count} files)", snap.display());
    println!(
        "resume: del {} — the keeper resumes",
        dir.join("panic.pause").display()
    );
    Ok(())
}

/// Kill the bee behind a FRESH worker.lock. A stale lock never kills:
/// pids recycle, and the operator's halt must not become a roulette.
fn stop_bee(dir: &Path) -> Option<u32> {
    if !worker_lock::is_busy(dir) {
        eprintln!("bee: none — worker.lock is absent or stale");
        return None;
    }
    let text = fs::read_to_string(dir.join("worker.lock")).ok()?;
    let pid = text
        .lines()
        .find(|l| l.starts_with("pid="))?
        .trim_start_matches("pid=")
        .parse::<u32>()
        .ok()?;
    if pid == 0 || pid == std::process::id() {
        eprintln!("bee: none — lock pid {pid} is ourselves or 0");
        return None; // never ourselves, never pid 0
    }
    if let Err(why) = kill_tree(pid) {
        eprintln!("bee: none — taskkill failed on pid {pid}: {why}");
        return None;
    }
    let _ = fs::remove_file(dir.join("worker.lock")); // swallow: best-effort-cleanup — its Guard died with it
    Some(pid)
}

/// Kill a whole process tree. Windows: taskkill /T (tree) /F (force).
/// Err carries taskkill's own words — the operator deserves them.
fn kill_tree(pid: u32) -> Result<(), String> {
    // A process still in initialization can refuse TERMINATE with
    // Access denied (observed live: taskkill on a freshly-spawned
    // python). Bounded retries; then absence counts as stopped.
    let mut last = String::new();
    for _ in 0..3 {
        let out = taskkill(pid);
        if out.is_ok() {
            return Ok(());
        }
        last = out.unwrap_err();
        if !pid_alive(pid) {
            return Ok(()); // already gone — the halt's goal is met
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
    Err(last)
}

fn taskkill(pid: u32) -> Result<(), String> {
    #[cfg(windows)]
    let out = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .output()
        .map_err(|e| format!("spawn taskkill: {e}"))?;
    #[cfg(not(windows))]
    let out = Command::new("kill")
        .args(["-9", &pid.to_string()])
        .output()
        .map_err(|e| format!("spawn kill: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
}

fn pid_alive(pid: u32) -> bool {
    #[cfg(windows)]
    let out = Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}")])
        .output();
    #[cfg(not(windows))]
    let out = Command::new("kill").args(["-0", &pid.to_string()]).output();
    match out {
        Ok(o) => String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()),
        Err(_) => true, // cannot prove absence — treat as alive, fail closed
    }
}

/// Copy the small state into <rotation>/panic/<id>-<ts>/. Absent files
/// are skipped; the copy count reports what survived.
fn snapshot(dir: &Path, id: &str, ts: u64) -> Result<(PathBuf, usize), String> {
    let root = dir
        .parent()
        .and_then(|l| l.parent())
        .ok_or_else(|| "lineage dir has no rotation parent".to_string())?
        .join("panic")
        .join(format!("{id}-{ts}"));
    fs::create_dir_all(&root).map_err(|e| format!("mkdir {}: {e}", root.display()))?;
    let mut count = 0;
    for name in SNAPSHOT_FILES {
        let src = dir.join(name);
        // swallow: best-effort-cleanup — unreadable snapshot source skipped
        if let Ok(text) = fs::read_to_string(&src) {
            let dst = root.join(name);
            if let Some(parent) = dst.parent() {
                let _ = fs::create_dir_all(parent); // swallow: best-effort-cleanup — talk/ may be absent
            }
            fs::write(&dst, text).map_err(|e| format!("snapshot {name}: {e}"))?;
            count += 1;
        }
    }
    Ok((root, count))
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
