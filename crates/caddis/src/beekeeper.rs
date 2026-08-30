//! beekeeper.rs — CARD-0236. The waking-session HOST loop, ported from
//! `_worker_loop.py` (deleted by this card — never both live: two
//! beekeepers are two writers).
//!
//! The locked worker-bees spec says the worker is "not a daemon, not a
//! loop" — so the loop may NOT live in `caddis worker`. This is the
//! host, exactly as caddis-organs states the split: organs report and
//! prove; hosts decide and halt. The beekeeper owns only: read queue
//! head, call the worker tick, call scan when a card ran, redraw the
//! board, sleep. It holds NO law — the withheld halt is the organ
//! side's gate (CARD-0235); every future halt decision comes from
//! `eddy::verdict` (CARD-0237).
//!
//! `--once` runs exactly one cycle (tests, cron-style drivers); the
//! default loops at the given interval until killed. Foreground only:
//! no daemon claims, no background spawn.

use crate::worker_board;
use crate::worker_scan;

pub enum Error {
    Usage(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Usage(s) => write!(f, "{s}"),
        }
    }
}

const DEFAULT_INTERVAL_SECS: u64 = 3;

pub fn run(args: &[String]) -> Result<i32, Error> {
    let mut lineage = String::new();
    let mut once = false;
    let mut interval = DEFAULT_INTERVAL_SECS;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--lineage" => {
                lineage = args
                    .get(i + 1)
                    .cloned()
                    .ok_or_else(|| Error::Usage("missing --lineage value".into()))?;
                i += 2;
            }
            "--once" => {
                once = true;
                i += 1;
            }
            "--interval-secs" => {
                interval = args
                    .get(i + 1)
                    .and_then(|v| v.parse().ok())
                    .ok_or_else(|| Error::Usage("--interval-secs must be a number".into()))?;
                i += 2;
            }
            a => return Err(Error::Usage(format!("unknown argument {a}"))),
        }
    }
    if lineage.is_empty() {
        return Err(Error::Usage("beekeeper requires --lineage".into()));
    }
    loop {
        cycle(&lineage);
        if once {
            return Ok(0);
        }
        std::thread::sleep(std::time::Duration::from_secs(interval));
    }
}
/// One host cycle, in the Python loop's exact order (see module docs).
fn cycle(lineage: &str) {
    let dir = match crate::lineage::dir(lineage) {
        Ok(d) => d,
        Err(why) => {
            eprintln!("beekeeper: lineage {lineage}: {why}");
            return;
        }
    };
    // CARD-0262: bump the heartbeat at the top of every cycle so the
    // overnight watch sees PACE WORK even when bee.log is quiet mid-card.
    bump_heartbeat(&dir);
    let has_head = crate::pace::remaining_card(&dir).is_some();
    let _ = tick_cmd(lineage); // swallow: best-effort-telemetry — the tick's own output is the record; a failed cycle must not kill the host
    if has_head {
        let _ = scan_cmd(lineage); // swallow: best-effort-telemetry — scan failures print and the loop continues
                                   // CARD-0262: re-bump after observing bee output (PACE WORK).
        bump_heartbeat(&dir);
    }
    let _ = board_cmd(lineage); // swallow: best-effort-telemetry — the board is a view, never a gate
    ensure_dash_throttled(lineage);
}

/// CARD-0262: touch `<lineage-dir>/keeper.heartbeat` (mtime bump, no
/// content needed) so every organ can see the beekeeper is alive.
fn bump_heartbeat(dir: &std::path::Path) {
    let _ = std::fs::write(dir.join("keeper.heartbeat"), b""); // swallow: best-effort-telemetry — a failed heartbeat must not kill the host
}

/// CARD-0243: while the worker runs, the live view is ALWAYS in a
/// split pane of the same herdr workspace (kill-switch:
/// CADDIS_DASH_NO_ENSURE=1 — tests MUST set it). Throttled to one
/// check per minute.
fn ensure_dash_throttled(lineage: &str) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static LAST: AtomicU64 = AtomicU64::new(0);
    let now = caddis_organs::util::unix_ms();
    let last = LAST.load(Ordering::Relaxed);
    if now.saturating_sub(last) < 60_000 {
        return;
    }
    LAST.store(now, Ordering::Relaxed);
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| ".".into());
    crate::worker_dash::ensure_herdr_split(lineage, &cwd);
}

fn tick_cmd(lineage: &str) -> i32 {
    let args = vec![
        // worker::run takes the args AFTER the "worker" prefix (main
        // dispatch already stripped it).
        "tick".to_string(),
        "--lineage".to_string(),
        lineage.to_string(),
    ];
    match crate::worker::run(&args) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("beekeeper: worker tick: {e}");
            1
        }
    }
}

fn scan_cmd(lineage: &str) -> i32 {
    let args = vec![
        // worker_scan::run / worker_board::run take args AFTER their
        // own subcommand word (worker::run strips both levels).
        "--lineage".to_string(),
        lineage.to_string(),
    ];
    match worker_scan::run(&args) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("beekeeper: worker scan: {e}");
            1
        }
    }
}

fn board_cmd(lineage: &str) -> i32 {
    let args = vec!["--lineage".to_string(), lineage.to_string()];
    match worker_board::run(&args) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("beekeeper: worker board: {e}");
            1
        }
    }
}
