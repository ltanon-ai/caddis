//! sqlite_ledger.rs — CARD-LEDGER-SPLIT-1. Tool-call writer INSERTs SQLite.
//!
//! TCB stays rusqlite-free: Python stdlib sqlite3 does the file I/O.
//! Production JSONL (`~/.caddis/warden-ledger.jsonl`) is frozen. JSONL
//! append survives only when `CADDIS_WARDEN_LEDGER` points at a test file.

use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use caddis_core::envelope::Envelope;
use caddis_core::ledger::Ledger;

const PY: &str = include_str!("../../../tools/ledger_sqlite.py");

fn caddis_dir() -> PathBuf {
    let home = env::var("USERPROFILE")
        .or_else(|_| env::var("HOME"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".caddis")
}

pub fn sqlite_path() -> PathBuf {
    // swallow: fail-safe-by-law
    if let Ok(p) = env::var("CADDIS_WARDEN_LEDGER_SQLITE") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    if let Some(jsonl) = jsonl_test_path() {
        return jsonl.with_file_name("ledger.sqlite");
    }
    caddis_dir().join("ledger.sqlite")
}

fn jsonl_test_path() -> Option<PathBuf> {
    let raw = env::var("CADDIS_WARDEN_LEDGER").ok()?;
    if raw.is_empty() {
        return None;
    }
    let p = PathBuf::from(&raw);
    let default = caddis_dir().join("warden-ledger.jsonl");
    if p == default {
        None
    } else {
        Some(p)
    }
}

pub fn jsonl_test_override() -> Option<PathBuf> {
    jsonl_test_path()
}

fn cwd_project() -> String {
    // swallow: fail-safe-by-law
    if let Ok(p) = env::var("CADDIS_PROJECT") {
        if !p.is_empty() {
            return p;
        }
    }
    env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "unknown".into())
}

fn path_from_body(body: &str) -> String {
    body.split('|').nth(2).unwrap_or("").to_string()
}

fn python() -> Result<Command, String> {
    for bin in ["python", "python3"] {
        let mut c = Command::new(bin);
        c.arg("-c")
            .arg("import sqlite3")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if c.status().map(|s| s.success()).unwrap_or(false) {
            return Ok(Command::new(bin));
        }
    }
    let mut c = Command::new("py");
    c.arg("-3")
        .arg("-c")
        .arg("import sqlite3")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if c.status().map(|s| s.success()).unwrap_or(false) {
        let mut out = Command::new("py");
        out.arg("-3");
        return Ok(out);
    }
    Err("no python with sqlite3".into())
}

fn ensure_tool(db: &Path) -> Result<PathBuf, String> {
    let parent = db.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    let tool = parent.join("ledger_tool.py");
    match fs::read_to_string(&tool) {
        Ok(cur) if cur == PY => {}
        _ => fs::write(&tool, PY).map_err(|e| format!("write ledger_tool.py: {e}"))?,
    }
    Ok(tool)
}

fn insert_sqlite(env: &Envelope) -> Result<u64, String> {
    let db = sqlite_path();
    let tool = ensure_tool(&db)?;
    let payload = serde_lite(env);
    let mut cmd = python()?;
    let mut child = cmd
        .arg(&tool)
        .arg("insert")
        .arg("--db")
        .arg(&db)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn python: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(payload.as_bytes())
            .map_err(|e| format!("stdin: {e}"))?;
    }
    let out = child
        .wait_with_output()
        .map_err(|e| format!("python wait: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "sqlite insert failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    s.parse::<u64>()
        .map_err(|_| format!("sqlite insert seq not a number: {s}"))
}

fn serde_lite(env: &Envelope) -> String {
    let path = path_from_body(&env.body);
    let proj = cwd_project();
    format!(
        "{{\"ts\":{},\"project\":{},\"from\":{},\"type\":{},\"body\":{},\"path\":{},\"cwd_project\":{}}}",
        json_str(&env.ts),
        json_str(&proj),
        json_str(&env.from),
        json_str(&env.r#type),
        json_str(&env.body),
        json_str(&path),
        json_str(&proj),
    )
}

fn json_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// CARD-0321: production writes BOTH stores — the readers' JSONL and
/// the sqlite query store; a reader is never forked from a writer (the
/// live split: two binary vintages wrote different stores and every
/// reader went blind). CADDIS_WARDEN_LEDGER set = TEST isolation:
/// JSONL only, no sqlite side-effects. Either production store failing
/// refuses the row. Supersedes DB-2's premature JSONL freeze.
pub fn commit_open(env: &Envelope) -> u64 {
    let path = crate::identity::ledger_path();
    let seq = match Ledger::open(&path).and_then(|mut led| led.append(env)) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("caddis-warden: ledger append failed: {e}");
            return 0;
        }
    };
    if jsonl_test_path().is_none() {
        if let Err(e) = insert_sqlite(env) {
            eprintln!("caddis-warden: sqlite insert failed: {e}");
            return 0;
        }
    }
    seq
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn env_row(body: &str) -> Envelope {
        Envelope {
            v: 1,
            id: "wardtest000000001".into(),
            idem_key: "abcdabcdabcdabcd".into(),
            r#type: "tool.read".into(),
            from: "omp".into(),
            to: "warden".into(),
            body: body.into(),
            ts: "1".into(),
        }
    }

    #[test]
    fn sqlite_override_wins() {
        env::set_var("CADDIS_WARDEN_LEDGER_SQLITE", "Z:/tmp/x.sqlite");
        let p = sqlite_path();
        env::remove_var("CADDIS_WARDEN_LEDGER_SQLITE");
        assert_eq!(p, PathBuf::from("Z:/tmp/x.sqlite"));
    }

    #[test]
    fn insert_does_not_touch_jsonl() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = env::temp_dir().join(format!("caddis-sql-{stamp}"));
        fs::create_dir_all(&dir).unwrap();
        let db = dir.join("ledger.sqlite");
        let jsonl = dir.join("frozen.jsonl");
        fs::write(&jsonl, "{\"seq\":1}\n").unwrap();
        env::set_var("CADDIS_WARDEN_LEDGER_SQLITE", &db);
        env::remove_var("CADDIS_WARDEN_LEDGER");
        let before = fs::read_to_string(&jsonl).unwrap();
        let n = insert_sqlite(&env_row("allow|read|C:/x/caddis-workshop/a.rs||")).unwrap();
        env::remove_var("CADDIS_WARDEN_LEDGER_SQLITE");
        let after = fs::read_to_string(&jsonl).unwrap();
        assert!(n >= 1);
        assert_eq!(before, after);
        assert!(db.exists());
    }
}
