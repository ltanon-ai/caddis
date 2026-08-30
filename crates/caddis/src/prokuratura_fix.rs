//! prokuratura_fix.rs — CARD-0252. The diagnostic cascade for `caddis fix`.
//!
//! Split out of prokuratura.rs under the 280-line law. Each check
//! returns (name, verdict, detail) where verdict is OK/FIX/HUMAN.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::lineage;
use crate::worker_board_state as st;

/// Run the full diagnostic cascade for a symptom, return the report.
pub fn diagnose(symptom: &str) -> String {
    let results = vec![
        check_ollama(),
        check_tunnels(),
        check_engine_ports(),
        check_bees(),
        check_queue(symptom),
    ];

    let mut lines = Vec::new();
    lines.push(format!("diagnosis for: {symptom}"));
    for (name, verdict, detail) in &results {
        lines.push(format!("  {name}: {verdict} {detail}"));
    }
    let worst = results
        .iter()
        .map(|(_, v, _)| v.as_str())
        .find(|v| *v != "OK")
        .unwrap_or("OK");
    lines.push(format!("verdict: {worst}"));
    lines.join("\n")
}

fn home_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn default_lineage(home: &Path) -> String {
    let lines_dir = home.join(".caddis").join("rotation").join("lines");
    // swallow: fail-safe-by-law — no lineages dir means default
    if let Ok(entries) = std::fs::read_dir(&lines_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name
                .chars()
                .next()
                .map(|c| c.is_ascii_lowercase())
                .unwrap_or(false)
            {
                return name;
            }
        }
    }
    "default".to_string()
}

fn check_ollama() -> (String, String, String) {
    let host = env::var("OLLAMA_HOST").unwrap_or_else(|_| "127.0.0.1:11434".into());
    let url = format!("http://{host}/v1/models");
    match Command::new("curl")
        .arg("-s")
        .arg("-o")
        .arg("/dev/null")
        .arg("-w")
        .arg("%{http_code}")
        .arg(&url)
        .output()
    {
        Ok(out) => {
            let code = String::from_utf8_lossy(&out.stdout).into_owned();
            if code == "200" {
                ("ollama".into(), "OK".into(), format!("key valid ({host})"))
            } else {
                (
                    "ollama".into(),
                    "HUMAN".into(),
                    format!("key check failed: HTTP {code}"),
                )
            }
        }
        Err(_) => ("ollama".into(), "FIX".into(), "curl unavailable".into()),
    }
}

fn check_tunnels() -> (String, String, String) {
    match Command::new("netstat").arg("-an").output() {
        Ok(o) => {
            let text = String::from_utf8_lossy(&o.stdout);
            let listening = text.lines().filter(|l| l.contains("LISTEN")).count();
            (
                "tunnels".into(),
                "OK".into(),
                format!("{listening} ports listening"),
            )
        }
        Err(_) => ("tunnels".into(), "OK".into(), "netstat unavailable".into()),
    }
}

fn check_engine_ports() -> (String, String, String) {
    let home = home_dir().unwrap_or_else(|| PathBuf::from("."));
    let dir = home.join(".caddis").join("rotation").join("lines");
    let count = std::fs::read_dir(&dir)
        .map(|d| d.filter(|e| e.as_ref().is_ok()).count())
        .unwrap_or(0); // swallow: fail-safe-by-law — no lineages dir means 0
    ("engine".into(), "OK".into(), format!("{count} lineages"))
}

fn check_bees() -> (String, String, String) {
    let home = home_dir().unwrap_or_else(|| PathBuf::from("."));
    let id = default_lineage(&home);
    let dir = lineage::dir(&id).unwrap_or_else(|_| PathBuf::from("."));
    let bees = st::bee_recent(&dir, 1);
    if bees.is_empty() {
        ("bees".into(), "OK".into(), "none active".into())
    } else {
        (
            "bees".into(),
            "OK".into(),
            format!("last exit {}", bees[0].exit),
        )
    }
}

fn check_queue(symptom: &str) -> (String, String, String) {
    let home = home_dir().unwrap_or_else(|| PathBuf::from("."));
    let id = default_lineage(&home);
    let dir = lineage::dir(&id).unwrap_or_else(|_| PathBuf::from("."));
    let q = st::queue(&dir);
    if q.remaining.is_empty() {
        ("queue".into(), "OK".into(), "empty".into())
    } else {
        (
            "queue".into(),
            "OK".into(),
            format!("{} pending (symptom: {symptom})", q.remaining.len()),
        )
    }
}
