//! ledger.rs — CARD-LEDGER-DB-3. `caddis ledger orient`.
//!
//! Reads `~/.caddis/ledger.sqlite` via the shared Python helper. No rusqlite.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const HELP: &str = "\
USAGE: caddis ledger orient [--project NAME] [--since 90d|24h|UNIX]
";
const PY: &str = include_str!("../../../tools/ledger_sqlite.py");

pub fn run(args: &[String]) -> Result<(), String> {
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    if matches!(a.as_slice(), ["orient", "--help"] | ["--help"]) {
        eprint!("{HELP}");
        return Ok(());
    }
    let flags = match a.as_slice() {
        [] | ["orient"] => &[][..],
        ["orient", tail @ ..] => tail,
        _ => return Err(HELP.trim_end().to_string()),
    };
    let (project, since) = parse_flags(flags)?;
    orient(project, since)
}

fn parse_flags<'a>(flags: &'a [&str]) -> Result<(Option<&'a str>, Option<&'a str>), String> {
    let mut project = None;
    let mut since = None;
    let mut i = 0;
    while i < flags.len() {
        match flags[i] {
            "--project" if i + 1 < flags.len() && !flags[i + 1].is_empty() => {
                project = Some(flags[i + 1]);
                i += 2;
            }
            "--since" if i + 1 < flags.len() && !flags[i + 1].is_empty() => {
                since = Some(flags[i + 1]);
                i += 2;
            }
            _ => return Err(HELP.trim_end().to_string()),
        }
    }
    Ok((project, since))
}

fn sqlite_path() -> PathBuf {
    // swallow: fail-safe-by-law
    if let Ok(p) = env::var("CADDIS_WARDEN_LEDGER_SQLITE") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    caddis_dir().join("ledger.sqlite")
}

fn caddis_dir() -> PathBuf {
    // swallow: fail-safe-by-law
    if let Ok(h) = env::var("CADDIS_HOME") {
        if !h.is_empty() {
            return PathBuf::from(h);
        }
    }
    dirs_home().join(".caddis")
}

fn dirs_home() -> PathBuf {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn cwd_project() -> String {
    env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "unknown".into())
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
    let tool = db
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("ledger_tool.py");
    match fs::read_to_string(&tool) {
        Ok(cur) if cur == PY => {}
        _ => fs::write(&tool, PY).map_err(|e| format!("write ledger_tool.py: {e}"))?,
    }
    Ok(tool)
}

fn orient(project: Option<&str>, since: Option<&str>) -> Result<(), String> {
    let project = project.unwrap_or("").to_string();
    let project = if project.is_empty() {
        cwd_project()
    } else {
        project
    };
    let db = sqlite_path();
    if !db.exists() {
        return Err(format!("no sqlite ledger at {}", db.display()));
    }
    let tool = ensure_tool(&db)?;
    let mut cmd = python()?;
    cmd.arg(&tool)
        .arg("orient")
        .arg("--db")
        .arg(&db)
        .arg("--project")
        .arg(&project);
    if let Some(s) = since {
        cmd.arg("--since").arg(s);
    }
    let out = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("spawn python: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "orient failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    print!("{}", String::from_utf8_lossy(&out.stdout));
    Ok(())
}
