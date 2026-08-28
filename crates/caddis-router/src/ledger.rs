//! Decision + outcome ledger (P2 slice 1, R6): the append-only JSONL stream a
//! task card references via `route_id` (F3/R10 — decision rows, not cards).
//!
//! Three row kinds share one stream:
//! - **decision** — what [`crate::route`] chose and why-cheaply (the P1
//!   [`RouteDecision`] value, persisted);
//! - **outcome** — the brief's telemetry row
//!   `{card_id, task_class, lane, model, cost_tokens, cost_usd_est,
//!   latency_ms, verify_outcome, escalated_to}` — the A6 collector's shape,
//!   born retroactive. EWMA capability ([`crate::stats`]) folds THESE only;
//! - **promotion** (P4/R2) — the transient->persistent transition marker the
//!   [`crate::alerts`] scan appends when a lane's trailing RED-TEST fails
//!   reach hysteresis (demotion) or a pass clears them again (healed). Not
//!   capability evidence: the fold ignores it, exactly like decisions.
//!
//! QQ1a is a TYPE law here: `verify_outcome` is pass|fail from the task
//! card's own RED-TEST (R3: deterministic checks only). A warden policy-deny
//! is NOT an outcome — it never decays a lane — so it simply has no way to
//! enter the stream. R5 (warden-signed identity) lands in P4; `model` is the
//! transport-record identity as plain data until then.
//!
//! Append law (R6): lock (O_EXCL + token, fail-closed) -> read max seq ->
//! seq = max+1 (model-voice lesson: NEVER the line count — a forked or
//! hand-edited file must not re-fork) -> ONE `write_all` under ROW_CAP (a
//! single syscall; tearing cannot split it) -> `sync_data` BEFORE the lock
//! releases. A crash mid-append leaves at most one torn trailing line, which
//! [`load`] reports honestly and [`crate::verify::verify_path`] shows forever.

use std::collections::BTreeMap;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::lane::LaneTier;
use crate::lock::{Lock, LockErr};
use crate::route::RouteDecision;

/// How long a writer waits for the ledger lock before failing closed.
pub const LOCK_WAIT: Duration = Duration::from_secs(2);

/// One row-size the single-`write_all` atomicity claim rests on (same law as
/// caddis-core: above this `write_all` LOOPS, and a loop is where tearing
/// returns).
const ROW_CAP: usize = 4096;
/// Escaped-byte cap per string field. Twelve fields at 256 plus the JSON
/// skeleton keep every row under ROW_CAP BY CONSTRUCTION.
pub(crate) const FIELD_CAP: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The task card's RED-TEST passed after this lane's work.
    Pass,
    /// RED-TEST red (R3) — the only signal that decays a lane (QQ1a).
    Fail,
}

impl Outcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Outcome::Pass => "pass",
            Outcome::Fail => "fail",
        }
    }
    fn parse(s: &str) -> Option<Self> {
        match s {
            "pass" => Some(Outcome::Pass),
            "fail" => Some(Outcome::Fail),
            _ => None,
        }
    }
}

/// What [`crate::route`] chose — persisted as a `decision` row.
#[derive(Debug, Clone, PartialEq)]
pub struct DecisionRow {
    pub route_id: String,
    pub card_id: String,
    pub task_class: String,
    pub lane_id: String,
    pub lane_tier: LaneTier,
    pub cost_per_task_usd: f64,
    /// R4: chosen below floor because no measured lane reached it.
    pub degraded: bool,
}

impl From<&RouteDecision> for DecisionRow {
    fn from(d: &RouteDecision) -> Self {
        DecisionRow {
            route_id: d.route_id.clone(),
            card_id: d.card_id.clone(),
            task_class: d.task_class.clone(),
            lane_id: d.lane_id.clone(),
            lane_tier: d.lane_tier,
            cost_per_task_usd: d.cost_per_task_usd,
            degraded: d.degraded,
        }
    }
}

/// The telemetry row the A6 collector emits — quality folds THESE only.
#[derive(Debug, Clone, PartialEq)]
pub struct OutcomeRow {
    pub card_id: String,
    pub task_class: String,
    pub lane_id: String,
    /// Model identity FROM THE TRANSPORT RECORD (lesson-bank law); R5 signs
    /// it in P4.
    pub model: String,
    pub cost_tokens: u64,
    pub cost_usd_est: f64,
    pub latency_ms: u64,
    pub outcome: Outcome,
    /// Escalation hop (O2): the lane the task was re-routed to after a fail.
    pub escalated_to: Option<String>,
}

/// R2 transition marker (P4): a lane's decay became PERSISTENT (demoted) or
/// cleared (healed). Appended only by the [`crate::alerts`] scan, which
/// derives transitions from outcome rows — never by dispatch adapters.
#[derive(Debug, Clone, PartialEq)]
pub struct PromotionRow {
    pub lane_id: String,
    pub task_class: String,
    /// true = demoted to persistent decay; false = healed (one pass, QQ2).
    pub demoted: bool,
    /// Trailing fails AT the transition (0 for healed).
    pub trailing_fails: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Row {
    Decision(DecisionRow),
    Outcome(OutcomeRow),
    Promotion(PromotionRow),
}

#[derive(Debug)]
pub enum LedgerErr {
    Io(std::io::Error),
    /// R6: the lock stayed held for the whole budget — append refused
    /// (concurrent appends are forbidden by construction). Retry.
    LockBusy,
    /// A row failed encode-time validation (e.g. non-finite cost — JSON has
    /// no NaN, and writing one would corrupt the stream for every reader).
    BadRow(&'static str),
}

// io::Error is not PartialEq: compare by KIND (same law as lock::LockErr).
impl PartialEq for LedgerErr {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (LedgerErr::Io(a), LedgerErr::Io(b)) => a.kind() == b.kind(),
            (LedgerErr::LockBusy, LedgerErr::LockBusy) => true,
            (LedgerErr::BadRow(a), LedgerErr::BadRow(b)) => a == b,
            _ => false,
        }
    }
}
impl From<std::io::Error> for LedgerErr {
    fn from(e: std::io::Error) -> Self {
        LedgerErr::Io(e)
    }
}
impl From<LockErr> for LedgerErr {
    fn from(e: LockErr) -> Self {
        match e {
            LockErr::Busy => LedgerErr::LockBusy,
            LockErr::Io(e) => LedgerErr::Io(e),
        }
    }
}

impl std::fmt::Display for LedgerErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LedgerErr::Io(e) => write!(f, "ledger io: {e}"),
            LedgerErr::LockBusy => write!(f, "ledger lock busy (R6 fail-closed)"),
            LedgerErr::BadRow(why) => write!(f, "bad row: {why}"),
        }
    }
}

/// One parsed row with its coordinates: `line` in the file (1-based) and the
/// row's own `seq`. Stats fold by seq; findings point at lines.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedRow {
    pub line: u64,
    pub seq: u64,
    pub row: Row,
}

/// The whole file, honestly: parsed rows AND the lines that would not parse.
#[derive(Debug, Default, PartialEq)]
pub struct Loaded {
    pub rows: Vec<ParsedRow>,
    /// (line number, reason) for every line that failed to parse. Kept, not
    /// skipped silently — verify reports them and append never renumbers
    /// past them (seq comes from parsed max, not the line count).
    pub bad: Vec<(u64, String)>,
}

impl Loaded {
    pub fn max_seq(&self) -> u64 {
        self.rows.iter().map(|r| r.seq).max().unwrap_or(0)
    }
}

pub struct Ledger {
    path: PathBuf,
}

impl Ledger {
    pub fn new<P: AsRef<Path>>(path: P) -> Self {
        Ledger {
            path: path.as_ref().to_path_buf(),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Append one row (clock: real time). See [`Ledger::append_ts`] for the
    /// deterministic-entry point tests use.
    pub fn append(&self, row: &Row) -> Result<u64, LedgerErr> {
        self.append_ts(row, &now_iso(), LOCK_WAIT)
    }

    pub(crate) fn append_ts(&self, row: &Row, ts: &str, wait: Duration) -> Result<u64, LedgerErr> {
        // The first organ write materializes its own state home — a
        // missing parent dir is birth, not corruption (os error 3).
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let _guard = Lock::acquire(&self.path, wait)?;
        // Read under the lock: max seq over PARSED rows (a hand-forked or
        // torn file must not re-fork the next append — model-voice seq lesson).
        let loaded = self.load_unlocked()?;
        let seq = loaded.max_seq() + 1;
        let line = encode(seq, ts, row)?;
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        f.write_all(line.as_bytes())?;
        // R6: durability before the lock releases — a row that can vanish
        // after a crash is a row the decision trail never had.
        f.sync_data()?;
        Ok(seq)
    }

    /// Parse the whole file. Lock-free: readers never exclude writers (an
    /// append is one atomic syscall-sized write; a reader sees either the
    /// pre- or post-write line, never half of one).
    pub fn load(&self) -> Result<Loaded, LedgerErr> {
        self.load_unlocked()
    }

    fn load_unlocked(&self) -> Result<Loaded, LedgerErr> {
        let bytes = match fs::read(&self.path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Loaded::default()),
            Err(e) => return Err(LedgerErr::Io(e)),
        };
        let text = String::from_utf8_lossy(&bytes);
        Ok(parse_stream(&text))
    }
}

/// Parse a whole ledger text into rows + honest bad-line list.
pub fn parse_stream(text: &str) -> Loaded {
    let mut loaded = Loaded::default();
    for (idx, line) in text.split('\n').enumerate() {
        let line_no = (idx + 1) as u64;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match parse_line(trimmed) {
            Ok((seq, row)) => loaded.rows.push(ParsedRow {
                line: line_no,
                seq,
                row,
            }),
            Err(why) => loaded.bad.push((line_no, why)),
        }
    }
    loaded
}

// ---------------------------------------------------------------------------
// Encoding
// ---------------------------------------------------------------------------

/// CARD-WARDEN-1 escaping law (verbatim semantics, caddis-core): the two
/// structural characters, the five short forms, and every remaining C0 as
/// `\u00xx`. A raw newline inside a JSONL record ENDS the record — this is
/// what keeps one append reading back as one line.
pub(crate) fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Elide `s` so its ESCAPED form fits FIELD_CAP, saying so with a trailing
/// `...` when it does. Budgeted on the escaped form — `esc` can turn one
/// byte into six, so a raw-byte budget is not a row-byte budget.
pub(crate) fn fit(s: &str) -> String {
    if esc(s).len() <= FIELD_CAP {
        return s.to_string();
    }
    let mut cut = String::new();
    for c in s.chars() {
        if esc(&cut).len() + esc(&c.to_string()).len() + 3 > FIELD_CAP {
            break;
        }
        cut.push(c);
    }
    cut.push_str("...");
    cut
}

fn num(v: f64) -> String {
    // Rust's Display for f64 is shortest-roundtrip; JSON readers parse it back.
    format!("{v}")
}

/// Encode one JSONL row. Every value is an explicit token: [`Tok::Text`]
/// (escaped, quoted — free text and ONLY free text) or [`Tok::Raw`] (numbers,
/// bools, null, enum words). Quoting is decided HERE, structurally, never by
/// sniffing the value string — a lane genuinely named `null` must encode as
/// the STRING "null", not the JSON null.
fn encode(seq: u64, ts: &str, row: &Row) -> Result<String, LedgerErr> {
    let mut f: Vec<(&str, Tok)> = vec![
        ("seq", Tok::Raw(seq.to_string())),
        ("ts", Tok::Text(fit(ts))),
        ("kind", Tok::Raw(String::new())), // set below
    ];
    match row {
        Row::Decision(d) => {
            if !d.cost_per_task_usd.is_finite() {
                return Err(LedgerErr::BadRow("decision cost_per_task_usd not finite"));
            }
            f[2].1 = Tok::Text("decision".into());
            f.extend([
                ("route_id", Tok::Text(fit(&d.route_id))),
                ("card_id", Tok::Text(fit(&d.card_id))),
                ("task_class", Tok::Text(fit(&d.task_class))),
                ("lane_id", Tok::Text(fit(&d.lane_id))),
                ("tier", Tok::Text(d.lane_tier.as_str().into())),
                ("cost_per_task_usd", Tok::Raw(num(d.cost_per_task_usd))),
                ("degraded", Tok::Raw(d.degraded.to_string())),
            ]);
        }
        Row::Outcome(o) => {
            if !o.cost_usd_est.is_finite() {
                return Err(LedgerErr::BadRow("outcome cost_usd_est not finite"));
            }
            f[2].1 = Tok::Text("outcome".into());
            f.extend([
                ("card_id", Tok::Text(fit(&o.card_id))),
                ("task_class", Tok::Text(fit(&o.task_class))),
                ("lane_id", Tok::Text(fit(&o.lane_id))),
                ("model", Tok::Text(fit(&o.model))),
                ("cost_tokens", Tok::Raw(o.cost_tokens.to_string())),
                ("cost_usd_est", Tok::Raw(num(o.cost_usd_est))),
                ("latency_ms", Tok::Raw(o.latency_ms.to_string())),
                ("verify_outcome", Tok::Text(o.outcome.as_str().into())),
                (
                    "escalated_to",
                    match &o.escalated_to {
                        Some(lane) => Tok::Text(fit(lane)),
                        None => Tok::Raw("null".into()),
                    },
                ),
            ]);
        }
        Row::Promotion(p) => {
            f[2].1 = Tok::Text("promotion".into());
            f.extend([
                ("lane_id", Tok::Text(fit(&p.lane_id))),
                ("task_class", Tok::Text(fit(&p.task_class))),
                ("demoted", Tok::Raw(p.demoted.to_string())),
                ("trailing_fails", Tok::Raw(p.trailing_fails.to_string())),
            ]);
        }
    }
    let body: String = f
        .iter()
        .map(|(k, v)| {
            let val = match v {
                Tok::Text(s) => format!("\"{}\"", esc(s)),
                Tok::Raw(s) => s.clone(),
            };
            format!("\"{}\":{}", esc(k), val)
        })
        .collect::<Vec<_>>()
        .join(",");
    let line = format!("{{{}}}\n", body);
    debug_assert!(line.len() <= ROW_CAP, "row exceeds ROW_CAP by construction");
    Ok(line)
}

enum Tok {
    Text(String),
    Raw(String),
}

// ---------------------------------------------------------------------------
// Parsing — a flat-object JSON subset (no nesting: rows are flat by design,
// so the parser rejects nesting instead of half-supporting it)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Val {
    Str(String),
    Num(f64),
    Bool(bool),
    Null,
}

pub(crate) fn parse_line(line: &str) -> Result<(u64, Row), String> {
    let map = parse_object(line)?;
    decode(&map)
}

pub(crate) fn parse_object(line: &str) -> Result<BTreeMap<String, Val>, String> {
    let b: Vec<char> = line.chars().collect();
    let mut i = 0usize;
    let n = b.len();
    let skip_ws = |i: &mut usize| {
        while *i < n && b[*i].is_whitespace() {
            *i += 1;
        }
    };
    skip_ws(&mut i);
    if i >= n || b[i] != '{' {
        return Err("expected '{'".into());
    }
    i += 1;
    let mut map = BTreeMap::new();
    skip_ws(&mut i);
    if i < n && b[i] == '}' {
        return Ok(map);
    }
    loop {
        skip_ws(&mut i);
        let key = parse_string(&b, &mut i)?;
        skip_ws(&mut i);
        if i >= n || b[i] != ':' {
            return Err(format!("expected ':' after key '{key}'"));
        }
        i += 1;
        skip_ws(&mut i);
        let val = parse_value(&b, &mut i)?;
        map.insert(key, val);
        skip_ws(&mut i);
        if i >= n {
            return Err("unterminated object".into());
        }
        match b[i] {
            ',' => {
                i += 1;
            }
            '}' => {
                i += 1;
                skip_ws(&mut i);
                if i != n {
                    return Err("trailing content after '}'".into());
                }
                return Ok(map);
            }
            c => return Err(format!("expected ',' or '}}', got '{c}'")),
        }
    }
}

fn parse_string(b: &[char], i: &mut usize) -> Result<String, String> {
    if *i >= b.len() || b[*i] != '"' {
        return Err("expected string".into());
    }
    *i += 1;
    let mut out = String::new();
    // Each arm leaves `i` on the NEXT unconsumed char — there is no shared
    // trailing advance (that was the v0 bug: escape arms advanced twice and
    // silently swallowed the char after every escape).
    while *i < b.len() {
        match b[*i] {
            '"' => {
                *i += 1;
                return Ok(out);
            }
            '\\' => {
                *i += 1; // at the escape char
                if *i >= b.len() {
                    return Err("unterminated escape".into());
                }
                match b[*i] {
                    '"' => {
                        out.push('"');
                        *i += 1;
                    }
                    '\\' => {
                        out.push('\\');
                        *i += 1;
                    }
                    '/' => {
                        out.push('/');
                        *i += 1;
                    }
                    'b' => {
                        out.push('\u{0008}');
                        *i += 1;
                    }
                    'f' => {
                        out.push('\u{000c}');
                        *i += 1;
                    }
                    'n' => {
                        out.push('\n');
                        *i += 1;
                    }
                    'r' => {
                        out.push('\r');
                        *i += 1;
                    }
                    't' => {
                        out.push('\t');
                        *i += 1;
                    }
                    'u' => {
                        *i += 1; // at first hex digit
                        let cp = hex4(b, i)?; // leaves i one PAST the 4th digit
                                              // Surrogate pair combining (foreign files may carry
                                              // them; our writer never emits one).
                        if (0xD800..=0xDBFF).contains(&cp) {
                            if *i + 1 < b.len() && b[*i] == '\\' && b[*i + 1] == 'u' {
                                *i += 2; // at the low surrogate's first hex digit
                                let low = hex4(b, i)?;
                                if !(0xDC00..=0xDFFF).contains(&low) {
                                    return Err("unpaired surrogate".into());
                                }
                                let c = 0x10000 + ((cp - 0xD800) << 10) + (low - 0xDC00);
                                out.push(char::from_u32(c).ok_or("bad codepoint")?);
                            } else {
                                return Err("unpaired surrogate".into());
                            }
                        } else if (0xDC00..=0xDFFF).contains(&cp) {
                            return Err("unpaired low surrogate".into());
                        } else {
                            out.push(char::from_u32(cp).ok_or("bad codepoint")?);
                        }
                    }
                    c => return Err(format!("bad escape '\\{c}'")),
                }
            }
            c => {
                if (c as u32) < 0x20 {
                    return Err("raw control char in string".into());
                }
                out.push(c);
                *i += 1;
            }
        }
    }
    Err("unterminated string".into())
}

/// Four hex digits starting AT `*i`; leaves `*i` one past the last digit.
fn hex4(b: &[char], i: &mut usize) -> Result<u32, String> {
    let mut v: u32 = 0;
    for _ in 0..4 {
        if *i >= b.len() {
            return Err("short \\u escape".into());
        }
        let d = b[*i].to_digit(16).ok_or("bad \\u escape")?;
        v = v * 16 + d;
        *i += 1;
    }
    Ok(v)
}

fn parse_value(b: &[char], i: &mut usize) -> Result<Val, String> {
    match b.get(*i) {
        Some('"') => Ok(Val::Str(parse_string(b, i)?)),
        Some('t') => expect_word(b, i, "true").map(|_| Val::Bool(true)),
        Some('f') => expect_word(b, i, "false").map(|_| Val::Bool(false)),
        Some('n') => expect_word(b, i, "null").map(|_| Val::Null),
        Some(c) if *c == '-' || c.is_ascii_digit() => parse_number(b, i),
        Some(c) => Err(format!("unexpected '{c}'")),
        None => Err("value expected".into()),
    }
}

fn expect_word(b: &[char], i: &mut usize, word: &str) -> Result<(), String> {
    let w: Vec<char> = word.chars().collect();
    if *i + w.len() <= b.len() && b[*i..*i + w.len()] == w[..] {
        *i += w.len();
        Ok(())
    } else {
        Err(format!("expected '{word}'"))
    }
}

fn parse_number(b: &[char], i: &mut usize) -> Result<Val, String> {
    let start = *i;
    if b[*i] == '-' {
        *i += 1;
    }
    while *i < b.len() && b[*i].is_ascii_digit() {
        *i += 1;
    }
    if *i < b.len() && b[*i] == '.' {
        *i += 1;
        while *i < b.len() && b[*i].is_ascii_digit() {
            *i += 1;
        }
    }
    if *i < b.len() && (b[*i] == 'e' || b[*i] == 'E') {
        *i += 1;
        if *i < b.len() && (b[*i] == '+' || b[*i] == '-') {
            *i += 1;
        }
        while *i < b.len() && b[*i].is_ascii_digit() {
            *i += 1;
        }
    }
    let s: String = b[start..*i].iter().collect();
    if s.is_empty() || s == "-" {
        return Err("bad number".into());
    }
    s.parse::<f64>()
        .map(Val::Num)
        .map_err(|_| "bad number".into())
}

// --- decode: map -> typed row ----------------------------------------------

pub(crate) fn get<'a>(m: &'a BTreeMap<String, Val>, k: &str) -> Result<&'a Val, String> {
    m.get(k).ok_or_else(|| format!("missing field '{k}'"))
}
pub(crate) fn as_str<'a>(v: &'a Val, k: &str) -> Result<&'a str, String> {
    match v {
        Val::Str(s) => Ok(s),
        _ => Err(format!("field '{k}' not a string")),
    }
}
pub(crate) fn as_u64(v: &Val, k: &str) -> Result<u64, String> {
    match v {
        Val::Num(n) if n.fract() == 0.0 && *n >= 0.0 && *n <= u64::MAX as f64 => Ok(*n as u64),
        _ => Err(format!("field '{k}' not a non-negative integer")),
    }
}
fn as_f64(v: &Val, k: &str) -> Result<f64, String> {
    match v {
        Val::Num(n) if n.is_finite() => Ok(*n),
        _ => Err(format!("field '{k}' not a finite number")),
    }
}
fn as_bool(v: &Val, k: &str) -> Result<bool, String> {
    match v {
        Val::Bool(b) => Ok(*b),
        _ => Err(format!("field '{k}' not a bool")),
    }
}

pub(crate) fn decode(m: &BTreeMap<String, Val>) -> Result<(u64, Row), String> {
    let seq = as_u64(get(m, "seq")?, "seq")?;
    if seq == 0 {
        return Err("seq must start at 1".into());
    }
    match as_str(get(m, "kind")?, "kind")? {
        "decision" => {
            let tier = LaneTier::parse(as_str(get(m, "tier")?, "tier")?)
                .ok_or("unknown tier (O2: 'droid' is unparseable)")?;
            Ok((
                seq,
                Row::Decision(DecisionRow {
                    route_id: as_str(get(m, "route_id")?, "route_id")?.to_string(),
                    card_id: as_str(get(m, "card_id")?, "card_id")?.to_string(),
                    task_class: as_str(get(m, "task_class")?, "task_class")?.to_string(),
                    lane_id: as_str(get(m, "lane_id")?, "lane_id")?.to_string(),
                    lane_tier: tier,
                    cost_per_task_usd: as_f64(get(m, "cost_per_task_usd")?, "cost_per_task_usd")?,
                    degraded: as_bool(get(m, "degraded")?, "degraded")?,
                }),
            ))
        }
        "outcome" => {
            let outcome = Outcome::parse(as_str(get(m, "verify_outcome")?, "verify_outcome")?)
                .ok_or("verify_outcome must be pass|fail")?;
            let escalated_to = match get(m, "escalated_to")? {
                Val::Null => None,
                Val::Str(s) => Some(s.clone()),
                _ => return Err("field 'escalated_to' not string|null".into()),
            };
            Ok((
                seq,
                Row::Outcome(OutcomeRow {
                    card_id: as_str(get(m, "card_id")?, "card_id")?.to_string(),
                    task_class: as_str(get(m, "task_class")?, "task_class")?.to_string(),
                    lane_id: as_str(get(m, "lane_id")?, "lane_id")?.to_string(),
                    model: as_str(get(m, "model")?, "model")?.to_string(),
                    cost_tokens: as_u64(get(m, "cost_tokens")?, "cost_tokens")?,
                    cost_usd_est: as_f64(get(m, "cost_usd_est")?, "cost_usd_est")?,
                    latency_ms: as_u64(get(m, "latency_ms")?, "latency_ms")?,
                    outcome,
                    escalated_to,
                }),
            ))
        }
        "promotion" => Ok((
            seq,
            Row::Promotion(PromotionRow {
                lane_id: as_str(get(m, "lane_id")?, "lane_id")?.to_string(),
                task_class: as_str(get(m, "task_class")?, "task_class")?.to_string(),
                demoted: as_bool(get(m, "demoted")?, "demoted")?,
                trailing_fails: as_u64(get(m, "trailing_fails")?, "trailing_fails")? as u32,
            }),
        )),
        other => Err(format!("unknown kind '{other}'")),
    }
}

// ---------------------------------------------------------------------------
// Clock (ISO-8601 UTC, no dependency: civil-from-days, Howard Hinnant)
// ---------------------------------------------------------------------------

pub fn now_iso() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    iso_from_unix(secs)
}

pub fn iso_from_unix(secs: u64) -> String {
    let days = (secs / 86400) as i64;
    let rem = secs % 86400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y,
        m,
        d,
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
#[path = "ledger_tests.rs"]
mod tests;
