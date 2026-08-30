//! sentinel_engine.rs — CARD-0332. The audit organ's engine leg,
//! split from sentinel.rs under the 280 law (skaidyk): grok resolution,
//! headless read-only launch with NO TIMER on the agent (operator order
//! 2026-08-16), pid-unique prompt files, and the envelope-evidence save
//! that holds for failed engines too.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::sentinel::Opts;

const DEFAULT_HOME: &str = "E:/ClaudeToolbox/sentinel";
/// The raw envelope lands in runs/ ALWAYS — a failed parse is
/// diagnosable, never a mystery (the bash era had to be re-run blind).
pub(crate) fn save_envelope(envelope: &str) {
    let dir = runs_dir();
    // swallow: best-effort-telemetry — evidence saving never blocks the audit
    if std::fs::create_dir_all(&dir).is_ok() {
        let name = format!("{}-caddis-sentinel-envelope.json", std::process::id());
        let _ = std::fs::write(dir.join(name), envelope); // swallow: best-effort-telemetry
    }
}

fn grok_bin() -> PathBuf {
    std::env::var_os("CADDIS_SENTINEL_GROK_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".grok/bin/grok.exe"))
}

pub(crate) fn home() -> PathBuf {
    PathBuf::from(
        std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .unwrap_or_else(|_| ".".into()),
    )
}

fn sentinel_home() -> PathBuf {
    std::env::var_os("CADDIS_SENTINEL_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_HOME))
}

pub(crate) fn runs_dir() -> PathBuf {
    std::env::var_os("CADDIS_SENTINEL_RUNS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("E:/ClaudeToolbox/bee/runs"))
}

/// Spawn the engine headless, read-only, no subagents/web. NO TIMER.
pub(crate) fn launch_engine(
    repo: &Path,
    o: &Opts,
    scope: &str,
    model: &str,
) -> Result<String, String> {
    // Debug/replay seam: a captured envelope replaces the engine run.
    if let Some(path) = std::env::var_os("CADDIS_SENTINEL_ENVELOPE_FILE") {
        return fs::read_to_string(path).map_err(|e| format!("envelope file: {e}"));
    }
    let sh = sentinel_home();
    let schema = fs::read_to_string(sh.join("schema.json"))
        .map_err(|e| format!("schema.json: {e} (home={})", sh.display()))?;
    let prompt = format!(
        "Read-only code audit. Posture: you may READ the repo at {} only.\n\
         Charter (operator): report ONLY defects introduced, activated, or made \
         reachable by this patch (origin/master...HEAD); ignore pre-existing debt.\n\
         Scope: {}\nTask: {}\n\
         Emit the audit object per the schema: verdict CLEAR|FINDINGS, summary, \
         findings (evidenced only), cannot_verify. Do not emit a plan; emit the verdict.",
        repo.display(),
        scope,
        o.task
    );
    fs::create_dir_all(runs_dir()).map_err(|e| format!("runs dir: {e}"))?;
    // pid-unique: overlapping audits must never feed each other prompts
    let prompt_file = runs_dir().join(format!("{}-caddis-sentinel-prompt.txt", std::process::id()));
    fs::write(&prompt_file, &prompt).map_err(|e| format!("prompt file: {e}"))?;
    let bin = grok_bin();
    let out = Command::new(&bin)
        .args(["-m", model, "--prompt-file"])
        .arg(&prompt_file)
        .args(["--output-format", "json", "--json-schema"])
        .arg(&schema)
        .args([
            "--permission-mode",
            "dontAsk",
            "--sandbox",
            "read-only",
            "--no-subagents",
            "--disable-web-search",
            "--cwd",
        ])
        .arg(repo)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("engine spawn {}: {e}", bin.display()))?;
    if !out.status.success() {
        // CARD-0332: a failed engine's output is still evidence — save
        // it before refusing (the envelope law holds for failures too).
        let text = format!(
            "{{\"type\":\"error\",\"message\":\"engine exited {}\\nstdout:{}\\nstderr:{}\"}}",
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        save_envelope(&text);
        return Err(format!("engine exited {}", out.status.code().unwrap_or(-1)));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}
