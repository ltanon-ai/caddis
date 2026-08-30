//! no_agent_timeout.rs — CARD-NO-TIMEOUT-1.
//!
//! RED-TEST: no harness/wrapper in this crate sends --timeout,
//! --max-time, or kill-after to an omp/claude/qpi/bee agent process.
//! Watchdog health probes in caddis-organs are NOT agent timeouts —
//! they are excluded by crate boundary (this test scans only the
//! caddis crate's own src/ and tests/*.md).

use std::fs;
use std::path::{Path, PathBuf};

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Tokens that kill an agent on a timer. None of these may appear
/// on a line that also spawns or waits for an agent.
const KILL_TIMERS: &[&str] = &["--timeout", "--max-time", "kill-after"];

/// Keywords that mark a line as an agent spawn/wait path. If a line
/// carries one of these AND a kill timer, the gate fires.
const AGENT_CONTEXT: &[&str] = &["agent", "bee spawn", "herdr", "spawn", "wait"];

fn scan_file(path: &Path) -> Vec<String> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => return vec![],
    };
    let mut hits = vec![];
    for (i, line) in text.lines().enumerate() {
        let is_agent = AGENT_CONTEXT.iter().any(|k| line.contains(k));
        if !is_agent {
            continue;
        }
        for timer in KILL_TIMERS {
            if line.contains(timer) {
                hits.push(format!("{}:{}: {}", path.display(), i + 1, line.trim()));
            }
        }
    }
    hits
}

fn collect_files(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let mut out = vec![];
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) == Some(ext) {
                out.push(p);
            }
        }
    }
    out
}

#[test]
fn no_agent_kill_timer_in_spawn_wait_paths() {
    let root = crate_root();
    let mut files = vec![];
    files.extend(collect_files(&root.join("src"), "rs"));
    files.extend(collect_files(&root.join("tests"), "md"));

    let mut hits = vec![];
    for f in &files {
        hits.extend(scan_file(f));
    }
    assert!(
        hits.is_empty(),
        "agent-kill timers found in spawn/wait paths (CARD-NO-TIMEOUT-1):\n{}",
        hits.join("\n")
    );
}
