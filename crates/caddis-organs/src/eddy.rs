//! eddy.rs — the LOOP ORGAN, wave 2 (CARD-0228).
//!
//! Eddy watches the governed re-fire loop: every iteration of a live
//! `/loop` lands here as ONE tick, appended to a HOST-OWNED JSONL file
//! (the blocker.rs precedent — never the caddis-core TCB ledger, which
//! would carry 589 rows from a single 2.5h burn).
//!
//! THIS CARD WRITES NO LAW. It only records. Quorum ruling: every
//! performance claim in the original eddy brief was unmeasured, so the
//! first card produces the traces the later laws are allowed to read.
//! No verdict, no blocker, no halt lives in this file until CARD-0229
//! opens it. The hash is the ESTATE fnv1a (warden-identical, see
//! util.rs) — stable across rustc builds; `DefaultHasher` is seeded
//! per-process and is forbidden here.

use std::io::{self, Write};
use std::path::Path;

use crate::util::{fnv1a, json_escape, json_str_field};
// The halt law lives in eddy_law.rs (280-line split). Re-exported HERE
// so `eddy::verdict` and friends keep one canonical path.
pub use crate::eddy_health::{cache_health, enforce_health, HealthReport};
pub use crate::eddy_law::{
    enforce, halt_reason_text, verdict, HaltReason, Verdict, STAGNANT_WINDOW,
};

/// Build-stable hash (FNV-1a 64). Same algorithm as the warden's
/// `identity::fnv1a` — one hash law across the estate.
pub fn stable_hash(s: &str) -> u64 {
    fnv1a(s)
}

/// The fatal classes (CARD-0232): fatal-until-reset, never
/// "retry three times". Classified on the TYPED class the host
/// supplies — never on error body text (the K3 seat returned a
/// byte-identical 403 twice; text is not a discriminator).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FatalClass {
    Quota,
    Auth,
    Terminated,
}

impl FatalClass {
    pub fn as_str(&self) -> &'static str {
        match self {
            FatalClass::Quota => "fatal.quota",
            FatalClass::Auth => "fatal.auth",
            FatalClass::Terminated => "fatal.terminated",
        }
    }
    fn parse(s: &str) -> Option<FatalClass> {
        match s {
            "fatal.quota" => Some(FatalClass::Quota),
            "fatal.auth" => Some(FatalClass::Auth),
            "fatal.terminated" => Some(FatalClass::Terminated),
            _ => None,
        }
    }
}

/// What one loop iteration did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusClass {
    Ok,
    Fail,
    /// Fatal-until-reset: halts at ONE observation (CARD-0232).
    Fatal(FatalClass),
    /// A dispatch whose done cannot be PROVEN (no card file, no
    /// checks, failing checks) — CARD-0237. Its streak is its OWN:
    /// a provider Fail does not feed it and vice versa.
    Unprovable,
}
impl StatusClass {
    pub fn as_str(&self) -> &'static str {
        match self {
            StatusClass::Ok => "ok",
            StatusClass::Fail => "fail",
            StatusClass::Fatal(c) => c.as_str(),
            StatusClass::Unprovable => "unprovable",
        }
    }

    /// Wire parse, fail-closed: an unknown class string is REFUSED,
    /// never silently read as Ok.
    pub fn parse_wire(s: &str) -> Option<StatusClass> {
        match s {
            "ok" => Some(StatusClass::Ok),
            "fail" => Some(StatusClass::Fail),
            "unprovable" => Some(StatusClass::Unprovable),
            other => FatalClass::parse(other).map(StatusClass::Fatal),
        }
    }
}

/// One recorded loop iteration. Field order in the JSONL line is the
/// wire contract; `seq` is host-assigned and monotone per run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tick {
    pub run_id: String,
    pub seq: u64,
    pub payload_hash: u64,
    pub status_class: StatusClass,
    pub outcome_hash: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub latency_ms: u64,
    /// Host wall-clock at the tick, unix ms. 0 = legacy line without a
    /// clock (CARD-0231 duration bounds never fire on 0).
    pub ts_ms: u64,
    /// When the provider says the fatal condition resets, unix ms
    /// (CARD-0232). None = provider supplied nothing.
    pub resume_after: Option<u64>,
    /// Estate hash of the iteration's WORK PRODUCTS (CARD-0239).
    /// 0 = the host supplied nothing; the prose basis then governs.
    pub artifact_hash: u64,
    /// Context-page epoch (CARD-0242): the host bumps it on the first
    /// tick after a compaction rollover. 0 = legacy/unknown; hashes
    /// never compare across pages.
    pub page: u64,
}

impl Tick {
    pub fn to_jsonl(&self) -> String {
        format!(
            "{{\"run_id\":\"{}\",\"seq\":{},\"payload_hash\":\"{:016x}\",\
             \"status_class\":\"{}\",\"outcome_hash\":\"{:016x}\",\
             \"cache_read\":{},\"cache_write\":{},\"latency_ms\":{},\"ts_ms\":{},\
             \"resume_after\":{},\"artifact_hash\":\"{:016x}\",\"page\":{}}}",
            json_escape(&self.run_id),
            self.seq,
            self.payload_hash,
            self.status_class.as_str(),
            self.outcome_hash,
            self.cache_read,
            self.cache_write,
            self.latency_ms,
            self.ts_ms,
            match self.resume_after {
                Some(t) => t.to_string(),
                None => "null".into(),
            },
            self.artifact_hash,
            self.page
        )
    }

    fn from_jsonl(line: &str) -> Option<Tick> {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        let (run_id, seq, payload_hash, status_class) = head_fields(line)?;
        let (outcome_hash, cache_read, cache_write, latency_ms) = tail_fields(line)?;
        // Legacy CARD-0228 lines carry no clock: ts_ms defaults to 0.
        let ts_ms = num_field(line, "ts_ms").unwrap_or(0);
        let resume_after = num_field(line, "resume_after");
        // Legacy lines carry no artifact hash: 0 = prose basis governs.
        let artifact_hash = hex_field(line, "artifact_hash").unwrap_or(0);
        // Legacy lines carry no page: 0 = one-page run.
        let page = num_field(line, "page").unwrap_or(0);
        Some(Tick {
            run_id,
            seq,
            payload_hash,
            status_class,
            outcome_hash,
            cache_read,
            cache_write,
            latency_ms,
            ts_ms,
            resume_after,
            artifact_hash,
            page,
        })
    }
}

/// run_id, seq, payload_hash, status_class — the identity of the tick.
fn head_fields(line: &str) -> Option<(String, u64, u64, StatusClass)> {
    Some((
        json_str_field(line, "run_id")?,
        num_field(line, "seq")?,
        hex_field(line, "payload_hash")?,
        StatusClass::parse_wire(&json_str_field(line, "status_class")?)?,
    ))
}

/// outcome_hash, cache_read, cache_write, latency_ms — the measurement.
fn tail_fields(line: &str) -> Option<(u64, u64, u64, u64)> {
    Some((
        hex_field(line, "outcome_hash")?,
        num_field(line, "cache_read")?,
        num_field(line, "cache_write")?,
        num_field(line, "latency_ms")?,
    ))
}

/// Append one tick to the host-owned JSONL (best-effort file create,
/// atomic-per-line append — the blocker.rs write pattern).
pub fn record_tick(path: &Path, tick: &Tick) -> io::Result<()> {
    use std::fs::OpenOptions;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut f = OpenOptions::new().create(true).append(true).open(path)?;
    f.write_all(tick.to_jsonl().as_bytes())?;
    f.write_all(b"\n")
}

/// Read every parseable tick, in file order (absent file = none).
/// An unparseable line is SKIPPED, not fatal: the recorder is telemetry
/// and may be upgraded between iterations of a long run.
pub fn read_ticks(path: &Path) -> Vec<Tick> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines().filter_map(Tick::from_jsonl).collect()
}

fn num_field(line: &str, key: &str) -> Option<u64> {
    let pat = format!("\"{key}\":");
    let start = line.find(&pat)? + pat.len();
    let digits: String = line[start..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

fn hex_field(line: &str, key: &str) -> Option<u64> {
    json_str_field(line, key).and_then(|h| u64::from_str_radix(&h, 16).ok())
}
