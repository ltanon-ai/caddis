//! worker.rs — CARD-0216. One-shot tick; controls process bees.
//! SPEC-caddis-worker-bees-2026-08-28.md@LOCKED-v1. Not a chair.

use std::path::Path;

use crate::bee;
use crate::lineage;
use crate::pace;
use crate::worker_lock;

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

pub fn run(args: &[String]) -> Result<i32, Error> {
    let sub = args
        .first()
        .ok_or_else(|| Error::Usage("worker requires tick|board".into()))?;
    match sub.as_str() {
        "tick" => tick_cmd(&args[1..]),
        "board" => crate::worker_board::run(&args[1..])
            .map(|_| 0)
            .map_err(map_board),
        "scan" => crate::worker_scan::run(&args[1..])
            .map(|_| 0)
            .map_err(map_scan),
        "phase" => crate::worker_fsm::run(&args[1..])
            .map(|_| 0)
            .map_err(map_fsm),
        _ => Err(Error::Usage(format!("unknown worker subcommand {sub}"))),
    }
}

fn map_fsm(e: crate::worker_fsm::Error) -> Error {
    match e {
        crate::worker_fsm::Error::Usage(s) => Error::Usage(s),
        crate::worker_fsm::Error::Fail(s) => Error::Fail(s),
    }
}

fn tick_cmd(args: &[String]) -> Result<i32, Error> {
    let (id, rest) = lineage::take(args).map_err(Error::Usage)?;
    if rest.iter().any(|a| a == "--harness") {
        return Err(Error::Usage(
            "worker tick has no --harness; ARM kind only".into(),
        ));
    }
    if let Some(a) = rest.first() {
        return Err(Error::Usage(format!("unknown argument {a}")));
    }
    tick(&id)
}

fn tick(id: &str) -> Result<i32, Error> {
    let dir = lineage::dir(id).map_err(Error::Fail)?;
    // CARD-0312: a panic pause freezes all WORK — the heartbeat and
    // the board keep living (views, never gates).
    if dir.join("panic.pause").is_file() {
        println!("PANIC PAUSED — resume: del panic.pause");
        return Ok(0);
    }
    let verdict = pace::beat(id, None).map_err(map_pace)?;
    println!("{verdict}");
    let _lock = match worker_lock::acquire(&dir) {
        Ok(g) => g,
        Err(()) => {
            println!("WORKER BUSY");
            return Ok(0);
        }
    };
    let Some(card) = verdict.strip_prefix("PACE WORK ") else {
        return Ok(0);
    };
    let Some(argv) = take_task(&dir, card)? else {
        return Ok(0);
    };
    run_task(id, &dir, card, argv)
}

/// Queue gates between "there is a card" and "it runs": split-brain
/// check, TASK handoff, empty argv. None = nothing to execute now.
fn take_task(dir: &std::path::Path, card: &str) -> Result<Option<Vec<String>>, Error> {
    let Some((qid, argv)) = pace::remaining_work(dir) else {
        return Ok(None);
    };
    if qid != card {
        return Err(Error::Fail(format!("split brain: pace {card} queue {qid}")));
    }
    if crate::worker_phase::tasks_mode(dir) {
        // CARD-0221: the line is a TASK, not an argv. Journal the phase;
        // the execution organ (scout/build/scan/repair) lands on top.
        crate::worker_phase::journal(dir, card, "task");
        println!("{card} TASK accepted (workflow=tasks)");
        return Ok(None);
    }
    if argv.is_empty() {
        return Ok(None);
    }
    // CARD-0235: refuse an unprovable line at ARM time — no card file
    // means done can never be earned, so re-running it is a pure burn.
    if !crate::worker_done::card_file_exists(card) {
        return Err(Error::Fail(format!(
            "queue line refused at arm: {card} has no card file — done is unprovable by construction"
        )));
    }
    Ok(Some(argv))
}

fn run_task(id: &str, dir: &std::path::Path, card: &str, argv: Vec<String>) -> Result<i32, Error> {
    if is_chair(&argv[0]) {
        return Err(Error::Fail(format!("chair argv forbidden: {}", argv[0])));
    }
    let kind = pace::arm_kind(id).map_err(map_pace)?;
    let mut bee_args = vec!["spawn".into(), "--harness".into(), kind, "--".into()];
    let argv0 = argv[0].clone();
    bee_args.extend(argv);
    // CARD-0271: AKIS advisory — the bee runs `caddis akis --card <id>`
    // and fixes Error rows (nits never gate; the spine owns hard gates).
    bee_args.push(format!("run caddis akis --card {card} and fix Error rows"));
    let exit = match bee::run(&bee_args) {
        Ok(c) => c,
        Err(e) => {
            journal_bee(dir, card, &argv0, 1);
            return Err(map_bee(e));
        }
    };
    journal_bee(dir, card, &argv0, exit);
    if exit == 0 {
        let outcome = crate::worker_done::verify_done_when(dir, card);
        crate::worker_done::withheld_gate(dir, card, &outcome);
        if matches!(outcome, crate::worker_done::DoneOutcome::Marked(_)) {
            crate::worker_reach::judge_and_tell(dir, card); // CARD-0327 LAYER 3
        }
    }
    Ok(exit)
}

fn map_scan(e: crate::worker_scan::Error) -> Error {
    match e {
        crate::worker_scan::Error::Usage(s) => Error::Usage(s),
        crate::worker_scan::Error::Fail(s) => Error::Fail(s),
    }
}

/// FR-11 bee journal: one JSONL line per bee run.
fn journal_bee(dir: &std::path::Path, card: &str, argv0: &str, exit: i32) {
    use std::fs::OpenOptions;
    use std::io::Write;
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let ts = crate::receipt::timestamp();
    let (c, a) = (esc(card), esc(argv0));
    let line = format!("{{\"card\":\"{c}\",\"argv0\":\"{a}\",\"exit\":{exit},\"ts\":\"{ts}\"}}");
    // swallow: best-effort-telemetry
    if let Ok(mut f) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("bee.log"))
    {
        let _ = writeln!(f, "{line}"); // swallow: best-effort-telemetry
    }
}

fn map_board(e: crate::worker_board::Error) -> Error {
    match e {
        crate::worker_board::Error::Usage(s) => Error::Usage(s),
        crate::worker_board::Error::Fail(s) => Error::Fail(s),
    }
}

fn is_chair(argv0: &str) -> bool {
    let name = Path::new(argv0)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(argv0);
    matches!(
        name.to_ascii_lowercase().as_str(),
        "claude" | "omp" | "qpi" | "herdr" | "grok"
    )
}

fn map_pace(e: pace::Error) -> Error {
    match e {
        pace::Error::Usage(s) => Error::Usage(s),
        pace::Error::Fail(s) => Error::Fail(s),
    }
}

fn map_bee(e: bee::Error) -> Error {
    match e {
        bee::Error::Usage(s) | bee::Error::Fail(s) => Error::Fail(s),
    }
}
