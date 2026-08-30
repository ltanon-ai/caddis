//! sentinel.rs — CARD-0331. The audit organ in Rust (operator ruling:
//! the sentinel becomes a stable caddis organ). Argument-compatible
//! with the bee's call shape; slot-compatible with the push gate.
//! Post-processing lives in sentinel_post.rs (280-cap split). NO TIMER
//! on the agent: operator order 2026-08-16, verbatim in the bash era's
//! launch.sh — timers kill working agents and the empty result
//! overwrites a prior pass; --timeout is parsed and inert.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use crate::sentinel_engine::{home, launch_engine, save_envelope};

use crate::sentinel_post::write_records;

const DEFAULT_MODEL: &str = "grok-4.6";

pub fn cmd(args: &[String]) -> ExitCode {
    if args.first().map(String::as_str) == Some("model") {
        return model_cmd(&args[1..]);
    }
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("sentinel: {e}");
            ExitCode::from(1)
        }
    }
}

/// `caddis sentinel model [--set <id>]` — the model selection setting.
/// The state file (~/.caddis/sentinel.json) is ALSO the sentinel's
/// residence for the Pepe World warden room (feed honesty law: pepworld
/// reads ~/.caddis, never invents — the organ writes the truth here).
fn model_cmd(args: &[String]) -> ExitCode {
    match model_run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("sentinel: {e}");
            ExitCode::from(1)
        }
    }
}

fn model_run(args: &[String]) -> Result<(), String> {
    if let Some(i) = args.iter().position(|a| a == "--set") {
        let id = args.get(i + 1).ok_or("--set needs a model id")?;
        // CARD-0332: a model change must not wipe the warden-room
        // last-audit truth — carry the existing `last` object through.
        let last = crate::sentinel_post::state_last();
        crate::sentinel_post::write_state(Some(id), last.as_deref())?;
        println!("sentinel: model set to {id}");
        return Ok(());
    }
    println!("sentinel: model {}", current_model());
    Ok(())
}

/// The persisted model default (grok-4.6 unless ruled otherwise).
pub(crate) fn current_model() -> String {
    let path = state_path();
    // swallow: fail-safe-by-law — absent/unreadable state file means the default model
    if let Ok(text) = fs::read_to_string(&path) {
        let marker = "\"model\":\"";
        if let Some(a) = text.find(marker) {
            let a = a + marker.len();
            if let Some(len) = text[a..].find('"') {
                return text[a..a + len].to_string();
            }
        }
    }
    DEFAULT_MODEL.to_string()
}
pub(crate) fn state_path() -> PathBuf {
    home().join(".caddis").join("sentinel.json")
}
pub(crate) struct Opts {
    pub(crate) mode: String,
    pub(crate) cwd: Option<String>,
    pub(crate) target: Option<String>,
    model: Option<String>,
    pub(crate) task: String,
}

fn run(args: &[String]) -> Result<(), String> {
    let o = parse(args)?;
    if o.mode != "audit" {
        return Err(format!(
            "v1 mode is `audit` only (got {}); the bash sentinel owns the other modes until they port",
            o.mode
        ));
    }
    let repo = repo_dir(&o)?;
    let sha = git_line(&repo, &["rev-parse", "HEAD"])?;
    let scope = scope_files(&repo, &o)?;
    let model = o.model.clone().unwrap_or_else(current_model);
    let envelope = launch_engine(&repo, &o, &scope, &model)?;
    save_envelope(&envelope); // evidence for every run, pass or fail
    let report = crate::sentinel_post::extract_report(&envelope)?;
    write_records(&repo, &sha, &report, &model)?;
    println!(
        "sentinel: model={} verdict={} findings={}",
        model, report.verdict, report.findings
    );
    Ok(())
}

fn parse(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts {
        mode: String::new(),
        cwd: None,
        target: None,
        model: None,
        task: String::new(),
    };
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if value_flag(&mut o, args, a, &mut i)? {
            i += 1; // skip the VALUE — a mode-word value never reroutes
            continue;
        }
        if is_mode_word(a) {
            o.mode = a.to_string();
            i += 1;
        } else if a.starts_with("--") {
            return Err(format!("unknown flag {a}"));
        } else {
            o.task = a.to_string();
            i += 1;
        }
    }
    if o.mode.is_empty() {
        return Err("usage: caddis sentinel audit [--model ID] [--cwd DIR] [--target FILES] [--timeout N] <task>".into());
    }
    Ok(o)
}

/// The value-taking flags. True = consumed (caller continues).
fn value_flag(o: &mut Opts, args: &[String], a: &str, i: &mut usize) -> Result<bool, String> {
    match a {
        "--mode" => o.mode = arg(args, i)?,
        "--cwd" => o.cwd = Some(arg(args, i)?),
        "--target" => o.target = Some(arg(args, i)?),
        "--model" => o.model = Some(arg(args, i)?),
        // operator order 2026-08-16: the value is VALIDATED, NEVER
        "--timeout" => {
            arg(args, i)?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}

fn arg(args: &[String], i: &mut usize) -> Result<String, String> {
    *i += 1;
    args.get(*i)
        .cloned()
        .ok_or_else(|| format!("flag {} needs a value", args[*i - 1]))
}

/// The bash sentinel's mode set (positional words route as --mode).
fn is_mode_word(a: &str) -> bool {
    matches!(
        a,
        "patrol" | "openloop" | "closure" | "drift" | "verify" | "audit" | "free"
    )
}

fn repo_dir(o: &Opts) -> Result<PathBuf, String> {
    let d = match &o.cwd {
        Some(c) => PathBuf::from(c),
        None => std::env::current_dir().map_err(|e| format!("cwd: {e}"))?,
    };
    if !d.join(".git").exists() {
        return Err(format!("not a git repo: {}", d.display()));
    }
    Ok(d)
}

fn git_line(repo: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .map_err(|e| format!("git {}: {e}", args[0]))?;
    if !out.status.success() {
        return Err(format!("git {} failed", args[0]));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// The audit scope: --target's files, else origin/master...HEAD names.
fn scope_files(repo: &Path, o: &Opts) -> Result<String, String> {
    if let Some(t) = &o.target {
        return Ok(t.clone());
    }
    git_line(repo, &["diff", "--name-only", "origin/master...HEAD"])
}
