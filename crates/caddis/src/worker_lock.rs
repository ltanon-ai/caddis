//! worker_lock.rs — CARD-0216. Exclusive tick lock. No pid probes.
//! A held lock is FRESH; a leftover is STALE (ts older than 60s).
//! Steal only stale. Fresh lock = WORKER BUSY.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const STALE_SECS: u64 = 60;

pub struct Guard {
    path: PathBuf,
}

pub fn acquire(dir: &Path) -> Result<Guard, ()> {
    let path = dir.join("worker.lock");
    let mut f = match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(f) => f,
        Err(_) => {
            if !stale(&path) {
                return Err(());
            }
            let _ = fs::remove_file(&path); // swallow: best-effort-cleanup
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .map_err(|_| ())?
        }
    };
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let _ = writeln!(f, "pid={}\nts={ts}", std::process::id()); // swallow: best-effort-telemetry
    drop(f);
    Ok(Guard { path })
}

fn stale(path: &Path) -> bool {
    let Ok(text) = fs::read_to_string(path) else {
        return true; // unreadable = leftover
    };
    let Some(line) = text.lines().find(|l| l.starts_with("ts=")) else {
        return true; // no ts = leftover
    };
    let Ok(written) = line.trim_start_matches("ts=").parse::<u64>() else {
        return true;
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    now.saturating_sub(written) > STALE_SECS
}

/// True when a FRESH worker.lock exists — a bee run is in progress.
/// CARD-0256: the board's phase panel uses this to distinguish a
/// live bee (show the card) from an idle lineage (show idle).
pub fn is_busy(dir: &Path) -> bool {
    let path = dir.join("worker.lock");
    path.is_file() && !stale(&path)
}

impl Drop for Guard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path); // swallow: best-effort-cleanup
    }
}
