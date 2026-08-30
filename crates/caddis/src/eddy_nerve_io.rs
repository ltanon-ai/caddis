//! eddy_nerve_io.rs — the nerve's tick/arm I/O: flat-JSON readers,
//! stdin-tick parsing, arm-file persistence. Split out of
//! eddy_nerve.rs under the 280-line law (CARD-0233). Same organ, same
//! fail-closed law.

use caddis_organs::eddy::{self, StatusClass, Tick};
use caddis_organs::eddy_arm::{ArmSpec, Armed, Bound, LoopClass};

use crate::eddy_nerve::{closed, Error};

/// Minimal flat-JSON readers (same tolerance as the organs' util).
pub(crate) fn json_str(line: &str, key: &str) -> Option<String> {
    let pat = format!("\"{key}\":");
    let start = line.find(&pat)? + pat.len();
    let rest = &line[start..];
    let rest = rest.trim_start();
    if !rest.starts_with('"') {
        return None;
    }
    let body = &rest[1..];
    let end = body.find('"')?;
    Some(body[..end].replace("\\n", "\n").replace("\\\"", "\""))
}

pub(crate) fn json_num(line: &str, key: &str) -> Option<u64> {
    let pat = format!("\"{key}\":");
    let start = line.find(&pat)? + pat.len();
    let digits: String = line[start..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

/// Parse the stdin tick. Unknown status class REFUSES (fail-closed):
/// an unjudgeable tick may never be recorded as Ok.
pub(crate) fn parse_tick(run_id: &str, input: &str, seq: u64) -> Result<Tick, Error> {
    let payload = json_str(input, "payload").ok_or_else(|| closed("tick has no payload"))?;
    let class =
        json_str(input, "status_class").ok_or_else(|| closed("tick has no status_class"))?;
    let status = StatusClass::parse_wire(&class)
        .ok_or_else(|| closed(&format!("unknown status_class {class:?}")))?;
    let outcome = json_str(input, "outcome").ok_or_else(|| closed("tick has no outcome"))?;
    Ok(Tick {
        run_id: run_id.into(),
        seq,
        payload_hash: eddy::stable_hash(&payload),
        status_class: status,
        outcome_hash: eddy::stable_hash(&outcome),
        cache_read: json_num(input, "cache_read").unwrap_or(0),
        cache_write: json_num(input, "cache_write").unwrap_or(0),
        latency_ms: json_num(input, "latency_ms").unwrap_or(0),
        ts_ms: caddis_organs::util::unix_ms(),
        resume_after: json_num(input, "resume_after"),
        artifact_hash: json_hex(input, "artifact_hash"),
        page: json_num(input, "page").unwrap_or(0),
    })
}

/// Hex string field (`"a1b2…"`), 0 when absent/null.
pub(crate) fn json_hex(line: &str, key: &str) -> u64 {
    json_str(line, key)
        .and_then(|h| u64::from_str_radix(&h, 16).ok())
        .unwrap_or(0)
}

pub(crate) fn next_seq(ticks_path: &std::path::Path) -> u64 {
    eddy::read_ticks(ticks_path).len() as u64 + 1
}

/// The arm file: `until=N` | `for_ms=T` plus `class=…`, written on the
/// first tick, read on every later one — the contract of the whole run.
pub(crate) fn write_arm(arm_path: &std::path::Path, armed: &Armed) -> Result<(), Error> {
    if let Some(dir) = arm_path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| closed(&format!("arm dir: {e}")))?;
    }
    std::fs::write(arm_path, arm_text(armed)).map_err(|e| closed(&format!("arm write: {e}")))
}

pub(crate) fn read_arm(arm_path: &std::path::Path) -> Result<Armed, Error> {
    let text = std::fs::read_to_string(arm_path).map_err(|e| closed(&format!("arm: {e}")))?;
    let bound = parse_arm_bound(&text).ok_or_else(|| closed("arm file unreadable"))?;
    let class = if text.contains("class=until-change") {
        Some(LoopClass::UntilChange)
    } else {
        Some(LoopClass::UntilExternal)
    };
    let lease_ms = text
        .lines()
        .find(|l| l.starts_with("wait_ms="))
        .and_then(|l| l["wait_ms=".len()..].parse().ok());
    Armed::arm(
        "persisted",
        ArmSpec {
            bound: Some(bound),
            class,
            lease_ms,
        },
    )
    .map_err(usage_of_arm)
}

pub(crate) fn usage_of_arm(e: caddis_organs::eddy_arm::ArmError) -> Error {
    match e {
        caddis_organs::eddy_arm::ArmError::Unbounded { reason } => Error::Closed(reason),
    }
}

fn arm_text(a: &Armed) -> String {
    let bound_line = match a.bound() {
        Bound::Iterations(n) => format!("until={n}\n"),
        Bound::Millis(t) => format!("for_ms={t}\n"),
    };
    let class_line = match a.class() {
        LoopClass::UntilChange => "class=until-change\n".to_string(),
        LoopClass::UntilExternal => "class=until-external\n".to_string(),
    };
    let lease_line = a
        .lease_ms()
        .map(|ms| format!("wait_ms={ms}\n"))
        .unwrap_or_default();
    format!("{bound_line}{class_line}{lease_line}")
}

fn parse_arm_bound(text: &str) -> Option<Bound> {
    if let Some(n) = text
        .lines()
        .find(|l| l.starts_with("until="))
        .and_then(|l| l["until=".len()..].parse().ok())
    {
        return Some(Bound::Iterations(n));
    }
    if let Some(t) = text
        .lines()
        .find(|l| l.starts_with("for_ms="))
        .and_then(|l| l["for_ms=".len()..].parse().ok())
    {
        return Some(Bound::Millis(t));
    }
    None
}

/// Blocker filing on the organ's behalf (host-owned blockers.jsonl in
/// the eddy dir).
pub(crate) fn file_blocker(
    run_id: &str,
    ticks: &[Tick],
    home: &std::path::Path,
) -> Result<(), Error> {
    let path = home.join(".caddis").join("eddy").join("blockers.jsonl");
    eddy::enforce(run_id, ticks, &path)
        .map(|_| ())
        .map_err(|e| closed(&format!("blocker: {e}")))
}

/// ONE `loop.epoch` envelope row per page rollover (CARD-0242): the
/// rollover is an event the operator can replay — never per-tick.
pub(crate) fn write_epoch_row(
    run_id: &str,
    last: &Tick,
    from_page: u64,
    home: &std::path::Path,
) -> Result<(), Error> {
    let body = format!(
        "run={}|from_page={}|to_page={}|payload={:016x}",
        run_id, from_page, last.page, last.payload_hash
    );
    let id = format!("eddy{:012x}", eddy::stable_hash(&body));
    let idem = format!("{:016x}", eddy::stable_hash(&body));
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        .to_string();
    let env =
        caddis_core::envelope::validate(1, &id, &idem, "loop.epoch", "omp", "eddy", &body, &ts)
            .map_err(|e| closed(&format!("envelope refused: {} {}", e.code, e.why)))?;
    let path = home.join(".caddis").join("eddy-ledger.jsonl");
    caddis_core::ledger::Ledger::open(&path)
        .and_then(|mut led| led.append(&env))
        .map(|_| ())
        .map_err(|e| closed(&format!("epoch row: {e}")))
}
/// ONE caddis-core envelope row per RUN: the arm identity and the
/// final verdict — never per tick (589 ticks must not load the TCB).
pub(crate) fn write_run_row(
    run_id: &str,
    last: &Tick,
    reason: &str,
    home: &std::path::Path,
) -> Result<(), Error> {
    let body = format!(
        "run={}|payload={:016x}|seq={}|{}",
        run_id, last.payload_hash, last.seq, reason
    );
    let id = format!("eddy{:012x}", eddy::stable_hash(run_id));
    let idem = format!("{:016x}", eddy::stable_hash(&body));
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        .to_string();
    let env = caddis_core::envelope::validate(1, &id, &idem, "loop.run", "omp", "eddy", &body, &ts)
        .map_err(|e| closed(&format!("envelope refused: {} {}", e.code, e.why)))?;
    let path = home.join(".caddis").join("eddy-ledger.jsonl");
    caddis_core::ledger::Ledger::open(&path)
        .and_then(|mut led| led.append(&env))
        .map(|_| ())
        .map_err(|e| closed(&format!("run row: {e}")))
}

/// CARD-0241: health never gates the verdict — it reports and flags.
pub(crate) fn health_report(run_id: &str, ticks_path: &std::path::Path, home: &std::path::Path) {
    let ticks = eddy::read_ticks(ticks_path);
    let Some(report) = eddy::cache_health(&ticks) else {
        return;
    };
    println!(
        "{{\"health\":\"cache-cold-after-warm\",\"last_warm_seq\":{}}}",
        report.last_warm_seq
    );
    // swallow: best-effort-telemetry — health flags, never gates
    let _ = eddy::enforce_health(
        run_id,
        &report,
        &home.join(".caddis").join("eddy").join("blockers.jsonl"),
    );
}
