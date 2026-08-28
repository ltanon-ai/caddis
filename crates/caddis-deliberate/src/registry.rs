//! registry.rs — P1: the SEAT REGISTRY as an append-only card
//! stream (BUILD-QUEUE r2-organs-rewrite; plan P1; F2 — groq Q2 + nvidia
//! R2: "seat registry = append-only card stream (truth) + CACHED JSON
//! VIEW for fast reads, re-synced on each row; edits via warden-gated
//! propose→operator-confirm").
//!
//! Laws transcribed:
//! - **TRUTH = the stream** (`seats.jsonl`). The view (`seats-view.json`)
//!   is a CACHE: it carries the sha256 of the stream bytes it was derived
//!   from; a loader that finds a mismatch re-derives and rewrites it. The
//!   view is never hand-edited, never trusted blindly.
//! - **One flat JSON object per line** (router JSONL precedent): the card
//!   grammar is FLAT — a nested value is malformed, an unknown field is
//!   malformed, a missing field is malformed. A typo must never silently
//!   drop the row it was trying to change (policy-file law).
//! - **Append-only**: edits are NEW rows; the fold is LAST-row-per-id
//!   wins. History is never rewritten. Superseding rows arrive through
//!   the warden-gated propose→operator-confirm path (P1 slice 3); this
//!   slice ships the stream mechanics + the seed collector only.
//! - **Deterministic bytes**: `render_seed` emits the same bytes for the
//!   same card set — no timestamps inside cards (the idempotency
//!   Done-When). Provenance is the deterministic `source` field
//!   (`models.json#<digest8>` from the collector).
//! - **No secrets, ever**: the card fields below carry no credential
//!   material. `auth_path` is a VAULT PATH (a file path), never a key
//!   value — the collector enforces this at the boundary (Ruling 9).
//!
//! P1 slice 2 extends the grammar: provider cards carry `caps`
//! (Ruling 7 per-provider concurrency) and seat cards carry
//! `since_epoch_s` (when the seat entered its state; 0 = the clock-free
//! seed). The laws that READ them live in [`crate::caps`] (cap law +
//! dispatch planner) and [`crate::ttl`] (TTL state machine) — this
//! module stays the stream grammar only.
//! - **Fail-closed I/O**: a malformed line refuses the WHOLE load with
//!   the 1-based line number (the registry never half-loads); an append
//!   re-derives the view from the full stream, not an incremental patch
//!   (F2 "re-synced on each row" — simple, provable, cheap at this size).

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::json::{self, Value};
use crate::sha256;

/// Wire word for a provider card row.
pub const CLASS_PROVIDER: &str = "provider";
/// Wire word for a seat card row.
pub const CLASS_SEAT: &str = "seat";

// ---------------------------------------------------------------------------
// Wire words for the P0 enums (one vocabulary, parse refuses the rest).
// ---------------------------------------------------------------------------

/// [`LaneType`](crate::LaneType) wire words, in enum order.
pub fn lane_type_words() -> [&'static str; 3] {
    ["http", "bridge", "cli"]
}

/// Pub: sibling grammars (session cards) share the ONE vocabulary law —
/// a second wire-word copy anywhere is banned.
pub fn parse_lane_type(s: &str) -> Option<crate::LaneType> {
    match s {
        "http" => Some(crate::LaneType::Http),
        "bridge" => Some(crate::LaneType::Bridge),
        "cli" => Some(crate::LaneType::Cli),
        _ => None,
    }
}

pub fn lane_type_word(lt: crate::LaneType) -> &'static str {
    match lt {
        crate::LaneType::Http => "http",
        crate::LaneType::Bridge => "bridge",
        crate::LaneType::Cli => "cli",
    }
}

pub fn parse_cost_class(s: &str) -> Option<crate::CostClass> {
    match s {
        "free" => Some(crate::CostClass::Free),
        "mid" => Some(crate::CostClass::Mid),
        "premium" => Some(crate::CostClass::Premium),
        _ => None,
    }
}

pub fn cost_class_word(cc: crate::CostClass) -> &'static str {
    match cc {
        crate::CostClass::Free => "free",
        crate::CostClass::Mid => "mid",
        crate::CostClass::Premium => "premium",
    }
}

fn parse_seat_state(s: &str) -> Option<crate::SeatState> {
    match s {
        "live" => Some(crate::SeatState::Live),
        "expired" => Some(crate::SeatState::Expired),
        "rate-limited" => Some(crate::SeatState::RateLimited),
        "retired" => Some(crate::SeatState::Retired),
        "probing" => Some(crate::SeatState::Probing),
        "failed" => Some(crate::SeatState::Failed),
        _ => None,
    }
}

fn seat_state_word(st: crate::SeatState) -> &'static str {
    match st {
        crate::SeatState::Live => "live",
        crate::SeatState::Expired => "expired",
        crate::SeatState::RateLimited => "rate-limited",
        crate::SeatState::Retired => "retired",
        crate::SeatState::Probing => "probing",
        crate::SeatState::Failed => "failed",
    }
}

// ---------------------------------------------------------------------------
// Cards
// ---------------------------------------------------------------------------

/// One registered provider row (Ruling 9 superset: provider+model
/// registration; per-provider auth as vault PATHS).
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderCard {
    pub id: String,
    pub lane_type: crate::LaneType,
    /// Honest blank ("") when the source carries none (e.g. per-model api).
    pub base_url: String,
    /// VAULT PATH only — never a credential value. Empty = auth lives in
    /// the source file, un-copied (the collector's honest blank).
    pub auth_path: String,
    /// Max CONCURRENT dispatches across all this provider's seats
    /// (Ruling 7). ollama/ollama-cloud rule 1 with hard ceiling 2;
    /// others seed the F4 serialized default 1. The law + planner live
    /// in [`crate::caps`].
    pub caps: u32,
    /// Deterministic provenance, e.g. `models.json#a1b2c3d4`.
    pub source: String,
}

/// One registered seat row (a deliberation seat = provider x model lane).
#[derive(Debug, Clone, PartialEq)]
pub struct SeatCard {
    /// Registry key. Collector law: `<provider>/<model>`.
    pub id: String,
    pub provider: String,
    /// Family grouping — floors count DISTINCT families (monoculture
    /// guard). Collector law: family = provider id; a ruling may re-group.
    pub family: String,
    pub model: String,
    pub lane_type: crate::LaneType,
    pub cost_class: crate::CostClass,
    pub state: crate::SeatState,
    /// When this seat entered `state` (Unix epoch seconds). `0` = no
    /// clock data — exactly the deterministic collector seed (no clocks
    /// in seed cards); every later state-change row stamps `now`. The
    /// TTL machine ([`crate::ttl`]) reads it; `Probing` + 0 = "never
    /// probed" (first probe due now), not "probed at epoch".
    pub since_epoch_s: u64,
    /// Max concurrent dispatches for THIS seat — the effective cap is
    /// the min with the provider row's ([`crate::caps::effective_caps`]).
    /// Seeds carry 1 = serialized-by-default (F4).
    pub caps: u32,
    /// Measured facts, USD per 1M tokens (0 = the lane bills nothing).
    pub cost_in_usd_per_mtok: f64,
    pub cost_out_usd_per_mtok: f64,
    pub context_window: u64,
    pub max_tokens: u64,
    pub source: String,
}

impl SeatCard {
    /// Project onto the P0 substrate [`Seat`](crate::Seat) so panel
    /// construction and quorum-pool selection consume the REGISTRY, not
    /// ad-hoc seat lists (one law, one selection order). `caps` travels;
    /// `last_probe` is None — probing is P1 slice 2 / P3 work.
    pub fn to_seat(&self) -> crate::Seat {
        crate::Seat {
            lane_id: self.id.clone(),
            lane_type: self.lane_type,
            family: self.family.clone(),
            provider: self.provider.clone(),
            model: self.model.clone(),
            cost_class: self.cost_class,
            state: self.state,
            caps: self.caps,
            last_probe: None,
        }
    }
}

/// One stream row.
#[derive(Debug, Clone, PartialEq)]
pub enum Card {
    Provider(ProviderCard),
    Seat(SeatCard),
}

impl Card {
    /// The fold key: class + id. Superseding rows share the key.
    pub fn key(&self) -> (&'static str, &str) {
        match self {
            Card::Provider(p) => (CLASS_PROVIDER, p.id.as_str()),
            Card::Seat(s) => (CLASS_SEAT, s.id.as_str()),
        }
    }
}

// ---------------------------------------------------------------------------
// Encode (deterministic; what the writer writes is the only shape the
// loader accepts — the router audit==obey law)
// ---------------------------------------------------------------------------

/// One flat JSON object, fixed key order, no spaces after `:`/`,` —
/// byte-deterministic for a given card.
pub fn encode_card(card: &Card) -> String {
    let mut o = String::new();
    match card {
        Card::Provider(p) => {
            o.push_str("{\"class\":\"provider\",\"id\":");
            json_str(&p.id, &mut o);
            o.push_str(",\"lane_type\":\"");
            o.push_str(lane_type_word(p.lane_type));
            o.push_str("\",\"base_url\":");
            json_str(&p.base_url, &mut o);
            o.push_str(",\"auth_path\":");
            json_str(&p.auth_path, &mut o);
            o.push_str(",\"caps\":");
            o.push_str(&p.caps.to_string());
            o.push_str(",\"source\":");
            json_str(&p.source, &mut o);
            o.push('}');
        }
        Card::Seat(s) => {
            o.push_str("{\"class\":\"seat\",\"id\":");
            json_str(&s.id, &mut o);
            o.push_str(",\"provider\":");
            json_str(&s.provider, &mut o);
            o.push_str(",\"family\":");
            json_str(&s.family, &mut o);
            o.push_str(",\"model\":");
            json_str(&s.model, &mut o);
            o.push_str(",\"lane_type\":\"");
            o.push_str(lane_type_word(s.lane_type));
            o.push_str("\",\"cost_class\":\"");
            o.push_str(cost_class_word(s.cost_class));
            o.push_str("\",\"state\":\"");
            o.push_str(seat_state_word(s.state));
            o.push_str("\",\"since_epoch_s\":");
            o.push_str(&s.since_epoch_s.to_string());
            o.push_str(",\"caps\":");
            o.push_str(&s.caps.to_string());
            o.push_str(",\"cost_in_usd_per_mtok\":");
            push_num(s.cost_in_usd_per_mtok, &mut o);
            o.push_str(",\"cost_out_usd_per_mtok\":");
            push_num(s.cost_out_usd_per_mtok, &mut o);
            o.push_str(",\"context_window\":");
            o.push_str(&s.context_window.to_string());
            o.push_str(",\"max_tokens\":");
            o.push_str(&s.max_tokens.to_string());
            o.push_str(",\"source\":");
            json_str(&s.source, &mut o);
            o.push('}');
        }
    }
    o
}

/// JSON string literal with the vendored writer's escape law (keeps
/// encode_card byte-identical to json::to_string for string fields).
fn json_str(s: &str, out: &mut String) {
    out.push_str(&json::to_string(&Value::Str(s.to_string())));
}

/// Finite, >= 0, shortest round-trip form (the vendored writer's number
/// law). NaN/inf/negative are REFUSED at parse; encode asserts the same
/// by construction — debug_assert for the writer, error for the loader.
fn push_num(n: f64, out: &mut String) {
    debug_assert!(n.is_finite() && n >= 0.0, "card numbers are finite >= 0");
    out.push_str(&json::to_string(&Value::Num(n)));
}

/// Render a full seed stream: one line per card, LF-terminated, cards in
/// the GIVEN order (the collector sorts deterministically). Same cards in
/// ⇒ same bytes out — the idempotency Done-When.
pub fn render_seed(cards: &[Card]) -> String {
    let mut out = String::new();
    for c in cards {
        out.push_str(&encode_card(c));
        out.push('\n');
    }
    out
}

// ---------------------------------------------------------------------------
// Parse (exact field law; fail-closed with the 1-based line number)
// ---------------------------------------------------------------------------

/// Stream load refusals. Malformed rows carry the 1-based line number —
/// the registry never half-loads.
#[derive(Debug, PartialEq)]
pub enum StreamErr {
    Malformed { line: usize, msg: String },
}

impl fmt::Display for StreamErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StreamErr::Malformed { line, msg } => write!(f, "seats.jsonl line {line}: {msg}"),
        }
    }
}

impl std::error::Error for StreamErr {}

/// Parse stream text into cards, IN ORDER (the fold is the caller's; the
/// parse is pure). Empty lines are skipped; every other line must parse
/// as a card with its EXACT field set.
pub fn parse_stream(text: &str) -> Result<Vec<Card>, StreamErr> {
    let mut out = Vec::new();
    for (idx, raw) in text.lines().enumerate() {
        let line_no = idx + 1;
        let t = raw.trim();
        if t.is_empty() {
            continue;
        }
        out.push(parse_line(t, line_no)?);
    }
    Ok(out)
}

fn parse_line(t: &str, line_no: usize) -> Result<Card, StreamErr> {
    let v = json::parse(t).map_err(|e| StreamErr::Malformed {
        line: line_no,
        msg: format!("not JSON: {}", e.msg),
    })?;
    let obj = v.as_obj().ok_or_else(|| StreamErr::Malformed {
        line: line_no,
        msg: "card must be a flat JSON object".into(),
    })?;
    // Flat law: any nested value is malformed, before field checks.
    for (k, val) in obj {
        if !matches!(val, Value::Str(_) | Value::Num(_) | Value::Bool(_)) {
            return Err(StreamErr::Malformed {
                line: line_no,
                msg: format!("nested value at \"{k}\" — cards are flat"),
            });
        }
    }
    let class = str_field(obj, "class", line_no)?;
    match class.as_str() {
        CLASS_PROVIDER => parse_provider(obj, line_no),
        CLASS_SEAT => parse_seat(obj, line_no),
        other => Err(StreamErr::Malformed {
            line: line_no,
            msg: format!("unknown class {other:?}"),
        }),
    }
}

fn field<'a>(
    obj: &'a [(String, Value)],
    key: &str,
    line_no: usize,
) -> Result<&'a Value, StreamErr> {
    obj.iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v)
        .ok_or_else(|| StreamErr::Malformed {
            line: line_no,
            msg: format!("missing field \"{key}\""),
        })
}

fn str_field(obj: &[(String, Value)], key: &str, line_no: usize) -> Result<String, StreamErr> {
    match field(obj, key, line_no)? {
        Value::Str(s) => Ok(s.clone()),
        _ => Err(StreamErr::Malformed {
            line: line_no,
            msg: format!("field \"{key}\" must be a string"),
        }),
    }
}

fn u32_field(obj: &[(String, Value)], key: &str, line_no: usize) -> Result<u32, StreamErr> {
    num_field(obj, key, line_no).and_then(|n| {
        if n.fract() != 0.0 || n < 0.0 || n > u32::MAX as f64 {
            Err(StreamErr::Malformed {
                line: line_no,
                msg: format!("field \"{key}\" must be a non-negative integer"),
            })
        } else {
            Ok(n as u32)
        }
    })
}

fn u64_field(obj: &[(String, Value)], key: &str, line_no: usize) -> Result<u64, StreamErr> {
    num_field(obj, key, line_no).and_then(|n| {
        if n.fract() != 0.0 || n < 0.0 || n > u64::MAX as f64 {
            Err(StreamErr::Malformed {
                line: line_no,
                msg: format!("field \"{key}\" must be a non-negative integer"),
            })
        } else {
            Ok(n as u64)
        }
    })
}

fn num_field(obj: &[(String, Value)], key: &str, line_no: usize) -> Result<f64, StreamErr> {
    match field(obj, key, line_no)? {
        Value::Num(n) if n.is_finite() && *n >= 0.0 => Ok(*n),
        _ => Err(StreamErr::Malformed {
            line: line_no,
            msg: format!("field \"{key}\" must be a finite number >= 0"),
        }),
    }
}

fn lane_type_field(obj: &[(String, Value)], line_no: usize) -> Result<crate::LaneType, StreamErr> {
    let w = str_field(obj, "lane_type", line_no)?;
    parse_lane_type(&w).ok_or_else(|| StreamErr::Malformed {
        line: line_no,
        msg: format!("unknown lane_type {w:?} (vocabulary: http|bridge|cli — Ruling 5)"),
    })
}

fn non_empty(obj: &[(String, Value)], key: &str, line_no: usize) -> Result<String, StreamErr> {
    let s = str_field(obj, key, line_no)?;
    if s.is_empty() {
        Err(StreamErr::Malformed {
            line: line_no,
            msg: format!("field \"{key}\" must be non-empty"),
        })
    } else {
        Ok(s)
    }
}

const PROVIDER_FIELDS: &[&str] = &[
    "class",
    "id",
    "lane_type",
    "base_url",
    "auth_path",
    "caps",
    "source",
];
const SEAT_FIELDS: &[&str] = &[
    "class",
    "id",
    "provider",
    "family",
    "model",
    "lane_type",
    "cost_class",
    "state",
    "since_epoch_s",
    "caps",
    "cost_in_usd_per_mtok",
    "cost_out_usd_per_mtok",
    "context_window",
    "max_tokens",
    "source",
];

fn exact_fields(obj: &[(String, Value)], want: &[&str], line_no: usize) -> Result<(), StreamErr> {
    for (k, _) in obj {
        if !want.contains(&k.as_str()) {
            return Err(StreamErr::Malformed {
                line: line_no,
                msg: format!("unknown field \"{k}\""),
            });
        }
    }
    Ok(())
}

fn parse_provider(obj: &[(String, Value)], line_no: usize) -> Result<Card, StreamErr> {
    exact_fields(obj, PROVIDER_FIELDS, line_no)?;
    Ok(Card::Provider(ProviderCard {
        id: non_empty(obj, "id", line_no)?,
        lane_type: lane_type_field(obj, line_no)?,
        base_url: str_field(obj, "base_url", line_no)?,
        auth_path: str_field(obj, "auth_path", line_no)?,
        caps: u32_field(obj, "caps", line_no)?,
        source: non_empty(obj, "source", line_no)?,
    }))
}

fn parse_seat(obj: &[(String, Value)], line_no: usize) -> Result<Card, StreamErr> {
    exact_fields(obj, SEAT_FIELDS, line_no)?;
    let cost_word = str_field(obj, "cost_class", line_no)?;
    let state_word = str_field(obj, "state", line_no)?;
    Ok(Card::Seat(SeatCard {
        id: non_empty(obj, "id", line_no)?,
        provider: non_empty(obj, "provider", line_no)?,
        family: non_empty(obj, "family", line_no)?,
        model: non_empty(obj, "model", line_no)?,
        lane_type: lane_type_field(obj, line_no)?,
        cost_class: parse_cost_class(&cost_word).ok_or_else(|| StreamErr::Malformed {
            line: line_no,
            msg: format!("unknown cost_class {cost_word:?} (free|mid|premium)"),
        })?,
        state: parse_seat_state(&state_word).ok_or_else(|| StreamErr::Malformed {
            line: line_no,
            msg: format!(
                "unknown state {state_word:?} (live|expired|rate-limited|retired|probing|failed)"
            ),
        })?,
        since_epoch_s: u64_field(obj, "since_epoch_s", line_no)?,
        caps: u32_field(obj, "caps", line_no)?,
        cost_in_usd_per_mtok: num_field(obj, "cost_in_usd_per_mtok", line_no)?,
        cost_out_usd_per_mtok: num_field(obj, "cost_out_usd_per_mtok", line_no)?,
        context_window: u64_field(obj, "context_window", line_no)?,
        max_tokens: u64_field(obj, "max_tokens", line_no)?,
        source: non_empty(obj, "source", line_no)?,
    }))
}

// ---------------------------------------------------------------------------
// Fold + view
// ---------------------------------------------------------------------------

/// The registry state after folding the stream: last row per key wins.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Registry {
    /// Deterministic iteration: BTreeMap, key = (class, id).
    pub providers: BTreeMap<String, ProviderCard>,
    pub seats: BTreeMap<String, SeatCard>,
}

impl Registry {
    /// Fold parsed cards in order. Later rows supersede earlier rows with
    /// the same key (append-only edit model).
    pub fn fold(cards: &[Card]) -> Registry {
        let mut r = Registry::default();
        for c in cards {
            match c {
                Card::Provider(p) => {
                    r.providers.insert(p.id.clone(), p.clone());
                }
                Card::Seat(s) => {
                    r.seats.insert(s.id.clone(), s.clone());
                }
            }
        }
        r
    }

    /// Project every registered seat onto the substrate (selection order
    /// belongs to [`construct_panel`](crate::construct_panel), not here).
    pub fn seats(&self) -> Vec<crate::Seat> {
        self.seats.values().map(|s| s.to_seat()).collect()
    }

    /// The cached view JSON: stream digest + row count + the folded
    /// registry, byte-deterministic for the same stream.
    pub fn encode_view(&self, stream_sha256: &str, rows: usize) -> String {
        let mut o = String::new();
        o.push_str("{\"stream_sha256\":");
        json_str(stream_sha256, &mut o);
        o.push_str(",\"rows\":");
        o.push_str(&rows.to_string());
        o.push_str(",\"providers\":[");
        for (i, p) in self.providers.values().enumerate() {
            if i > 0 {
                o.push(',');
            }
            o.push_str(&encode_card(&Card::Provider(p.clone())));
        }
        o.push_str("],\"seats\":[");
        for (i, s) in self.seats.values().enumerate() {
            if i > 0 {
                o.push(',');
            }
            o.push_str(&encode_card(&Card::Seat(s.clone())));
        }
        o.push_str("]}");
        o
    }
}

/// Digest label for a stream's bytes (hex, full 64 chars — the view
/// verifies against exactly this).
///
/// DEFECT FIX (P4 slice 1, 2026-08-28): this WAS
/// `hex(&sha256(text))` — a DOUBLE hash, because [`sha256::hex`] is the
/// complete one-shot helper, not a bytes→hex encoder. The F2 law says the
/// view carries THE sha256 of the stream bytes; every external verifier
/// (python tooling, the P4 world bridge, the seed verify-gate) computing
/// plain sha256 over the stream must land on exactly this value. Pinned
/// against an outside-toolchain vector in registry_tests.
pub fn stream_digest(text: &str) -> String {
    sha256::hex(text.as_bytes())
}

/// Registry I/O errors (load/append/resync).
#[derive(Debug)]
pub enum IoErr {
    Stream(StreamErr),
    Fs(PathBuf, std::io::Error),
}

impl fmt::Display for IoErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IoErr::Stream(e) => write!(f, "{e}"),
            IoErr::Fs(p, e) => write!(f, "{}: {e}", p.display()),
        }
    }
}

impl std::error::Error for IoErr {}

impl From<StreamErr> for IoErr {
    fn from(e: StreamErr) -> Self {
        IoErr::Stream(e)
    }
}

fn fs_err(p: &Path, e: std::io::Error) -> IoErr {
    IoErr::Fs(p.to_path_buf(), e)
}

/// Load the stream, fold it, and (re-)sync the cached view file.
///
/// The view is re-derived and rewritten whenever it is missing, its
/// `stream_sha256` differs from the stream's actual digest, or its parse
/// fails — the cache is PROVEN against the truth, never trusted (F2).
/// Returns the folded registry plus whether the view was rewritten.
pub fn load_and_sync(stream_path: &Path, view_path: &Path) -> Result<(Registry, bool), IoErr> {
    let text = fs::read_to_string(stream_path).map_err(|e| fs_err(stream_path, e))?;
    let cards = parse_stream(&text)?;
    let reg = Registry::fold(&cards);
    let digest = stream_digest(&text);
    let want = reg.encode_view(&digest, cards.len());

    let stale = match fs::read_to_string(view_path) {
        Ok(have) => {
            // A view that fails to parse, or disagrees on the digest, is
            // stale. Byte comparison alone would rewrite on cosmetic
            // drift; digest comparison is the honest cache contract.
            match json::parse(&have) {
                Ok(v) => v.get("stream_sha256").and_then(|d| d.as_str()) != Some(digest.as_str()),
                Err(_) => true,
            }
        }
        Err(_) => true,
    };
    if stale {
        atomic_write(view_path, &want)?;
        return Ok((reg, true));
    }
    Ok((reg, false))
}

/// Append one card to the stream (single `write_all`, router ledger law —
/// appends are one syscall-sized line, never a rewrite), then re-sync the
/// view from the FULL stream (F2: re-synced on each row).
pub fn append_card(
    stream_path: &Path,
    view_path: &Path,
    card: &Card,
) -> Result<(Registry, bool), IoErr> {
    {
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(stream_path)
            .map_err(|e| fs_err(stream_path, e))?;
        let mut line = encode_card(card);
        line.push('\n');
        f.write_all(line.as_bytes())
            .map_err(|e| fs_err(stream_path, e))?;
        f.sync_data().map_err(|e| fs_err(stream_path, e))?;
    }
    load_and_sync(stream_path, view_path)
}

/// Atomic file replace: write a sibling temp file, then rename over the
/// destination (view writes only; the stream is append-only and never
/// passes through here).
fn atomic_write(path: &Path, contents: &str) -> Result<(), IoErr> {
    let mut tmp: PathBuf = path.to_path_buf();
    let mut name = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "view".into());
    name.push_str(".tmp");
    tmp.set_file_name(name);
    {
        let mut f = fs::File::create(&tmp).map_err(|e| fs_err(&tmp, e))?;
        f.write_all(contents.as_bytes())
            .map_err(|e| fs_err(&tmp, e))?;
        f.sync_all().map_err(|e| fs_err(&tmp, e))?;
    }
    fs::rename(&tmp, path).map_err(|e| fs_err(path, e))?;
    Ok(())
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
