//! pace.rs — CARD-0214 + CARD-0215. Conscience pace in the join crate.
//!
//! Not TCB, not a skill, not watcher prose. Empty queue = no force.
//! WORK when a named card remains and the chair is not LiveAgent.

use std::fs;
use std::path::Path;

use crate::drain::{self, DrainResult};
use crate::hmac;
use crate::lineage;
use crate::receipt;

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

pub fn run(args: &[String]) -> Result<(), Error> {
    let (id, rest) = lineage::take(args).map_err(Error::Usage)?;
    let pace_flag = take_pace(&rest)?;
    check(&id, pace_flag.as_deref())
}

fn take_pace(rest: &[String]) -> Result<Option<String>, Error> {
    let mut pace = None;
    let mut i = 0;
    while i < rest.len() {
        if rest[i] != "--pace" {
            return Err(Error::Usage(format!("unknown argument {}", rest[i])));
        }
        i += 1;
        let v = rest
            .get(i)
            .ok_or_else(|| Error::Usage("missing --pace value".into()))?;
        if v != "run" && v != "stop" {
            return Err(Error::Usage("pace must be run or stop".into()));
        }
        if pace.is_some() {
            return Err(Error::Usage("duplicate --pace".into()));
        }
        pace = Some(v.clone());
        i += 1;
    }
    Ok(pace)
}

fn check(id: &str, pace_flag: Option<&str>) -> Result<(), Error> {
    let verdict = beat(id, pace_flag)?;
    println!("{verdict}");
    Ok(())
}

pub(crate) fn beat(id: &str, pace_flag: Option<&str>) -> Result<String, Error> {
    let (dir, body, key) = load_arm(id)?;
    match_lineage(&body, id)?;
    if let Some(p) = pace_flag {
        restamp_pace(&dir, &body, &key, id, p)?;
    }
    let body = if pace_flag.is_some() {
        load_arm(id)?.1
    } else {
        body
    };
    let (pace, kind, pane) = arm_fields_of(&body)?;
    let card = remaining_card(&dir);
    let idle = agent_idle(&kind, pane.as_deref());
    let verdict = decide(&pace, card.as_deref(), idle);
    write_line(&dir, &key, id, &verdict)?;
    Ok(verdict)
}

/// dir + receipt body + key for a lineage (the beat preamble).
fn load_arm(id: &str) -> Result<(std::path::PathBuf, Vec<u8>, Vec<u8>), Error> {
    let dir = lineage::dir(id).map_err(Error::Fail)?;
    let (body, key) = read_arm(&dir)?;
    Ok((dir, body, key))
}

fn arm_fields_of(body: &[u8]) -> Result<(String, String, Option<String>), Error> {
    let pace = receipt::extract_field(body, "pace").unwrap_or_else(|| "run".into());
    let kind = field(body, "kind")?;
    let pane = receipt::extract_field(body, "pane");
    Ok((pace, kind, pane))
}

fn agent_idle(kind: &str, pane: Option<&str>) -> bool {
    !matches!(drain::drain(kind, pane), DrainResult::LiveAgent(_))
}

fn decide(pace: &str, card: Option<&str>, idle: bool) -> String {
    if pace == "stop" {
        return "PACE STOP".into();
    }
    match card {
        None => "PACE IDLE-OK".into(),
        Some(c) if idle => format!("PACE WORK {c}"),
        Some(_) => "PACE BUSY".into(),
    }
}

pub(crate) fn remaining_work(dir: &Path) -> Option<(String, Vec<String>)> {
    let text = fs::read_to_string(dir.join("queue")).ok()?;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty()
            || line.starts_with('#')
            || line.starts_with("done ")
            || line.starts_with("withheld ")
        {
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(id) = parts.next() else {
            continue;
        };
        if !id.starts_with("CARD-") {
            continue;
        }
        let argv: Vec<String> = parts.map(str::to_string).collect();
        return Some((id.to_string(), argv));
    }
    None
}

pub(crate) fn remaining_card(dir: &Path) -> Option<String> {
    remaining_work(dir).map(|(id, _)| id)
}

pub(crate) fn arm_kind(id: &str) -> Result<String, Error> {
    let dir = lineage::dir(id).map_err(Error::Fail)?;
    let (body, _) = read_arm(&dir)?;
    field(&body, "kind")
}

fn restamp_pace(dir: &Path, body: &[u8], key: &[u8], id: &str, pace: &str) -> Result<(), Error> {
    let kind = field(body, "kind")?;
    let model = field(body, "model")?;
    let pane = receipt::extract_field(body, "pane").unwrap_or_default();
    lineage::write_paced(dir, "arm.receipt", key, &kind, &model, &pane, id, pace)
        .map_err(Error::Fail)?;
    Ok(())
}

fn write_line(dir: &Path, key: &[u8], id: &str, sentence: &str) -> Result<(), Error> {
    let ts = receipt::timestamp();
    let body = format!("sentence={sentence}\nlineage={id}\nts={ts}\n");
    let mac = hmac::hmac_sha256(key, body.as_bytes());
    let text = format!("{body}---\n{}\n", receipt::hex_string(&mac));
    fs::write(dir.join("pace.line"), text.as_bytes())
        .map_err(|e| Error::Fail(format!("write pace.line: {e}")))?;
    Ok(())
}

/// Fold-class nerve may print this frozen sentence only.
pub(crate) fn print_frozen(id: &str) {
    let Ok(dir) = lineage::dir(id) else {
        return;
    };
    let Ok(key) = receipt::load_key(&dir) else {
        return;
    };
    let Ok(bytes) = fs::read(dir.join("pace.line")) else {
        return;
    };
    let Some((body, mac)) = receipt::split_receipt(&bytes) else {
        return;
    };
    if hmac::hmac_sha256(&key, body) != mac {
        return;
    }
    if let Some(s) = receipt::extract_field(body, "sentence") {
        println!("{s}");
    }
}

/// Refresh pace.line (no ARM → silent) then feed the frozen sentence.
pub(crate) fn feed(id: &str) {
    let _ = beat(id, None); // swallow: best-effort-telemetry
    print_frozen(id);
}

fn read_arm(dir: &Path) -> Result<(Vec<u8>, Vec<u8>), Error> {
    let bytes =
        fs::read(dir.join("arm.receipt")).map_err(|e| Error::Fail(format!("no arm: {e}")))?;
    let key = receipt::load_key(dir).map_err(Error::Fail)?;
    let (body, mac) = receipt::split_receipt(&bytes)
        .ok_or_else(|| Error::Fail("arm receipt is malformed".into()))?;
    if hmac::hmac_sha256(&key, body) != mac {
        return Err(Error::Fail("arm receipt HMAC mismatch".into()));
    }
    Ok((body.to_vec(), key))
}

fn match_lineage(body: &[u8], id: &str) -> Result<(), Error> {
    let got = field(body, "lineage")?;
    if got != id {
        return Err(Error::Fail(format!("arm lineage {got} != --lineage {id}")));
    }
    Ok(())
}

fn field(body: &[u8], name: &str) -> Result<String, Error> {
    receipt::extract_field(body, name).ok_or_else(|| Error::Fail(format!("arm has no {name}")))
}
