//! worker_done.rs — CARD-0218 + 0235 + 0317. Done is EARNED: bee exit
//! 0 AND the card's mechanical checks green (a host-minted
//! prove-receipt may cover a check the gate env cannot run — E5),
//! else withheld; consecutive withheld dispatches take the line out of
//! rotation at the eddy threshold (watchdog::DEFAULT_MAX_FAILURES).
use std::path::Path;

use caddis_organs::util::json_escape;

/// What the Done-When gate concluded for one dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DoneOutcome {
    /// Every check passed; the queue line is marked `done`.
    Marked(usize),
    /// Done withheld, with the structural why (names the failing
    /// check — CARD-0260: a verdict that cannot say which check
    /// failed is not evidence).
    Withheld(String),
}

/// CARD-0218 gate. Withheld is NOT failure of the bee — it is the
/// absence of proof that the card's own contract held.
/// CARD-0260: each check runs through a shell with PATH discovery
/// when the direct spawn cannot resolve argv0. Zero behavior change
/// when the direct spawn works.
pub(crate) fn verify_done_when(dir: &Path, card: &str) -> DoneOutcome {
    let Some(checks) = done_when_checks(card) else {
        let why = if card_file_exists(card) {
            "no $ checks in Done-When".to_string()
        } else {
            "no card file".to_string()
        };
        println!("{card} done withheld: {why}");
        return DoneOutcome::Withheld(why);
    };
    let total = checks.len();
    let mut passed = 0usize;
    let mut by_receipt = 0usize;
    let mut failed = None;
    for c in &checks {
        match run_check(c) {
            CheckResult::Passed => passed += 1,
            CheckResult::Failed(why) => {
                if crate::prove::receipt_covers(dir, &c.join(" ")) {
                    by_receipt += 1;
                } else if failed.is_none() {
                    failed = Some(why);
                }
            }
        }
    }
    if passed + by_receipt == total {
        mark_queue_line(dir, card, "done");
        let label = if by_receipt > 0 {
            format!(" ({by_receipt} by prove-receipt)")
        } else {
            String::new()
        };
        println!("DW-OK {}/{total}{label}", passed + by_receipt);
        crate::soul_writer::write_joy(dir, card);
        DoneOutcome::Marked(passed + by_receipt)
    } else {
        let why = failed.unwrap_or_else(|| "checks failed".to_string());
        println!("DW-FAIL {passed}/{total}: {why}");
        DoneOutcome::Withheld(why)
    }
}

/// One Done-When check's outcome.
enum CheckResult {
    Passed,
    Failed(String), // names the check — CARD-0260
}

/// Run one Done-When check. CARD-0260: the beekeeper's bare PATH may
/// not carry `python`/`bash`, so try direct, then a login shell
/// (`sh -lc`/`bash -lc`, PATH via ~/.profile). A check that cannot
/// RESOLVE is still a fail — just an honest-labelled one.
fn run_check(argv: &[String]) -> CheckResult {
    let argv0 = &argv[0];
    let via = |ok: bool, label: &str| {
        if ok {
            CheckResult::Passed
        } else {
            CheckResult::Failed(format!("check failed: `{argv0}` ({label}) exited non-zero"))
        }
    };
    if let Some(ok) = spawn_direct(argv) {
        return via(ok, "direct");
    }
    if let Some(ok) = spawn_shell(argv) {
        return via(ok, "via shell");
    }
    CheckResult::Failed(format!(
        "check unresolved: `{argv0}` not found (direct or shell PATH discovery)"
    ))
}

/// Direct spawn. `Some(bool)` = command resolved; `None` = argv0 not
/// found on PATH.
fn spawn_direct(argv: &[String]) -> Option<bool> {
    std::process::Command::new(&argv[0])
        .args(&argv[1..])
        .status()
        .ok() // swallow: fail-safe-by-law — Err means argv0 unresolvable, mapped to None for the shell fallback
        .map(|s| s.code().unwrap_or(1) == 0)
}

/// Shell fallback: a login shell discovers PATH via `~/.profile`.
/// `Some(bool)` = shell resolved and the inner command ran; `None` =
/// neither shell on PATH or the command was still unresolvable (127).
fn spawn_shell(argv: &[String]) -> Option<bool> {
    let cmd = argv.join(" ");
    for shell in ["sh", "bash"] {
        // swallow: fail-safe-by-law — shell spawn Err means the shell itself is unresolvable, try the next
        if let Ok(s) = std::process::Command::new(shell)
            .arg("-lc")
            .arg(&cmd)
            .status()
        {
            let code = s.code().unwrap_or(1);
            if code == 127 {
                continue; // inner command not found — try next shell
            }
            return Some(code == 0);
        }
    }
    None
}

/// CARD-0235: `_card_<num>.md` exists in the dispatch cwd. A line
/// without one is refused at arm time — its done is unprovable by
/// construction, so re-running it can never converge.
pub(crate) fn card_file_exists(card: &str) -> bool {
    card_num(card)
        .map(|n| Path::new(&format!("_card_{n}.md")).is_file())
        .unwrap_or(false)
}

/// `_card_<num>.md` in cwd; `# Done-When` bullets `- $ <cmd…>` are
/// checks. No mechanical checks = done withheld (prose is not proof).
fn done_when_checks(card: &str) -> Option<Vec<Vec<String>>> {
    let num = card_num(card)?;
    let text = std::fs::read_to_string(format!("_card_{num}.md")).ok()?;
    let checks = scan_done_when(&text);
    if checks.is_empty() {
        None
    } else {
        Some(checks)
    }
}

/// The lineage's host-owned eddy tick trail (the organ's ONLY input).
fn eddy_trail(dir: &Path) -> std::path::PathBuf {
    dir.join("eddy.jsonl")
}

/// Record one dispatch outcome as an eddy tick, then let the ONE
/// verdict decide. Returns true when the line was taken out of
/// rotation. The HOST halts, the ORGAN judges (spec v3).
pub(crate) fn withheld_gate(dir: &Path, card: &str, outcome: &DoneOutcome) -> bool {
    let run_id = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "lineage".into());
    let (status, why): (caddis_organs::eddy::StatusClass, &str) = match outcome {
        DoneOutcome::Marked(_) => (caddis_organs::eddy::StatusClass::Ok, ""),
        DoneOutcome::Withheld(why) => (caddis_organs::eddy::StatusClass::Unprovable, why),
    };
    let trail = eddy_trail(dir);
    let seq = caddis_organs::eddy::read_ticks(&trail).len() as u64 + 1;
    let tick = caddis_organs::eddy::Tick {
        run_id,
        seq,
        payload_hash: caddis_organs::eddy::stable_hash(card),
        status_class: status,
        outcome_hash: caddis_organs::eddy::stable_hash(why),
        cache_read: 0,
        cache_write: 0,
        latency_ms: 0,
        ts_ms: caddis_organs::util::unix_ms(),
        resume_after: None,
        artifact_hash: 0,
        page: 0,
    };
    if caddis_organs::eddy::record_tick(&trail, &tick).is_err() {
        return false; // an unrecorded dispatch judges nothing; the next one retries
    }
    match caddis_organs::eddy::verdict(&caddis_organs::eddy::read_ticks(&trail)) {
        caddis_organs::eddy::Verdict::UnprovableDone { streak } => {
            println!("WITHHELD-HALT {card} after {streak} dispatches: {why}");
            file_withheld_blocker(dir, card, why, streak);
            crate::soul_writer::write_pain(dir, card, why);
            // No state to reset: the tick trail IS the state. A failed
            // queue mark leaves the streak at threshold in the organ,
            // so the next dispatch retries the halt, not the burn.
            mark_queue_line(dir, card, "withheld")
        }
        _ => false,
    }
}

/// One JSONL blocker the operator must resolve (blocker.rs record
/// shape; the file is host-owned in the lineage dir).
fn file_withheld_blocker(dir: &Path, card: &str, why: &str, count: u32) {
    let record = format!(
        "{{\"source\":\"worker:{}\",\"reason\":\"{}\",\"ts\":\"{}\"}}",
        json_escape(card),
        json_escape(&format!(
            "done withheld {count}x: {why} — line halted out of rotation"
        )),
        crate::receipt::timestamp()
    );
    // swallow: best-effort-telemetry — an unwritable blocker file must not undo the queue halt
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("blockers.jsonl"))
    {
        use std::io::Write;
        let _ = writeln!(f, "{record}"); // swallow: best-effort-telemetry — the queue mark is the halt, the blocker is the flag
    }
}
fn card_num(card: &str) -> Option<&str> {
    let num = card.strip_prefix("CARD-")?;
    if num.is_empty() || !num.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(num)
}

/// `- $ <cmd…>` bullet lines inside the `# Done-When` section, each split
/// to argv; shorter than two words is not a runnable check.
fn scan_done_when(text: &str) -> Vec<Vec<String>> {
    let mut in_section = false;
    let mut checks = Vec::new();
    for line in text.lines() {
        if line.starts_with("# ") {
            in_section = line.trim_end() == "# Done-When";
            continue;
        }
        if !in_section {
            continue;
        }
        if let Some(body) = line.strip_prefix("- $ ") {
            let argv: Vec<String> = body.split_whitespace().map(str::to_string).collect();
            if argv.len() >= 2 {
                checks.push(argv);
            }
        }
    }
    checks
}
/// Prefix a queue line (`done ` / `withheld `) — both take it out of
/// rotation; only `done` claims the work held. Returns true when the
/// line was actually marked (the caller's halt decision depends on it).
fn mark_queue_line(dir: &Path, card: &str, prefix: &str) -> bool {
    let Ok(text) = std::fs::read_to_string(dir.join("queue")) else {
        return false;
    };
    let mut out = String::new();
    let mut marked = false;
    for raw in text.lines() {
        let line = raw.trim_start();
        let already = line.starts_with("done ") || line.starts_with("withheld ");
        if !already && (line == card || line.starts_with(&format!("{card} "))) {
            out.push_str(prefix);
            out.push(' ');
            marked = true;
        }
        out.push_str(raw);
        out.push('\n');
    }
    if !marked {
        return false;
    }
    let _ = std::fs::write(dir.join("queue"), out); // swallow: checked-elsewhere — write-fail keeps count at threshold, retried
    true
}
