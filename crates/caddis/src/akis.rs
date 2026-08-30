//! akis.rs — CARD-0271. rust-analyzer as an external LANE (fail-open).
//!
//! rust-analyzer is a MACHINE TOOL the organ TALKS TO (subprocess
//! lane) — never a code dependency. Lane up => diagnostics over
//! stdio JSON-RPC; lane down => exit 0, skipped (dynamic-availability
//! law). Nits NEVER gate — Error rows are ADVISORY to the bee, written
//! to the card's akis.jsonl. std-only: hand-rolled over Child stdio
//! (Content-Length framing) — no tokio, no deps.

use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::akis_json::{self, Json};

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

const HELP: &str = "USAGE: caddis akis --card <id> [--file <path>...]\n";

/// One-shot: probe the lane, drive LSP over the touched files, write
/// akis.jsonl. Exit 0 always (advisory) — lane down OR Error rows
/// never gate.
pub fn run(args: &[String]) -> Result<i32, Error> {
    let (_card, files) = parse_args(args)?;
    let cmd = lane_cmd();
    if !probe(&cmd) {
        println!("akis: lane down, skipped");
        return Ok(0);
    }
    let rows = lsp_session(&cmd, &files)?;
    write_jsonl(&rows);
    let errs = rows.iter().filter(|r| r.severity == "Error").count();
    println!("akis: {} rows ({} Error, advisory)", rows.len(), errs);
    Ok(0)
}

fn parse_args(args: &[String]) -> Result<(String, Vec<PathBuf>), Error> {
    let mut card = None;
    let mut files = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if let Some(v) = eat(a, "--card", args, &mut i) {
            set_once(&mut card, v, "--card")?;
        } else if let Some(v) = eat(a, "--file", args, &mut i) {
            files.push(PathBuf::from(v));
        } else {
            return Err(Error::Usage(format!("unknown argument {a}\n{HELP}")));
        }
    }
    let card = card.ok_or_else(|| Error::Usage("akis requires --card <id>\n".into()))?;
    Ok((card, files))
}

fn eat(a: &str, flag: &str, args: &[String], i: &mut usize) -> Option<String> {
    if a == flag {
        let v = args.get(*i + 1)?.clone();
        *i += 2;
        Some(v)
    } else if let Some(v) = a.strip_prefix(&format!("{flag}=")) {
        *i += 1;
        Some(v.to_string())
    } else {
        None
    }
}

fn set_once(slot: &mut Option<String>, v: String, flag: &str) -> Result<(), Error> {
    if slot.is_some() {
        return Err(Error::Usage(format!("duplicate {flag}")));
    }
    *slot = Some(v);
    Ok(())
}

fn lane_cmd() -> Vec<String> {
    if let Some(raw) = env::var_os("CADDIS_AKIS_BIN") {
        let argv: Vec<String> = raw
            .to_string_lossy()
            .split_whitespace()
            .map(str::to_string)
            .collect();
        if !argv.is_empty() {
            return argv;
        }
    }
    vec!["rust-analyzer".to_string()]
}

fn probe(cmd: &[String]) -> bool {
    Command::new(&cmd[0])
        .args(&cmd[1..])
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false) // swallow: fail-safe-by-law — unspawnable lane = down
}

fn lsp_session(cmd: &[String], files: &[PathBuf]) -> Result<Vec<Row>, Error> {
    if files.is_empty() {
        return Ok(Vec::new());
    }
    let mut child = Command::new(&cmd[0])
        .args(&cmd[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| Error::Fail(format!("akis spawn: {e}")))?;
    let rows = {
        let mut stdin = child.stdin.take().unwrap();
        let _ = send(&mut stdin, &initialize()); // swallow: fail-safe-by-law — lane write is best-effort
        for f in files {
            let text = fs::read_to_string(f).unwrap_or_default(); // swallow: fail-safe-by-law — unreadable file -> empty text
            let _ = send(&mut stdin, &did_open(f, &text)); // swallow: fail-safe-by-law — lane write is best-effort
        }
        drop(stdin); // close stdin -> server drains and exits
        let stdout = child.stdout.take().unwrap();
        collect_diagnostics(stdout)
    };
    let _ = child.wait(); // swallow: best-effort-telemetry — reap the lane child
    Ok(rows)
}

fn collect_diagnostics<R: Read>(r: R) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut br = BufReader::new(r);
    while let Some(bytes) = read_frame(&mut br) {
        if let Some(json) = akis_json::parse(&bytes) {
            collect_from(&json, &mut rows);
        }
    }
    rows
}

fn collect_from(json: &Json, rows: &mut Vec<Row>) {
    if json.get("method").and_then(|j| j.as_str()) != Some("textDocument/publishDiagnostics") {
        return;
    }
    let Some(params) = json.get("params") else {
        return;
    };
    let uri = params.get("uri").and_then(|u| u.as_str()).unwrap_or("");
    let Some(diags) = params.get("diagnostics").and_then(|d| d.as_array()) else {
        return;
    };
    for d in diags {
        rows.push(normalize(d, uri));
    }
}

fn read_frame<R: BufRead>(r: &mut R) -> Option<Vec<u8>> {
    let mut len = None;
    let mut line = String::new();
    loop {
        line.clear();
        let n = r.read_line(&mut line).ok()?;
        if n == 0 {
            return None;
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        if let Some(v) = trimmed.strip_prefix("Content-Length:") {
            len = v.trim().parse().ok();
        }
    }
    let len = len?;
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).ok()?;
    Some(buf)
}

/// One normalized diagnostic row: {file, line, code, severity, message}.
fn normalize(d: &Json, uri: &str) -> Row {
    let line = d
        .get("range")
        .and_then(|r| r.get("start"))
        .and_then(|s| s.get("line"))
        .and_then(|l| l.as_i64())
        .unwrap_or(0);
    let sev = d.get("severity").and_then(|s| s.as_i64()).unwrap_or(1);
    let severity = match sev {
        2 => "Warning",
        3 => "Info",
        4 => "Hint",
        _ => "Error", // LSP: 1=Error .. 4=Hint; unknown => Error
    }
    .to_string();
    let code = d.get("code").map(|c| c.display()).unwrap_or_default();
    let message = d
        .get("message")
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_string();
    Row {
        file: uri.strip_prefix("file://").unwrap_or(uri).to_string(),
        line: line as u64,
        code,
        severity,
        message,
    }
}

struct Row {
    file: String,
    line: u64,
    code: String,
    severity: String,
    message: String,
}

fn write_jsonl(rows: &[Row]) {
    let mut s = String::new();
    for r in rows {
        s.push_str(&format!(
            "{{\"file\":\"{}\",\"line\":{},\"code\":\"{}\",\"severity\":\"{}\",\"message\":\"{}\"}}\n",
            esc(&r.file),
            r.line,
            esc(&r.code),
            esc(&r.severity),
            esc(&r.message)
        ));
    }
    let _ = fs::write("akis.jsonl", s); // swallow: best-effort-telemetry
}

fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn send<W: Write>(w: &mut W, json: &str) -> Result<(), Error> {
    let bytes = json.as_bytes();
    let header = format!("Content-Length: {}\r\n\r\n", bytes.len());
    w.write_all(header.as_bytes())
        .and_then(|_| w.write_all(bytes))
        .and_then(|_| w.flush())
        .map_err(|e| Error::Fail(format!("akis write: {e}")))
}

fn initialize() -> String {
    "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"processId\":0,\"rootUri\":null,\"capabilities\":{}}}".into()
}

fn did_open(path: &Path, text: &str) -> String {
    let uri = format!("file://{}", path.display());
    format!(
        "{{\"jsonrpc\":\"2.0\",\"method\":\"textDocument/didOpen\",\"params\":{{\"textDocument\":{{\"uri\":\"{}\",\"languageId\":\"rust\",\"version\":1,\"text\":\"{}\"}}}}}}",
        esc(&uri),
        esc(text)
    )
}
