//! sessions.rs — P3 slice 1: SESSION CARDS (`class: session`) — the R4
//! full-session usage record, ONE mechanism feeding the model-visibility
//! and first-pass-per-euro views (UNIFY law: these rows record usage
//! FACTS; the consuming views compute cost OPINIONS — numeric cost math
//! deliberately does not live in this stream).
//!
//! Laws transcribed (plan P3 / R4, brief lesson, registry grammar law):
//! - **open/close per convening** (R4): the executor appends `open`
//!   before the first dispatch leg, ONE `usage` row per answered seat as
//!   its wave completes (crash-honest — a run that dies mid-flight leaves
//!   open + partial usage and NO close: an auditable hole, never a
//!   silent one), and `close` after the verdict carrying the digest
//!   link.
//! - **Deterministic bytes**: one flat JSON object per line, exact field
//!   set per `evt`, fixed order, LF-terminated, no timestamps (MV11 —
//!   the warden ledger owns times), no secrets, no nested values.
//! - **Provenance law** (brief lesson): the usage `model` is the
//!   TRANSPORT-served model, never the seat's registered self-report.
//!   `cost_class` travels with the seat (a class survives model drift);
//!   the numeric per-token rates stay in the registry where they were
//!   measured — this stream carries no derived money.
//! - Wire words for `lane_type` / `cost_class` are the registry's ONE
//!   vocabulary ([`crate::registry::lane_type_word`] and friends) — a
//!   second copy anywhere is banned.

use std::fmt;
use std::path::Path;

use crate::json::{self, Value};
use crate::registry::{cost_class_word, lane_type_word, parse_cost_class, parse_lane_type};

/// Wire word for a session-card row.
pub const CLASS_SESSION: &str = "session";

/// EVT word for the convening-open row.
pub const EVT_OPEN: &str = "open";
/// EVT word for a per-seat usage row.
pub const EVT_USAGE: &str = "usage";
/// EVT word for the convening-close row.
pub const EVT_CLOSE: &str = "close";

/// The convening opened: who convened, under which card and pin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionOpen {
    pub conv: String,
    /// `council` (the quorum card carries its own word when it executes).
    pub kind: String,
    pub pin: String,
    pub stakes: String,
    pub rerun_of: String,
    pub actor: String,
    pub warden_card: String,
}

/// One answered seat: usage facts as the TRANSPORT recorded them.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionUsage {
    pub conv: String,
    pub lane: String,
    pub lane_type: crate::LaneType,
    pub provider: String,
    /// TRANSPORT-served model — provenance law, never the seat's
    /// self-report.
    pub model: String,
    pub cost_class: crate::CostClass,
    pub tokens_in: u64,
    pub tokens_out: u64,
}

/// The convening closed with a verdict: the digest links this row to the
/// ledger row it summarizes (tamper-evident pairing).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionClose {
    pub conv: String,
    pub verdict_digest: String,
    pub ship: u64,
    pub ship_with_changes: u64,
    pub do_not_ship: u64,
}

/// One session-stream row.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionRow {
    Open(SessionOpen),
    Usage(SessionUsage),
    Close(SessionClose),
}

/// The exact field sets (flat grammar, registry law — same count, same
/// names, no duplicates, no unknowns).
const OPEN_FIELDS: &[&str] = &[
    "class",
    "evt",
    "conv",
    "kind",
    "pin",
    "stakes",
    "rerun_of",
    "actor",
    "warden_card",
];
const USAGE_FIELDS: &[&str] = &[
    "class",
    "evt",
    "conv",
    "lane",
    "lane_type",
    "provider",
    "model",
    "cost_class",
    "tokens_in",
    "tokens_out",
];
const CLOSE_FIELDS: &[&str] = &[
    "class",
    "evt",
    "conv",
    "verdict_digest",
    "ship",
    "ship_with_changes",
    "do_not_ship",
];

/// Stream parse refusals. Malformed rows carry the 1-based line number —
/// the stream never half-parses.
#[derive(Debug, PartialEq)]
pub enum SessionRowErr {
    Malformed { line: usize, msg: String },
}

impl fmt::Display for SessionRowErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SessionRowErr::Malformed { line, msg } => {
                write!(f, "session row {line}: {msg}")
            }
        }
    }
}

impl std::error::Error for SessionRowErr {}

fn json_str(s: &str, out: &mut String) {
    out.push_str(&json::to_string(&Value::Str(s.to_string())));
}

/// Encode one row as a flat JSON line (no trailing LF — the caller's
/// append adds it; byte-deterministic for a given row).
pub fn encode_row(row: &SessionRow) -> String {
    let mut o = String::with_capacity(192);
    o.push_str("{\"class\":\"session\",\"evt\":");
    match row {
        SessionRow::Open(r) => {
            o.push_str("\"open\",\"conv\":");
            json_str(&r.conv, &mut o);
            o.push_str(",\"kind\":");
            json_str(&r.kind, &mut o);
            o.push_str(",\"pin\":");
            json_str(&r.pin, &mut o);
            o.push_str(",\"stakes\":");
            json_str(&r.stakes, &mut o);
            o.push_str(",\"rerun_of\":");
            json_str(&r.rerun_of, &mut o);
            o.push_str(",\"actor\":");
            json_str(&r.actor, &mut o);
            o.push_str(",\"warden_card\":");
            json_str(&r.warden_card, &mut o);
        }
        SessionRow::Usage(r) => {
            o.push_str("\"usage\",\"conv\":");
            json_str(&r.conv, &mut o);
            o.push_str(",\"lane\":");
            json_str(&r.lane, &mut o);
            o.push_str(",\"lane_type\":\"");
            o.push_str(lane_type_word(r.lane_type));
            o.push_str("\",\"provider\":");
            json_str(&r.provider, &mut o);
            o.push_str(",\"model\":");
            json_str(&r.model, &mut o);
            o.push_str(",\"cost_class\":\"");
            o.push_str(cost_class_word(r.cost_class));
            o.push_str("\",\"tokens_in\":");
            o.push_str(&r.tokens_in.to_string());
            o.push_str(",\"tokens_out\":");
            o.push_str(&r.tokens_out.to_string());
        }
        SessionRow::Close(r) => {
            o.push_str("\"close\",\"conv\":");
            json_str(&r.conv, &mut o);
            o.push_str(",\"verdict_digest\":");
            json_str(&r.verdict_digest, &mut o);
            o.push_str(",\"ship\":");
            o.push_str(&r.ship.to_string());
            o.push_str(",\"ship_with_changes\":");
            o.push_str(&r.ship_with_changes.to_string());
            o.push_str(",\"do_not_ship\":");
            o.push_str(&r.do_not_ship.to_string());
        }
    }
    o.push('}');
    o
}

/// Parse stream text into rows, IN ORDER. Empty lines are skipped; every
/// other line must parse with its EXACT field set (fail-closed, 1-based
/// line numbers).
pub fn parse_rows(text: &str) -> Result<Vec<SessionRow>, SessionRowErr> {
    let mut rows = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        rows.push(parse_line(line, i + 1)?);
    }
    Ok(rows)
}

fn parse_line(t: &str, line_no: usize) -> Result<SessionRow, SessionRowErr> {
    let v = json::parse(t).map_err(|e| bad(line_no, format!("not JSON: {}", e.msg)))?;
    let obj = v
        .as_obj()
        .ok_or_else(|| bad(line_no, "not a flat object"))?;
    let class = str_field(obj, "class", line_no)?;
    if class != CLASS_SESSION {
        return Err(bad(
            line_no,
            format!("class is '{class}' — this stream carries session rows"),
        ));
    }
    let evt = str_field(obj, "evt", line_no)?;
    match evt.as_str() {
        EVT_OPEN => {
            exact_fields(obj, OPEN_FIELDS, line_no)?;
            Ok(SessionRow::Open(SessionOpen {
                conv: str_field(obj, "conv", line_no)?,
                kind: str_field(obj, "kind", line_no)?,
                pin: str_field(obj, "pin", line_no)?,
                stakes: str_field(obj, "stakes", line_no)?,
                rerun_of: str_field(obj, "rerun_of", line_no)?,
                actor: str_field(obj, "actor", line_no)?,
                warden_card: str_field(obj, "warden_card", line_no)?,
            }))
        }
        EVT_USAGE => {
            exact_fields(obj, USAGE_FIELDS, line_no)?;
            let lane_type = parse_lane_type(&str_field(obj, "lane_type", line_no)?)
                .ok_or_else(|| bad(line_no, "lane_type is not a lane word"))?;
            let cost_class = parse_cost_class(&str_field(obj, "cost_class", line_no)?)
                .ok_or_else(|| bad(line_no, "cost_class is not a class word"))?;
            Ok(SessionRow::Usage(SessionUsage {
                conv: str_field(obj, "conv", line_no)?,
                lane: str_field(obj, "lane", line_no)?,
                lane_type,
                provider: str_field(obj, "provider", line_no)?,
                model: str_field(obj, "model", line_no)?,
                cost_class,
                tokens_in: u64_field(obj, "tokens_in", line_no)?,
                tokens_out: u64_field(obj, "tokens_out", line_no)?,
            }))
        }
        EVT_CLOSE => {
            exact_fields(obj, CLOSE_FIELDS, line_no)?;
            Ok(SessionRow::Close(SessionClose {
                conv: str_field(obj, "conv", line_no)?,
                verdict_digest: str_field(obj, "verdict_digest", line_no)?,
                ship: u64_field(obj, "ship", line_no)?,
                ship_with_changes: u64_field(obj, "ship_with_changes", line_no)?,
                do_not_ship: u64_field(obj, "do_not_ship", line_no)?,
            }))
        }
        other => Err(bad(
            line_no,
            format!("evt is '{other}' — open | usage | close only"),
        )),
    }
}

fn bad(line: usize, msg: impl Into<String>) -> SessionRowErr {
    SessionRowErr::Malformed {
        line,
        msg: msg.into(),
    }
}

fn field<'a>(
    obj: &'a [(String, Value)],
    key: &str,
    line_no: usize,
) -> Result<&'a Value, SessionRowErr> {
    obj.iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v)
        .ok_or_else(|| bad(line_no, format!("missing field '{key}'")))
}

fn str_field(obj: &[(String, Value)], key: &str, line_no: usize) -> Result<String, SessionRowErr> {
    match field(obj, key, line_no)? {
        Value::Str(s) => Ok(s.clone()),
        _ => Err(bad(line_no, format!("field '{key}' is not a string"))),
    }
}

fn u64_field(obj: &[(String, Value)], key: &str, line_no: usize) -> Result<u64, SessionRowErr> {
    match field(obj, key, line_no)? {
        Value::Num(n) if *n >= 0.0 && n.fract() == 0.0 => Ok(*n as u64),
        _ => Err(bad(line_no, format!("field '{key}' is not a count"))),
    }
}

fn exact_fields(
    obj: &[(String, Value)],
    want: &[&str],
    line_no: usize,
) -> Result<(), SessionRowErr> {
    for (k, _) in obj {
        if !want.contains(&k.as_str()) {
            return Err(bad(line_no, format!("unknown field '{k}'")));
        }
    }
    if obj.len() != want.len() {
        return Err(bad(
            line_no,
            format!(
                "exact field law: want {} fields, have {}",
                want.len(),
                obj.len()
            ),
        ));
    }
    Ok(())
}

/// Append one row to the session stream (single `write_all`, LF-terminated
/// — the router ledger append law; appends are one syscall-sized line,
/// never a rewrite).
pub fn append_row(path: &Path, row: &SessionRow) -> Result<(), std::io::Error> {
    use std::io::Write;
    let mut line = encode_row(row);
    line.push('\n');
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)?;
    f.write_all(line.as_bytes())
}

#[cfg(test)]
#[path = "sessions_tests.rs"]
mod tests;
