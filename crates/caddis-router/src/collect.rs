//! Retroactive collector — council-consult source (P2 remainder slice 2).
//!
//! F2 ruling (quorum-caddis-router): retroactive telemetry IS the spine —
//! the router's first quality measurements come from dispatch trails that
//! ALREADY exist, zero friction. This module walks the council consult
//! archive (one dir per consult, `YYYYMMDD-HHMMSS-topic`, since 2026-07-04)
//! and replays every seat dispatch as an [`OutcomeRow`] in the router
//! ledger. 413 consults at birth; ~2.2k seat rows over 25 lanes.
//!
//! Laws encoded:
//! - **Model identity FROM THE TRANSPORT RECORD** (lesson-bank law): lane
//!   and model are the consult `MANIFEST.json`'s `provider`/`model`
//!   verbatim — never guessed from seat names, roles, or reply content. A
//!   consult without a manifest has no identity and contributes NO row
//!   (counted, skipped). `lane_id` convention `<provider>/<model>` starts
//!   HERE and the future registry feed must match it.
//! - **Outcome = the consult's own contract** (QQ1a's closest retroactive
//!   analogue): a seat's dispatch PASSED iff the consult recorded a parsed
//!   verdict for it — `stance` approve|mixed|reject with non-empty text. A
//!   `reject` stance is the seat doing its critic job — a PASS. `none` or
//!   an empty verdict = the seat delivered nothing usable = FAIL. A
//!   warden deny is still not a row (it never enters this stream).
//! - **Zero-cost honesty:** the consult trail records no tokens/usd/latency
//!   — those fields append as 0 meaning NOT RECORDED, never "free". The
//!   brief's row shape has no optional fields; live collectors (P4) carry
//!   real numbers.
//! - **Idempotency:** a row is skipped when its (card_id, lane_id) already
//!   exists as an outcome row, so re-running the collector is a no-op.
//!   Caveat, real (110 panels run two seats on one lane): after a crash
//!   mid-consult, a same-lane sibling whose first row landed is skipped —
//!   one lost sample, bounded, honest.
//! - **Chronology:** consult dirs sort by name and the naming convention
//!   IS the timeline, so ledger seq order = EWMA fold order = real time.
//!
//! Purity law (F1) holds: no dispatch happens here — the collector only
//! READS trails and APPENDS telemetry rows through [`Ledger::append`].

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use crate::ledger::{Ledger, LedgerErr, Loaded, Outcome, OutcomeRow, Row};

/// Task class for council-consult dispatches (advisory review). Floors for
/// this class reach [`crate::policy`] only when consult routing goes live;
/// until then the rows feed [`crate::stats`] capability folding.
pub const TASK_CLASS_CONSULT: &str = "consult";

/// One seat dispatch from a consult MANIFEST — the transport identity the
/// lesson-bank law demands, verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeatDispatch {
    pub seat: String,
    /// `<provider>/<model>` — the lane-id convention this collector starts.
    pub lane_id: String,
    /// MANIFEST `model`, verbatim.
    pub model: String,
}

/// Honest counts for one collect run. Every skip has its own counter; the
/// report is the operator's audit surface (model-voice convention: report
/// what IS).
#[derive(Debug, Default, PartialEq)]
pub struct CollectReport {
    /// Consult-shaped dirs examined.
    pub consults_seen: u32,
    /// Outcome rows appended (under `dry_run`: that WOULD be appended).
    pub rows: u32,
    pub passes: u32,
    pub fails: u32,
    /// Consult skipped: no MANIFEST.json (no transport identity — the
    /// lesson-bank law forbids guessing).
    pub skipped_no_manifest: u32,
    /// Consult skipped: MANIFEST.json present but unparseable.
    pub skipped_manifest_bad: u32,
    /// Consult skipped: no VERDICTS.json (no quality record at all).
    pub skipped_no_verdicts: u32,
    /// Consult skipped: VERDICTS.json present but unparseable.
    pub skipped_verdicts_bad: u32,
    /// Seat dropped: manifest entry without a usable provider/model.
    pub skipped_seat_no_identity: u32,
    /// Seat dropped: dispatch has identity but no verdict entry exists.
    pub skipped_seat_no_verdict: u32,
    /// Row skipped: (card_id, lane_id) already an outcome row.
    pub skipped_already: u32,
    pub dry_run: bool,
}

#[derive(Debug)]
pub enum CollectErr {
    Io(std::io::Error),
    Ledger(LedgerErr),
}

impl From<std::io::Error> for CollectErr {
    fn from(e: std::io::Error) -> Self {
        CollectErr::Io(e)
    }
}

impl From<LedgerErr> for CollectErr {
    fn from(e: LedgerErr) -> Self {
        CollectErr::Ledger(e)
    }
}

impl std::fmt::Display for CollectErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CollectErr::Io(e) => write!(f, "io: {e}"),
            CollectErr::Ledger(e) => write!(f, "ledger: {e}"),
        }
    }
}

/// A consult dir name is `<8 digits>-<6 digits>-<topic>` (YYYYMMDD-HHMMSS)
/// — the timestamp prefix IS the chronology the seq order relies on.
fn is_consult_dir(name: &str) -> bool {
    let b = name.as_bytes();
    b.len() > 16
        && b[..8].iter().all(|c| c.is_ascii_digit())
        && b[8] == b'-'
        && b[9..15].iter().all(|c| c.is_ascii_digit())
        && b[15] == b'-'
}

/// Walk `councils`, replay every consult as outcome rows into `ledger`.
///
/// Consult-level parse failures are COUNTED SKIPS, not errors — an append
/// tool keeps its partial progress and reports what it could not read.
/// Only IO and ledger failures are hard errors.
pub fn collect_councils(
    councils: &Path,
    ledger: &Ledger,
    dry_run: bool,
) -> Result<CollectReport, CollectErr> {
    let mut rep = CollectReport {
        dry_run,
        ..CollectReport::default()
    };

    // Cross-run idempotency key: every outcome row already in the stream.
    let Loaded { rows: existing, .. } = ledger.load()?;
    let seen: BTreeSet<(String, String)> = existing
        .iter()
        .filter_map(|pr| match &pr.row {
            Row::Outcome(o) => Some((o.card_id.clone(), o.lane_id.clone())),
            _ => None,
        })
        .collect();

    let mut names: Vec<String> = fs::read_dir(councils)?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| is_consult_dir(n))
        .collect();
    names.sort(); // naming convention = timeline; seq order = fold order
    rep.consults_seen = names.len() as u32;

    for name in names {
        let dir = councils.join(&name);
        let card_id = format!("council/{name}");

        let mtext = match fs::read_to_string(dir.join("MANIFEST.json")) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                rep.skipped_no_manifest += 1;
                continue;
            }
            Err(e) => return Err(e.into()),
        };
        let (seats, no_identity) = match parse_manifest(&mtext) {
            Ok(v) => v,
            Err(_) => {
                rep.skipped_manifest_bad += 1;
                continue;
            }
        };
        rep.skipped_seat_no_identity += no_identity;

        let vtext = match fs::read_to_string(dir.join("VERDICTS.json")) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                rep.skipped_no_verdicts += 1;
                continue;
            }
            Err(e) => return Err(e.into()),
        };
        let verdicts = match parse_verdicts(&vtext) {
            Ok(v) => v,
            Err(_) => {
                rep.skipped_verdicts_bad += 1;
                continue;
            }
        };

        for sd in seats {
            if seen.contains(&(card_id.clone(), sd.lane_id.clone())) {
                rep.skipped_already += 1;
                continue;
            }
            let Some((stance, text)) = verdicts.get(&sd.seat) else {
                rep.skipped_seat_no_verdict += 1;
                continue;
            };
            // The consult contract: a PARSED verdict arrived. A reject is
            // the critic doing its job — a pass. None/empty = fail.
            let outcome =
                if matches!(stance.as_str(), "approve" | "mixed" | "reject") && !text.is_empty() {
                    Outcome::Pass
                } else {
                    Outcome::Fail
                };
            let row = Row::Outcome(OutcomeRow {
                card_id: card_id.clone(),
                task_class: TASK_CLASS_CONSULT.to_string(),
                lane_id: sd.lane_id.clone(),
                model: sd.model.clone(),
                // Not recorded in the consult trail — never "free".
                cost_tokens: 0,
                cost_usd_est: 0.0,
                latency_ms: 0,
                outcome,
                escalated_to: None,
            });
            if !dry_run {
                ledger.append(&row)?;
            }
            rep.rows += 1;
            match outcome {
                Outcome::Pass => rep.passes += 1,
                Outcome::Fail => rep.fails += 1,
            }
        }
    }
    Ok(rep)
}

// ---------------------------------------------------------------------------
// Bee-ledger source (P2 remainder slice 3a) — BEE-CARDS.json replay
// ---------------------------------------------------------------------------

/// Task class for bee card executions (mechanical feed/office work).
pub const TASK_CLASS_BEE: &str = "bee-card";

/// One bee lane from the bee lane registry (BEE-LANES.json + loop-runner
/// `started (model=...)` lines — the transport-side record of which model
/// each loop runs). Lane resolution NEVER guesses: a card resolves only
/// when its transport-written `assigned` field names a bee (`KAMANĖ`), a
/// loop (`bee`), or the lane's model string (`glm-5.2`) — all verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BeeLane {
    /// Bee name as it appears in `assigned` (matched case-insensitively
    /// by substring, so `KAMANĖ`/`KAMANE`/`kamane` all resolve).
    pub bee: &'static str,
    /// Loop id as it appears in `assigned` (loop `bee` = KAMANĖ, `bee2` =
    /// BITUTE — the registry's own mapping).
    pub loop_id: &'static str,
    /// `<provider>/<model>` — the lane-id convention from slice 2.
    pub lane_id: &'static str,
    /// Model string as `assigned` spells it when it names the model.
    pub model: &'static str,
}

/// The bee lane registry this collector resolves against. Provenance:
/// `~/.omp/sergeant/state/BEE-LANES.json` (kamane -> loop `bee`, glm-5.2
/// via z.ai; bitute -> loop `bee2`, llama3.2:3b-64k local ollama) and the
/// runner logs' `loop:bee: started (model=glm-5.2, ...)` lines.
pub const BEE_LANES: [BeeLane; 2] = [
    BeeLane {
        // "kaman" not "kamane": the diacritic in KAMANĖ would break a
        // full substring match; the 5-char prefix matches every spelling.
        bee: "kaman",
        loop_id: "bee",
        lane_id: "zai/glm-5.2",
        model: "glm-5.2",
    },
    BeeLane {
        bee: "bitute",
        loop_id: "bee2",
        lane_id: "ollama/llama3.2:3b-64k",
        model: "llama3.2:3b-64k",
    },
];

/// Honest counts for one bee collect run (model-voice convention).
#[derive(Debug, Default, PartialEq)]
pub struct BeeReport {
    /// Card objects examined.
    pub cards_seen: u32,
    /// Outcome rows appended (under `dry_run`: that WOULD be appended).
    pub rows: u32,
    /// Rows carry Pass only — see [`collect_bees`]: the bee trail has no
    /// fail representation; quality folds get sample counts.
    pub passes: u32,
    /// Card skipped: status is not `done` (blocked-* is an external block
    /// — the warden-deny analogue, it never enters the stream).
    pub skipped_not_done: u32,
    /// Card skipped: no usable `id` string.
    pub skipped_no_id: u32,
    /// Card skipped: `assigned` proves no lane (claim-time quirk, empty,
    /// or a model outside the registry — never guessed from card content).
    pub skipped_no_lane: u32,
    /// Row skipped: (card_id, lane_id) already an outcome row.
    pub skipped_already: u32,
    pub dry_run: bool,
}

/// Resolve one `assigned` value to a registry lane. The claim-time quirk
/// (the bee loop writes the claim TIMESTAMP into `assigned`) lands in
/// `None` — a timestamp names no lane, and the lesson-bank law forbids
/// reconstructing identity from note content.
fn resolve_bee_lane(assigned: &str) -> Option<&'static BeeLane> {
    let a = assigned.trim().to_lowercase();
    if a.is_empty() {
        return None;
    }
    // A timestamp (claim-time quirk) or any non-registry string falls
    // through every arm below -> None.
    BEE_LANES
        .iter()
        .find(|l| a.contains(l.bee) || a == l.loop_id || a == l.model)
}

/// Walk `cards_json` (BEE-CARDS.json shape `{"cards":[...]}`) and replay
/// every DONE card as an outcome row into `ledger`.
///
/// Laws encoded (slice-2 law applied to the bee trail):
/// - **Identity from the transport-written `assigned` field only**, via
///   [`BEE_LANES`]. The bee loop's claim-time quirk (a timestamp in
///   `assigned`) and note-content markers are NOT identity — such cards
///   are counted skips until the loop writes the model in (the fix the
///   beekeeper census asked for). Misattributed rows corrupt two lanes;
///   missing rows cost one sample.
/// - **Outcome = the card's own contract**: `done` means the bee verified
///   its Done-When (recorded in the card note). The bee trail has NO fail
///   representation — failed attempts stay `assigned` and contribute no
///   row — so every row here is Pass and the report says so honestly.
/// - **Zero-cost honesty:** the bee trail records no tokens/usd/latency;
///   those append as 0 meaning NOT RECORDED.
/// - **Idempotency:** (card_id, lane_id) already present -> skip, counted.
pub fn collect_bees(
    cards_json: &Path,
    ledger: &Ledger,
    dry_run: bool,
) -> Result<BeeReport, CollectErr> {
    let mut rep = BeeReport {
        dry_run,
        ..BeeReport::default()
    };

    let seen: BTreeSet<(String, String)> = ledger
        .load()?
        .rows
        .iter()
        .filter_map(|pr| match &pr.row {
            Row::Outcome(o) => Some((o.card_id.clone(), o.lane_id.clone())),
            _ => None,
        })
        .collect();

    let text = fs::read_to_string(cards_json)?;
    let members = split_members(&text)
        .map_err(|_| CollectErr::Io(std::io::Error::other("cards file is not a JSON object")))?;
    let Some(cards_raw) = members
        .iter()
        .find(|(k, _)| k == "cards")
        .map(|(_, v)| v.trim())
    else {
        return Err(CollectErr::Io(std::io::Error::other(
            "cards file has no 'cards' array",
        )));
    };
    let card_texts = split_array(cards_raw)
        .map_err(|_| CollectErr::Io(std::io::Error::other("'cards' is not a JSON array")))?;
    rep.cards_seen = card_texts.len() as u32;

    for ct in card_texts {
        // id/status/assigned are flat string members; nested arrays
        // (steps) are raw-captured and ignored.
        let Ok(fields) = split_members(&ct) else {
            rep.skipped_no_id += 1;
            continue;
        };
        let id = str_val(&fields, "id").unwrap_or_default();
        if id.is_empty() {
            rep.skipped_no_id += 1;
            continue;
        }
        let status = str_val(&fields, "status").unwrap_or_default();
        if status != "done" {
            rep.skipped_not_done += 1;
            continue;
        }
        let assigned = str_val(&fields, "assigned").unwrap_or_default();
        let Some(lane) = resolve_bee_lane(&assigned) else {
            rep.skipped_no_lane += 1;
            continue;
        };
        let card_id = format!("bee/{id}");
        if seen.contains(&(card_id.clone(), lane.lane_id.to_string())) {
            rep.skipped_already += 1;
            continue;
        }
        let row = Row::Outcome(OutcomeRow {
            card_id,
            task_class: TASK_CLASS_BEE.to_string(),
            lane_id: lane.lane_id.to_string(),
            model: lane.model.to_string(),
            // Not recorded in the bee trail — never "free".
            cost_tokens: 0,
            cost_usd_est: 0.0,
            latency_ms: 0,
            outcome: Outcome::Pass,
            escalated_to: None,
        });
        if !dry_run {
            ledger.append(&row)?;
        }
        rep.rows += 1;
        rep.passes += 1;
    }
    Ok(rep)
}

// ---------------------------------------------------------------------------
// Trail parsers — one nesting level, the only shape MANIFEST/VERDICTS have
// ---------------------------------------------------------------------------

/// Parse a consult MANIFEST into seat dispatches. Returns the identity
/// triples plus the count of entries dropped for missing identity.
fn parse_manifest(text: &str) -> Result<(Vec<SeatDispatch>, u32), String> {
    let mut seats = Vec::new();
    let mut no_identity = 0u32;
    for (seat, vtext) in split_members(text)? {
        let leaf = match leaf_members(&vtext) {
            Some(m) => m,
            None => {
                no_identity += 1;
                continue;
            }
        };
        let (provider, model) = match (str_val(&leaf, "provider"), str_val(&leaf, "model")) {
            (Some(p), Some(m)) if !p.is_empty() && !m.is_empty() => (p, m),
            _ => {
                no_identity += 1;
                continue;
            }
        };
        seats.push(SeatDispatch {
            seat,
            lane_id: format!("{provider}/{model}"),
            model,
        });
    }
    Ok((seats, no_identity))
}

/// Parse a consult VERDICTS file into `seat -> (stance, verdict text)`.
fn parse_verdicts(text: &str) -> Result<BTreeMap<String, (String, String)>, String> {
    let mut out = BTreeMap::new();
    for (seat, vtext) in split_members(text)? {
        let Some(leaf) = leaf_members(&vtext) else {
            continue;
        };
        let stance = str_val(&leaf, "stance").unwrap_or_default();
        let verdict = str_val(&leaf, "verdict").unwrap_or_default();
        out.insert(seat, (stance, verdict));
    }
    Ok(out)
}

/// Members of a nested-object value: `None` when the value is not an
/// object (a scalar/array seat entry carries no identity).
fn leaf_members(vtext: &str) -> Option<Vec<(String, String)>> {
    let t = vtext.trim();
    if t.starts_with('{') {
        split_members(t).ok()
    } else {
        None
    }
}

/// Decode one member's raw value as a JSON string literal, if it is one.
fn str_val(members: &[(String, String)], key: &str) -> Option<String> {
    let raw = members.iter().find(|(k, _)| k == key)?.1.trim();
    let b: Vec<char> = raw.chars().collect();
    let mut i = 0usize;
    let s = scan_string(&b, &mut i).ok()?;
    while i < b.len() && b[i].is_whitespace() {
        i += 1;
    }
    (i == b.len()).then_some(s)
}

/// Split a one-level-nested JSON object into `(key, raw-value-text)`
/// members. Nested objects are captured verbatim for flat re-parse; arrays
/// (MANIFEST `fileref`) are captured as raw text the callers ignore —
/// skipping wholesale beats half-supporting nesting.
fn split_members(text: &str) -> Result<Vec<(String, String)>, String> {
    let b: Vec<char> = text.chars().collect();
    let n = b.len();
    let mut i = 0usize;
    let ws = |i: &mut usize| {
        while *i < n && b[*i].is_whitespace() {
            *i += 1;
        }
    };
    ws(&mut i);
    if i >= n || b[i] != '{' {
        return Err("expected '{'".into());
    }
    i += 1;
    let mut out = Vec::new();
    loop {
        ws(&mut i);
        if i < n && b[i] == '}' {
            return Ok(out);
        }
        // key
        if i >= n || b[i] != '"' {
            return Err(format!("expected key string at {}", i));
        }
        let key = scan_string(&b, &mut i)?;
        ws(&mut i);
        if i >= n || b[i] != ':' {
            return Err(format!("expected ':' after key '{key}'"));
        }
        i += 1;
        ws(&mut i);
        let start = i;
        skip_value(&b, &mut i)?;
        out.push((key, b[start..i].iter().collect()));
        ws(&mut i);
        match b.get(i) {
            Some(',') => i += 1,
            Some('}') => return Ok(out),
            _ => return Err("expected ',' or '}'".into()),
        }
    }
}

/// Split a top-level JSON array text into its element texts (each a
/// verbatim slice). Used by the bee collector over `{"cards":[...]}`.
fn split_array(text: &str) -> Result<Vec<String>, String> {
    let b: Vec<char> = text.chars().collect();
    let n = b.len();
    let mut i = 0usize;
    while i < n && b[i].is_whitespace() {
        i += 1;
    }
    if i >= n || b[i] != '[' {
        return Err("expected '['".into());
    }
    i += 1;
    let mut out = Vec::new();
    loop {
        while i < n && b[i].is_whitespace() {
            i += 1;
        }
        if i < n && b[i] == ']' {
            return Ok(out);
        }
        let start = i;
        skip_value(&b, &mut i)?;
        out.push(b[start..i].iter().collect());
        while i < n && b[i].is_whitespace() {
            i += 1;
        }
        match b.get(i) {
            Some(',') => i += 1,
            Some(']') => return Ok(out),
            _ => return Err("expected ',' or ']'".into()),
        }
    }
}

/// Consume one JSON value starting at `i` (string / number / bool / null /
/// object / array), leaving `i` one past its last char.
fn skip_value(b: &[char], i: &mut usize) -> Result<(), String> {
    match b.get(*i) {
        Some('"') => {
            scan_string(b, i)?;
        }
        Some('{') | Some('[') => {
            let open = b[*i];
            let close = if open == '{' { '}' } else { ']' };
            *i += 1;
            let mut depth = 1usize;
            while *i < b.len() {
                match b[*i] {
                    '"' => {
                        scan_string(b, i)?;
                        continue;
                    }
                    c if c == open => depth += 1,
                    c if c == close => {
                        depth -= 1;
                        if depth == 0 {
                            *i += 1;
                            return Ok(());
                        }
                    }
                    _ => {}
                }
                *i += 1;
            }
            return Err("unterminated container".into());
        }
        Some(c) if *c == 't' || *c == 'f' || *c == 'n' || c.is_ascii_digit() || *c == '-' => {
            while *i < b.len() {
                let c = b[*i];
                if c == ',' || c == '}' || c == ']' || c.is_whitespace() {
                    break;
                }
                *i += 1;
            }
        }
        _ => return Err(format!("unexpected value start at {}", *i)),
    }
    Ok(())
}

/// Consume a quoted JSON string starting at `i` ('"' MUST be at `i`),
/// handling `\\` escapes, leaving `i` one past the closing quote. Content is
/// returned but callers here only need the SPAN.
fn scan_string(b: &[char], i: &mut usize) -> Result<String, String> {
    if b.get(*i) != Some(&'"') {
        return Err("expected string".into());
    }
    *i += 1;
    let mut out = String::new();
    while *i < b.len() {
        match b[*i] {
            '"' => {
                *i += 1;
                return Ok(out);
            }
            '\\' => {
                *i += 1;
                match b.get(*i) {
                    Some(e @ ('"' | '\\' | '/' | 'b' | 'f' | 'n' | 'r' | 't')) => {
                        out.push(match e {
                            'b' => '\u{8}',
                            'f' => '\u{c}',
                            'n' => '\n',
                            'r' => '\r',
                            't' => '\t',
                            o => *o,
                        });
                        *i += 1;
                    }
                    Some('u') => {
                        if *i + 4 >= b.len() {
                            return Err("truncated \\u escape".into());
                        }
                        let hex: String = b[*i + 1..*i + 5].iter().collect();
                        let cp = u32::from_str_radix(&hex, 16)
                            .map_err(|_| "bad \\u escape".to_string())?;
                        out.push(char::from_u32(cp).ok_or("bad \\u codepoint")?);
                        *i += 5;
                    }
                    _ => return Err("bad escape".into()),
                }
            }
            c => {
                out.push(c);
                *i += 1;
            }
        }
    }
    Err("unterminated string".into())
}

#[cfg(test)]
#[path = "collect_tests.rs"]
mod tests;
