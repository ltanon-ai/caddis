//! sentinel_post.rs — CARD-0331. The audit organ's post leg: extract
//! the report from the CLI envelope, write the evidence file and the
//! verify slot. FAIL-CLOSED: a failed audit writes NOTHING — never
//! destroy a prior verdict (the bash-era launch.sh lesson, structural).
//! The slot key is the DELIBERATE TWIN of push_gate_git.repo_slug.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::lease::write_atomic;
use crate::sentinel_engine::runs_dir;

pub(crate) struct Report {
    pub verdict: String,
    pub findings: usize,
}

/// The CLI envelope: `structuredOutput` is the report; `type:error` is
/// fail-closed. (sentinel_envelope.py's contract, the v1 slice — string
/// parsing like every organ here; no json crate in the TCB.)
pub(crate) fn extract_report(envelope: &str) -> Result<Report, String> {
    if field(envelope, "type").as_deref() == Some("error") {
        return Err(format!(
            "CLI error: {}",
            field(envelope, "message").unwrap_or_default()
        ));
    }
    // grok's real envelope puts the schema object in TEXT (escaped),
    // not structuredOutput — both legs of sentinel_envelope.py's law.
    if let Some(start) = envelope.find("\"structuredOutput\"") {
        let body = &envelope[start..];
        let verdict = field(body, "verdict").ok_or_else(|| "report has no verdict".to_string())?;
        return Ok(Report {
            verdict,
            findings: count_findings(body),
        });
    }
    report_from_text(envelope)
}

/// The text leg: grok's real envelope carries the schema object
/// ESCAPED inside the text field (sentinel_envelope.py's fallback).
fn report_from_text(envelope: &str) -> Result<Report, String> {
    let no_verdict = || "report has no verdict".to_string();
    let text = field_unescaped(envelope, "text").ok_or_else(no_verdict)?;
    let obj = last_json_with(&text, "\"verdict\"").ok_or_else(no_verdict)?;
    let verdict = field(obj, "verdict").ok_or_else(no_verdict)?;
    Ok(Report {
        verdict,
        findings: count_findings(obj),
    })
}

/// The last balanced {...} containing `needle` — the model's final
/// answer object inside the envelope's text field.
fn last_json_with<'a>(text: &'a str, needle: &str) -> Option<&'a str> {
    let v_at = text.rfind(needle)?;
    let open = text[..v_at].rfind('{')?;
    let close = walk_to_close(text, open)?;
    Some(&text[open..close])
}

/// Index just past the `}` closing the object opened at `open`.
/// Escaped pairs are opaque; braces inside strings do not count.
fn walk_to_close(text: &str, open: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 0usize;
    let mut in_str = false;
    let mut i = open;
    while i < text.len() {
        let c = bytes[i];
        if c == b'\\' {
            i += 2;
            continue;
        }
        if c == b'"' {
            in_str = !in_str;
        } else if !in_str && c == b'{' {
            depth += 1;
        } else if !in_str && c == b'}' {
            depth -= 1;
            if depth == 0 {
                return Some(i + 1);
            }
        }
        i += 1;
    }
    None
}

/// `"key":"value"` with the JSON escape set unfolded (the envelope's
/// text field carries the model's answer escaped).
fn field_unescaped(line: &str, key: &str) -> Option<String> {
    let a = value_start(line, key)?;
    let mut out = String::new();
    let mut chars = line[a..].chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(other) => out.push(other),
                None => return None,
            },
            _ => out.push(c),
        }
    }
    None
}

/// `"key": "value"` from a flat JSON slice — whitespace-tolerant after
/// the colon (grok pretty-prints its envelope), no escapes in our fields.
fn field(line: &str, key: &str) -> Option<String> {
    let a = value_start(line, key)?;
    let b = line[a..].find('"')? + a;
    Some(line[a..b].to_string())
}

/// Index just inside the opening quote of `key`'s string value.
fn value_start(line: &str, key: &str) -> Option<usize> {
    let marker = format!("\"{key}\":");
    let a = line.find(&marker)? + marker.len();
    let skipped = line[a..].trim_start();
    skipped
        .starts_with('"')
        .then_some(line.len() - skipped.len() + 1)
}

fn count_findings(body: &str) -> usize {
    let Some(a) = body.find("\"findings\":") else {
        return 0;
    };
    let rest = body[a..].trim_start_matches(|c: char| c != '[');
    let rest = rest.strip_prefix('[').unwrap_or(rest);
    let end = rest.find("\"cannot_verify\"").unwrap_or(rest.len());
    // top-level finding objects only: each carries exactly one severity
    rest[..end].matches("\"severity\":").count()
}

/// Write the evidence file, then the slot, then the state file (the
/// Pepe World warden-room residence — pepworld reads ~/.caddis, never
/// invents; this IS the sentinel's truth). Pass iff CLEAR + no findings.
pub(crate) fn write_records(
    repo: &Path,
    sha: &str,
    report: &Report,
    model: &str,
) -> Result<(), String> {
    let clean = report.verdict == "CLEAR" && report.findings == 0;
    let ts = unix_now();
    let name = format!("{ts}-caddis-sentinel.json");
    let evidence = format!(
        "{{\"repo\":\"{}\",\"verdict\":\"{}\",\"findings\":{}}}\n",
        json_esc(&repo.display().to_string()),
        report.verdict,
        report.findings
    );
    write_atomic(&runs_dir(), &name, evidence.as_bytes())
        .map_err(|e| format!("write evidence: {e}"))?;
    let audit_file = runs_dir().join(&name).display().to_string();
    let record = format!(
        "{{\"sha\":\"{sha}\",\"verdict\":\"{}\",\"ts\":{ts},\"repo\":\"{}\",\"run_id\":\"caddis-sentinel\",\"audit_file\":\"{}\"}}\n",
        if clean { "pass" } else { "fail" },
        json_esc(&repo.display().to_string()),
        json_esc(&audit_file)
    );
    let slot = format!("last-verify-{}.json", repo_slug(repo));
    // A Windows-path origin yields a slug with separators; the gate's
    // os.path.join reads the SAME nested path, so the dirs must exist.
    let slot_path = runs_dir().join(&slot);
    let parent = slot_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(runs_dir);
    std::fs::create_dir_all(&parent).map_err(|e| format!("mkdir slot parent: {e}"))?;
    write_atomic(
        &parent,
        slot_path.file_name().unwrap().to_string_lossy().as_ref(),
        record.as_bytes(),
    )
    .map_err(|e| format!("write slot: {e}"))?;
    let last = format!(
        "{{\"verdict\":\"{}\",\"findings\":{},\"sha\":\"{sha}\",\"ts\":{ts},\"repo\":\"{}\",\"slot\":\"{}\"}}",
        report.verdict,
        report.findings,
        json_esc(&repo.display().to_string()),
        json_esc(&slot)
    );
    write_state(Some(model), Some(&last))
}

/// The state file's `last` object, if present (model --set carries it).
pub(crate) fn state_last() -> Option<String> {
    let text = fs::read_to_string(crate::sentinel::state_path()).ok()?;
    let a = text.find("\"last\":")? + 7;
    let rest = &text[a..];
    let end = rest.find('}')? + a + 1;
    Some(rest[..end - a].to_string())
}

/// The sentinel state file: model selection + last audit truth.
pub(crate) fn write_state(model: Option<&str>, last: Option<&str>) -> Result<(), String> {
    let path = crate::sentinel::state_path();
    let prev_model = crate::sentinel::current_model();
    let m = model.unwrap_or(&prev_model);
    let body = match last {
        Some(l) => format!("{{\"model\":\"{m}\",\"last\":{l}}}\n"),
        None => format!("{{\"model\":\"{m}\"}}\n"),
    };
    let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    std::fs::create_dir_all(&dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
    let name = path.file_name().unwrap().to_string_lossy().to_string();
    write_atomic(&dir, &name, body.as_bytes()).map_err(|e| format!("write state: {e}"))?;
    Ok(())
}

/// Backslashes and quotes escaped for a JSON string body (Windows
/// paths are full of them; the gate json.loads the record).
fn json_esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// DELIBERATE TWIN of push_gate_git.repo_slug (and bee's writer): the
/// origin URL's last segment, .git stripped, lowercased; the repo dir's
/// basename as fallback. Drift between twins is detected by the gate's
/// own tests importing bee's version.
fn repo_slug(repo: &Path) -> String {
    // swallow: fail-safe-by-law — a git failure falls back to the dir basename
    if let Ok(out) = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(repo)
        .output()
    {
        if out.status.success() {
            let url = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let mut name = url
                .split(':')
                .next_back()
                .unwrap_or("")
                .trim_end_matches('/')
                .to_string();
            if let Some(seg) = name.rsplit('/').next() {
                name = seg.to_string();
            }
            if let Some(stripped) = name.strip_suffix(".git") {
                name = stripped.to_string();
            }
            if !name.is_empty() {
                return name.to_lowercase();
            }
        }
    }
    repo.file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_else(|| "repo".into())
}

#[allow(dead_code)] // slot twin probe kept for the tests' future use
fn slot_path(repo: &Path) -> PathBuf {
    runs_dir().join(format!("last-verify-{}.json", repo_slug(repo)))
}
