//! restart.rs — CARD-0305/0306/0318. The spawn transaction + talk organ:
//! enter orients at the stamped root; spawn splits AT the root and boots
//! the seat (<=80-char ASCII pointer, E2); turns are HMAC-stamped and
//! mac-VERIFIED where the gate reads them; answer|fix REQUIRE evidence
//! paths (E6); heartbeat clears armed-never-woke.

use std::fs;

use crate::lease::write_atomic;
use crate::lineage;
use crate::receipt;
use crate::which::herdr;

mod spawn;
use spawn::{extract_pane_id, seat_cmd, seat_identity};

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

/// The exact bytes a spawned pane receives — a short ASCII command whose
/// only variable is the validated lineage id (never a path; E2).
pub fn pointer(lineage: &str) -> String {
    format!("caddis restart enter --lineage {lineage}")
}

pub fn run(args: &[String]) -> Result<(), Error> {
    let sub = args
        .first()
        .ok_or_else(|| Error::Usage("restart requires a subcommand".into()))?;
    if matches!(sub.as_str(), "enter" | "spawn" | "heartbeat") {
        let (id, rest) = lineage::take(&args[1..]).map_err(Error::Usage)?;
        no_extra(&rest)?;
        return match sub.as_str() {
            "enter" => enter(&id),
            "heartbeat" => heartbeat(&id),
            _ => spawn(&id),
        };
    }
    if sub == "talk" {
        return talk(&args[1..]);
    }
    Err(Error::Usage(format!("unknown restart subcommand {sub}")))
}
fn enter(id: &str) -> Result<(), Error> {
    let dir = lineage::dir(id).map_err(Error::Fail)?;
    if !dir.join("arm.receipt").is_file() {
        return Err(Error::Fail(format!("lineage {id} has no arm receipt")));
    }
    println!("LINEAGE {id}");
    let root = fs::read_to_string(dir.join("ready.root"))
        .map(|s| s.trim_end().to_string())
        .unwrap_or_else(|_| "(no ready.root — legacy line)".into());
    println!("root: {root}");
    println!("packet: caddis lineage packet --lineage {id}");
    println!("duty 1: post your findings to the talk channel");
    println!("duty 2: heartbeat — your first lineage write proves you live");
    println!(
        "duty 3: verify only after the predecessor retires: caddis rotate verify --lineage {id}"
    );
    let (open, unverified) = gate_count(&dir);
    if unverified > 0 {
        println!("talk: {unverified} unverified turn(s) excluded — tamper-evidence bit");
    }
    if open > 0 {
        println!("talk: {open} unanswered finding(s) — retire-gate open");
    }
    Ok(())
}

/// CARD-0306 + 0318: findings with no later answer|fix — the retire-
/// gate count. Every turn line VERIFIES by mac first: a broken-mac
/// line is excluded and counted (tamper-evidence bites at the gate).
fn gate_count(dir: &std::path::Path) -> (usize, usize) {
    let turns = fs::read_to_string(dir.join("talk/turns.jsonl")).unwrap_or_default();
    let key = receipt::load_key(dir).unwrap_or_default();
    let mut open = 0usize;
    let mut unverified = 0usize;
    for line in turns.lines() {
        if !turn_mac(line, &key) {
            unverified += 1;
        } else if line.contains("\"kind\":\"finding\"") {
            open += 1;
        } else if line.contains("\"kind\":\"answer\"") || line.contains("\"kind\":\"fix\"") {
            open = open.saturating_sub(1);
        }
    }
    (open, unverified)
}

/// Recompute a turn's mac over the compose convention — one law.
fn turn_mac(line: &str, key: &[u8]) -> bool {
    let (Some(role), Some(pane), Some(kind), Some(text), Some(ts), Some(mac)) = (
        field(line, "role"),
        field(line, "pane"),
        field(line, "kind"),
        field(line, "text"),
        field(line, "ts"),
        field(line, "mac"),
    ) else {
        return false;
    };
    let expect =
        crate::hmac::hmac_sha256(key, format!("{role}|{pane}|{kind}|{text}|{ts}").as_bytes());
    receipt::hex_string(&expect) == mac
}

/// `"k":"v"` from a turn line, unescaping the compose set (\\ \").
fn field(line: &str, key: &str) -> Option<String> {
    let marker = format!("\"{key}\":\"");
    let a = line.find(&marker)? + marker.len();
    let mut out = String::new();
    let mut chars = line[a..].chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => match chars.next() {
                Some('\\') => out.push('\\'),
                Some('"') => out.push('"'),
                _ => return None, // not the compose escape set
            },
            _ => out.push(c),
        }
    }
    None
}

/// The spawn transaction: split AT the root, boot the SEAT with the
/// pointer (CARD-0315: a bare pointer is a letter with no reader).
fn spawn(id: &str) -> Result<(), Error> {
    let dir = lineage::dir(id).map_err(Error::Fail)?;
    let root = fs::read_to_string(dir.join("ready.root"))
        .map_err(|e| Error::Fail(format!("no ready.root (run ready at the work first): {e}")))?;
    let root = root.trim_end().to_string();
    if !std::path::Path::new(&root).is_dir() {
        return Err(Error::Fail(format!(
            "ready.root missing dir: {root} — re-run rotate ready"
        )));
    }
    let (kind, model) = seat_identity(&dir);
    let split = herdr(&[
        "pane",
        "split",
        "--current",
        "--direction",
        "right",
        "--cwd",
        &root,
        "--no-focus",
    ])
    .ok_or_else(|| Error::Fail("herdr unreachable — pane not split".into()))?;
    let Some(pane) = extract_pane_id(&split) else {
        return Err(Error::Fail(format!("pane split returned no id: {split}")));
    };
    println!("pane: {pane}");
    let seat = seat_cmd(id, &kind, &model);
    herdr(&["pane", "run", &pane, &seat])
        .ok_or_else(|| Error::Fail(format!("seat boot failed to {pane}")))?;
    println!("seat: sent ({seat})");
    // CARD-0315: only THIS pane's heartbeat proves the successor woke —
    // the predecessor's heartbeat must never mask an unbooted seat.
    let woke = fs::read_to_string(dir.join("heartbeat.receipt"))
        .ok()
        .and_then(|t| receipt::extract_field(t.as_bytes(), "pane"))
        .is_some_and(|p| p == pane);
    if woke {
        println!("heartbeat: present (this pane)");
        return Ok(());
    }
    let ts = receipt::timestamp();
    let body = format!("pane={pane}\nts={ts}\n");
    write_atomic(&dir, "armed-never-woke.lease", body.as_bytes()).map_err(Error::Fail)?;
    println!("heartbeat: not yet — armed-never-woke.lease written (the doctor watches)");
    Ok(())
}

/// CARD-0306 G1: the successor's first lineage write — proof of life.
fn heartbeat(id: &str) -> Result<(), Error> {
    let dir = lineage::dir(id).map_err(Error::Fail)?;
    let pane = std::env::var("HERDR_PANE_ID").unwrap_or_default();
    let ts = receipt::timestamp();
    let body = format!("pane={pane}\nts={ts}\n");
    write_atomic(&dir, "heartbeat.receipt", body.as_bytes()).map_err(Error::Fail)?;
    let _ = fs::remove_file(dir.join("armed-never-woke.lease")); // swallow: best-effort-cleanup — the wake clears the marker
    println!("heartbeat: pane={pane} ts={ts}");
    Ok(())
}

fn talk(args: &[String]) -> Result<(), Error> {
    let (id, rest) = lineage::take(args).map_err(Error::Usage)?;
    match rest.first().map(String::as_str) {
        Some("--post") => talk_post(&id, &rest[1..]),
        Some("--read") => talk_read(&id),
        _ => Err(Error::Usage(
            "usage: caddis restart talk --lineage <id> --post <finding|answer|fix|escalate> <text...> | --read".into(),
        )),
    }
}

fn talk_post(id: &str, args: &[String]) -> Result<(), Error> {
    let (kind, text) = parse_turn(args)?;
    validate_turn(kind, &text)?;
    let dir = lineage::dir(id).map_err(Error::Fail)?;
    let talk_dir = dir.join("talk");
    fs::create_dir_all(&talk_dir).map_err(|e| Error::Fail(format!("mkdir talk: {e}")))?;
    let line = compose_turn(&dir, kind, &text)?;
    let turns = talk_dir.join("turns.jsonl");
    let mut all = fs::read_to_string(&turns).unwrap_or_default();
    all.push_str(&line);
    write_atomic(&talk_dir, "turns.jsonl", all.as_bytes()).map_err(Error::Fail)?;
    println!("talk: {kind} posted");
    Ok(())
}

/// Split <kind> <text...>; usage errors stay here.
fn parse_turn(args: &[String]) -> Result<(&str, String), Error> {
    let kind = args
        .first()
        .ok_or_else(|| Error::Usage("talk --post requires <kind> <text...>".into()))?;
    if !matches!(kind.as_str(), "finding" | "answer" | "fix" | "escalate") {
        return Err(Error::Usage(format!("unknown talk kind {kind}")));
    }
    let text = args[1..].join(" ");
    if text.is_empty() {
        return Err(Error::Usage("talk --post requires text".into()));
    }
    Ok((kind, text))
}

/// E6 law: answer|fix turns carry an evidence path or do not land.
fn validate_turn(kind: &str, text: &str) -> Result<(), Error> {
    if matches!(kind, "answer" | "fix") && !(text.contains('/') || text.contains('\\')) {
        return Err(Error::Usage(format!(
            "{kind} turns REQUIRE an evidence path in the text (E6: receipts, not prose)"
        )));
    }
    Ok(())
}

/// One HMAC-stamped turn line; the key load is FALLIBLE (CARD-0318: never mint under a zero key).
fn compose_turn(dir: &std::path::Path, kind: &str, text: &str) -> Result<String, Error> {
    let pane = std::env::var("HERDR_PANE_ID").unwrap_or_default();
    let role = if pane.is_empty() { "past" } else { "present" };
    let ts = receipt::timestamp();
    let key = receipt::load_key(dir).map_err(Error::Fail)?;
    let mac =
        crate::hmac::hmac_sha256(&key, format!("{role}|{pane}|{kind}|{text}|{ts}").as_bytes());
    Ok(format!(
        "{{\"role\":\"{role}\",\"pane\":\"{pane}\",\"kind\":\"{kind}\",\"text\":\"{}\",\"ts\":\"{ts}\",\"mac\":\"{}\"}}\n",
        text.replace('\\', "\\\\").replace('"', "\\\""),
        crate::receipt::hex_string(&mac)
    ))
}

fn talk_read(id: &str) -> Result<(), Error> {
    let dir = lineage::dir(id).map_err(Error::Fail)?;
    let turns = fs::read_to_string(dir.join("talk/turns.jsonl")).unwrap_or_default();
    for line in turns.lines() {
        println!("{line}");
    }
    Ok(())
}

fn no_extra(rest: &[String]) -> Result<(), Error> {
    rest.first().map_or(Ok(()), |a| {
        Err(Error::Usage(format!("unknown argument {a}")))
    })
}
