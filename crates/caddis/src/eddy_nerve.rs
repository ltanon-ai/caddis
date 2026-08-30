//! eddy_nerve.rs — CARD-0233. `caddis eddy tick`: the JSON nerve
//! contract for the governed loop, FAIL-CLOSED.
//!
//! Seam ruling: (a) nerve for the interactive TUI + (d) headless
//! runner, jointly, sharing ONE pure verdict (caddis-organs). JS holds
//! NO threshold: the host reports a tick, the nerve judges. (c)
//! `before_tool_call` was REJECTED as a halt seam — it cannot see
//! provider errors or "no model selected", so it would not have
//! stopped the burn.
//!
//! FAIL-CLOSED INVERTS the warden's doctrine: the warden allows loudly
//! when its binary is unspawnable, because one unjudged tool call is
//! one bounded action; the nerve REFUSES, because an unjudged loop is
//! an unbounded 800ms re-fire. Malformed tick, unknown status class,
//! missing bound, unwritable state — exit 2 with a DISABLE directive,
//! never a silent claim of enforcement.
//!
//! Files under $HOME/.caddis/eddy/:
//!   <run>.jsonl    — host-owned ticks (one line per iteration)
//!   <run>.arm      — the arm-time contract (bound + class), first tick
//!   blockers.jsonl — filed by the organ on halt
//! and $HOME/.caddis/eddy-ledger.jsonl — ONE caddis-core envelope row
//!
//! omp-loop-guard.py's JS-side 3-strike stays as the LAST-RESORT
//! fallback until the patched bundle calls this nerve; the patch dies
//! on `omp upgrade`, this nerve does not.

use std::io::Read;
use std::path::PathBuf;

use caddis_organs::eddy::{self, Tick, Verdict};
use caddis_organs::eddy_arm::{ArmSpec, Armed, Bound, LoopClass};

use crate::eddy_nerve_io::{
    file_blocker, health_report, next_seq, parse_tick, read_arm, usage_of_arm, write_arm,
    write_epoch_row, write_run_row,
};

pub enum Error {
    /// Exit 2 + the DISABLE directive (fail-closed).
    Closed(String),
    /// Plain usage error.
    Usage(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Closed(s) | Error::Usage(s) => write!(f, "{s}"),
        }
    }
}

const HELP: &str = "\
USAGE: caddis eddy tick --run <id> (--until N | --for-ms T) [--class until-change|until-external]
  one tick as JSON on stdin:
  {\"payload\":\"...\",\"status_class\":\"ok|fail|fatal.quota|fatal.auth|fatal.terminated\",
   \"outcome\":\"...\",\"cache_read\":N,\"cache_write\":N,\"latency_ms\":N,\"resume_after\":N|null}
  exit 0 continue|stagnant · exit 3 halt · exit 2 FAIL-CLOSED (disable governed loop mode)
";

pub fn run(args: &[String]) -> Result<i32, Error> {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print!("{HELP}");
        return Ok(0);
    }
    let args = match args.first().map(String::as_str) {
        Some("tick") => &args[1..],
        _ => return Err(Error::Usage("eddy requires the tick subcommand".into())),
    };
    let p = parse_args(args)?;
    let (run_id, until, for_ms, class) = (p.run_id, p.until, p.for_ms, p.class);
    tick(&run_id, until, for_ms, class)
}

struct Parsed {
    run_id: String,
    until: Option<String>,
    for_ms: Option<String>,
    class: Option<String>,
}

fn parse_args(args: &[String]) -> Result<Parsed, Error> {
    let mut p = Parsed {
        run_id: String::new(),
        until: None,
        for_ms: None,
        class: None,
    };
    let mut i = 0;
    while i < args.len() {
        let name = args[i].as_str();
        let value = args
            .get(i + 1)
            .cloned()
            .ok_or_else(|| Error::Usage(format!("missing {name} value")))?;
        set_flag(&mut p, name, value)?;
        i += 2;
    }
    validate_run_id(&p.run_id)?;
    Ok(p)
}

fn set_flag(p: &mut Parsed, name: &str, value: String) -> Result<(), Error> {
    match name {
        "--run" => p.run_id = value,
        "--until" => p.until = Some(value),
        "--for-ms" => p.for_ms = Some(value),
        "--class" => p.class = Some(value),
        other => return Err(Error::Usage(format!("unknown argument {other}"))),
    }
    Ok(())
}

fn validate_run_id(run_id: &str) -> Result<(), Error> {
    if run_id.is_empty() {
        return Err(Error::Usage("eddy tick requires --run".into()));
    }
    if run_id.contains('/') || run_id.contains('\\') {
        return Err(Error::Usage("--run must be a plain name".into()));
    }
    Ok(())
}

fn tick(
    run_id: &str,
    until: Option<String>,
    for_ms: Option<String>,
    class: Option<String>,
) -> Result<i32, Error> {
    let home = home_dir()?;
    let dir = home.join(".caddis").join("eddy");
    let ticks_path = dir.join(format!("{run_id}.jsonl"));
    let arm_path = dir.join(format!("{run_id}.arm"));
    let arm = resolve_arm(&arm_path, until, for_ms, class)?;
    let input = read_stdin()?;
    let previous = next_seq(&ticks_path) - 1;
    let t = parse_tick(run_id, &input, previous + 1)?;
    eddy::record_tick(&ticks_path, &t).map_err(|e| closed(&format!("record: {e}")))?;
    // CARD-0242: page > prior max writes ONE loop.epoch row — a
    // replayable rollover event, never per-tick telemetry.
    let prior_max_page = eddy::read_ticks(&ticks_path)
        .iter()
        .take_while(|k| k.seq < t.seq)
        .map(|k| k.page)
        .max()
        .unwrap_or(0);
    if t.page > prior_max_page {
        write_epoch_row(run_id, &t, prior_max_page, &home)?;
    }
    health_report(run_id, &ticks_path, &home);
    judge_and_report(run_id, &arm, &ticks_path, &t, &home)
}

/// The run's contract: read the persisted arm, or write it on the
/// first tick (unbounded is refused HERE too — the organ law, not
/// just the nerve's opinion).
fn resolve_arm(
    arm_path: &std::path::Path,
    until: Option<String>,
    for_ms: Option<String>,
    class: Option<String>,
) -> Result<Armed, Error> {
    if arm_path.is_file() {
        return read_arm(arm_path);
    }
    let spec = ArmSpec {
        bound: bound_of(until, for_ms)?,
        class: class_of(class.as_deref())?,
        lease_ms: None, // CARD-0240's --wait-ms lands with the flag
    };
    let armed = Armed::arm("armed-at-first-tick", spec).map_err(usage_of_arm)?;
    write_arm(arm_path, &armed)?;
    Ok(armed)
}

/// The verdict + the report + the side effects (blocker, RUN row) —
/// and the exit code: 0 continue/stagnant, 3 halt.
fn judge_and_report(
    run_id: &str,
    arm: &Armed,
    ticks_path: &std::path::Path,
    last: &Tick,
    home: &std::path::Path,
) -> Result<i32, Error> {
    let ticks = eddy::read_ticks(ticks_path);
    match arm.judge(&ticks) {
        Verdict::Continue => {
            println!("{{\"verdict\":\"continue\",\"seq\":{}}}", last.seq);
            Ok(0)
        }
        Verdict::Stagnant => {
            println!("{{\"verdict\":\"stagnant\",\"seq\":{}}}", last.seq);
            Ok(0)
        }
        Verdict::UnprovableDone { streak } => {
            // CARD-0237: a stop verdict like Halt — the run cannot
            // prove its own done. Same blocker + RUN row + exit 3.
            let text = format!("unprovable done after {streak} dispatches: halting governed loop");
            file_blocker(run_id, &ticks, home)?;
            write_run_row(run_id, last, &text, home)?;
            println!(
                "{{\"verdict\":\"halt\",\"seq\":{},\"reason\":\"{}\"}}",
                last.seq,
                text.replace('\\', "\\\\").replace('"', "\\\"")
            );
            Ok(3)
        }
        Verdict::Halt(reason) => {
            let text = eddy::halt_reason_text(&reason);
            file_blocker(run_id, &ticks, home)?;
            write_run_row(run_id, last, &text, home)?;
            println!(
                "{{\"verdict\":\"halt\",\"seq\":{},\"reason\":\"{}\"}}",
                last.seq,
                text.replace('\\', "\\\\").replace('"', "\\\"")
            );
            Ok(3)
        }
    }
}

fn class_of(class: Option<&str>) -> Result<Option<LoopClass>, Error> {
    match class {
        None => Ok(None),
        Some("until-change") => Ok(Some(LoopClass::UntilChange)),
        Some("until-external") => Ok(Some(LoopClass::UntilExternal)),
        Some(other) => Err(Error::Usage(format!(
            "unknown class {other}: until-change|until-external"
        ))),
    }
}

fn bound_of(until: Option<String>, for_ms: Option<String>) -> Result<Option<Bound>, Error> {
    match (until, for_ms) {
        (None, None) => Ok(None),
        (Some(n), None) => n
            .parse()
            .map(|v| Some(Bound::Iterations(v)))
            .map_err(|_| Error::Usage("--until must be a number".into())),
        (None, Some(t)) => t
            .parse()
            .map(|v| Some(Bound::Millis(v)))
            .map_err(|_| Error::Usage("--for-ms must be a number".into())),
        (Some(_), Some(_)) => Err(Error::Usage("--until and --for-ms are exclusive".into())),
    }
}

fn read_stdin() -> Result<String, Error> {
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .map_err(|e| closed(&format!("stdin: {e}")))?;
    Ok(buf)
}

fn home_dir() -> Result<PathBuf, Error> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or_else(|| closed("HOME is unset"))
}

pub(crate) fn closed(why: &str) -> Error {
    // THE directive: fail-closed means the host disables governed loop
    // mode, it does NOT carry on unjudged.
    Error::Closed(format!(
        "{why}; DISABLE governed loop mode (eddy fail-closed: an unjudged loop is an unbounded re-fire)"
    ))
}
