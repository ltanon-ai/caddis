//! edits.rs — P1 slice 3: the registry EDIT PATH — warden-gated
//! propose→operator-confirm (F2: "edits via warden-gated
//! propose→operator-confirm"; plan P1 slice 3 "(warden gate-card)").
//! registry.rs shipped the stream mechanics and said superseding rows
//! "arrive through the warden-gated propose→operator-confirm path (P1
//! slice 3)" — this module IS that path.
//!
//! Laws transcribed:
//! - **Proposals are DURABLE** (MV13): `edits.jsonl` beside the stream is
//!   an append-only journal of flat exact-field rows; the fold is
//!   last-row-per-proposal_id. A pending proposal survives crashes and
//!   outlives the session that proposed it.
//! - **Optimistic concurrency** (router author law): `prior16` is
//!   sha256(stream)[0..16] captured at propose time; confirm compares it
//!   against the CURRENT stream. A mismatch is a STALE refusal — the edit
//!   is re-proposed against fresh state, never blind-written.
//! - **No-op refusal** (router law: a ruling is a change): a card
//!   fold-identical to the current row for its key is refused at propose
//!   AND re-checked at confirm.
//! - **THE WARDEN GATE** (F1's edit-side law): confirm requires an ACTIVE
//!   warden card for the CONFIRMING actor, derived READ-ONLY from the
//!   warden ledger through [`caddis_warden::card_state::active_for`] —
//!   the quorum-ruled "card state lives in the ledger and nowhere else"
//!   (CARD-0110), caller match EXACT (CARD-0109). No card = refusal,
//!   nothing written; unreadable ledger rows = Defect, because an answer
//!   that cannot be proven must not look like "no".
//! - **Identity from TRANSPORT records only** (F2/brief): `actor` and
//!   `actor_kind` arrive from the calling transport (`terminal` today;
//!   the additive vocabulary grows with P4 world surfaces, router
//!   quorum-fold-5 precedent). The organ never invents identity and
//!   never sniffs the environment.
//! - **Crash order: STREAM FIRST, JOURNAL LAST.** [`registry::append_card`]
//!   lands the card (the truth), then the confirm row is journaled. A
//!   crash between the two leaves an ORPHAN PENDING: the stream moved,
//!   the proposal stays pending forever, and any later confirm is
//!   refused STALE — detectable in [`status`], never double-applied. A
//!   confirm row without its card cannot exist (the row is appended
//!   last).
//! - **Deterministic bytes** (registry idempotency law): no timestamps,
//!   no clocks, no secrets in rows. MV11's `operator_confirmed_at` is
//!   the WARDEN ledger's fact to write, not this journal's.
//! - **R6 append law** (router journal precedent): O_EXCL lock →
//!   seq = max+1 over PARSED rows (never the line count — a hand-forked
//!   journal must not re-fork the next row) → ONE `write_all` ≤
//!   [`ROW_CAP`] → `sync_data`. The lock is held across the WHOLE
//!   confirm critical section (fold checks → stream append → journal
//!   row), so two racing confirms cannot both pass the pending check.
//!   The seed collector and TTL sweeps append through their own paths
//!   without this lock — their movement is caught by the confirm-time
//!   stale check, and the recovery is an honest re-propose.

use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use caddis_warden::card_state;

use crate::json::{self, Value};
use crate::registry::{self, Card, ProviderCard, Registry, SeatCard};

/// Same row-size law as the router journal: one append is ONE `write_all`
/// syscall — above this the call loops and tearing returns.
const ROW_CAP: usize = 4096;

/// How long a writer waits on a busy journal lock before failing.
const LOCK_WAIT: Duration = Duration::from_millis(750);

/// Wire words for journal row kinds.
pub const KIND_PROPOSE: &str = "propose";
pub const KIND_CONFIRM: &str = "confirm";
pub const KIND_REFUSE: &str = "refuse";

// ---------------------------------------------------------------------------
// Edit operations
// ---------------------------------------------------------------------------

/// One registry edit intent. Retirement is a STATE CHANGE through the
/// same upsert path (F10 vocabulary) — there is no tombstone class, the
/// fold keeps the row and selection filters non-Live seats.
#[derive(Debug, Clone, PartialEq)]
pub enum EditOp {
    UpsertSeat(SeatCard),
    UpsertProvider(ProviderCard),
}

impl EditOp {
    /// The wire word for this operation class.
    pub fn op_word(&self) -> &'static str {
        match self {
            Self::UpsertSeat(_) => "upsert-seat",
            Self::UpsertProvider(_) => "upsert-provider",
        }
    }

    /// The card this operation would append.
    pub fn to_card(&self) -> Card {
        match self {
            Self::UpsertSeat(s) => Card::Seat(s.clone()),
            Self::UpsertProvider(p) => Card::Provider(p.clone()),
        }
    }

    fn from_parts(op_word: &str, card: Card) -> Result<Self, String> {
        match (op_word, card) {
            ("upsert-seat", Card::Seat(s)) => Ok(Self::UpsertSeat(s)),
            ("upsert-provider", Card::Provider(p)) => Ok(Self::UpsertProvider(p)),
            (w, c) => Err(format!(
                "op '{w}' does not match card class '{}'",
                c.key().0
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Honest failure taxonomy (router AuthorErr law): `is_refusal()` = exit 1
/// — nothing was written, the proposal was stale/no-op/unknown/resolved or
/// the gate was closed; everything else is a Defect (exit 2) — malformed
/// journal, malformed stream, environment.
#[derive(Debug, Clone, PartialEq)]
pub enum EditErr {
    Stale {
        proposal_id: String,
        expected_prior16: String,
        actual_prior16: String,
    },
    Noop {
        key: String,
    },
    UnknownProposal {
        proposal_id: String,
    },
    NotPending {
        proposal_id: String,
        state: ProposalState,
    },
    GateClosed {
        actor: String,
    },
    Defect(String),
}

impl EditErr {
    /// Exit-1 class: a refusal, not a defect. Nothing was written.
    pub fn is_refusal(&self) -> bool {
        matches!(
            self,
            Self::Stale { .. }
                | Self::Noop { .. }
                | Self::UnknownProposal { .. }
                | Self::NotPending { .. }
                | Self::GateClosed { .. }
        )
    }
}

impl fmt::Display for EditErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stale { proposal_id, expected_prior16, actual_prior16 } => write!(
                f,
                "stale proposal {proposal_id}: pinned prior16 {expected_prior16} but stream is {actual_prior16} — re-propose against fresh state"
            ),
            Self::Noop { key } => {
                write!(f, "no-op refused for {key}: the fold already equals this card (a ruling is a change)")
            }
            Self::UnknownProposal { proposal_id } => {
                write!(f, "unknown proposal {proposal_id}")
            }
            Self::NotPending { proposal_id, state } => {
                write!(f, "proposal {proposal_id} is already {state:?}, not pending")
            }
            Self::GateClosed { actor } => write!(
                f,
                "warden gate closed: no active card in the ledger for '{actor}' — open a card before confirming"
            ),
            Self::Defect(msg) => write!(f, "defect: {msg}"),
        }
    }
}

impl std::error::Error for EditErr {}

impl From<registry::IoErr> for EditErr {
    fn from(e: registry::IoErr) -> Self {
        Self::Defect(format!("registry io: {e}"))
    }
}

// ---------------------------------------------------------------------------
// Journal rows
// ---------------------------------------------------------------------------

/// One parsed `edits.jsonl` row. `line` = 1-based file line.
#[derive(Debug, Clone, PartialEq)]
pub struct JournalRow {
    pub line: usize,
    pub seq: u64,
    pub kind: RowKind,
    pub proposal_id: String,
    pub actor: String,
    pub actor_kind: String,
    /// Propose rows only: the edit intent + the pinned prior.
    pub propose: Option<ProposeBody>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    Propose,
    Confirm,
    Refuse,
}

/// What a propose row carries beyond the shared fields.
#[derive(Debug, Clone, PartialEq)]
pub struct ProposeBody {
    pub op: EditOp,
    pub prior16: String,
}

/// The whole journal, honestly: parsed rows AND the lines that would not
/// parse (the read side never hides a defect — router Journal law).
#[derive(Debug, Default, PartialEq)]
pub struct Journal {
    pub rows: Vec<JournalRow>,
    pub unparseable: Vec<usize>,
}

/// Read the journal. An absent file is an EMPTY journal (the router
/// precedent); unparseable lines are counted, never dropped silently.
pub fn journal_load(path: &Path) -> Journal {
    let text = fs::read_to_string(path).unwrap_or_default();
    parse_journal(&text)
}

/// Parse journal text into rows + unparseable line numbers.
pub fn parse_journal(text: &str) -> Journal {
    let mut j = Journal::default();
    for (i, raw) in text.lines().enumerate() {
        let line_no = i + 1;
        let t = raw.trim();
        if t.is_empty() {
            continue;
        }
        match parse_journal_line(t, line_no) {
            Ok(row) => j.rows.push(row),
            Err(_) => j.unparseable.push(line_no),
        }
    }
    j
}

const PROPOSE_FIELDS: &[&str] = &[
    "seq",
    "kind",
    "proposal_id",
    "op",
    "prior16",
    "actor",
    "actor_kind",
    "card",
];
const RESOLVE_FIELDS: &[&str] = &["seq", "kind", "proposal_id", "actor", "actor_kind"];

fn parse_journal_line(t: &str, line_no: usize) -> Result<JournalRow, String> {
    let v = json::parse(t).map_err(|e| format!("line {line_no}: not JSON: {e:?}"))?;
    let obj = match &v {
        Value::Obj(pairs) => pairs,
        _ => return Err(format!("line {line_no}: not a flat object")),
    };
    let kind_word = str_field(obj, "kind", line_no)?;
    let (kind, fields) = match kind_word.as_str() {
        KIND_PROPOSE => (RowKind::Propose, PROPOSE_FIELDS),
        KIND_CONFIRM => (RowKind::Confirm, RESOLVE_FIELDS),
        KIND_REFUSE => (RowKind::Refuse, RESOLVE_FIELDS),
        other => return Err(format!("line {line_no}: unknown kind '{other}'")),
    };
    exact_fields(obj, fields, line_no)?;
    let seq = u64_field(obj, "seq", line_no)?;
    let proposal_id = str_field(obj, "proposal_id", line_no)?;
    if proposal_id.is_empty() {
        return Err(format!("line {line_no}: empty proposal_id"));
    }
    let actor = str_field(obj, "actor", line_no)?;
    if actor.is_empty() {
        return Err(format!(
            "line {line_no}: empty actor (identity is transport-served)"
        ));
    }
    let actor_kind = str_field(obj, "actor_kind", line_no)?;
    if actor_kind.is_empty() {
        return Err(format!("line {line_no}: empty actor_kind"));
    }
    let propose = if kind == RowKind::Propose {
        let op_word = str_field(obj, "op", line_no)?;
        let prior16 = str_field(obj, "prior16", line_no)?;
        if !is_hex16(&prior16) {
            return Err(format!(
                "line {line_no}: prior16 is not 16 lowercase hex chars"
            ));
        }
        // A propose row's id is DERIVED from its own seq — one id space, no
        // second authority to drift.
        if proposal_id != format!("e{seq}") {
            return Err(format!(
                "line {line_no}: propose row id {proposal_id} != e{seq}"
            ));
        }
        let card_text = str_field(obj, "card", line_no)?;
        let mut cards = registry::parse_stream(&card_text)
            .map_err(|e| format!("line {line_no}: embedded card: {e}"))?;
        if cards.len() != 1 {
            return Err(format!(
                "line {line_no}: embedded card text holds {} cards, want 1",
                cards.len()
            ));
        }
        let card = cards.pop().expect("len checked");
        let op = EditOp::from_parts(&op_word, card).map_err(|e| format!("line {line_no}: {e}"))?;
        Some(ProposeBody { op, prior16 })
    } else {
        None
    };
    Ok(JournalRow {
        line: line_no,
        seq,
        kind,
        proposal_id,
        actor,
        actor_kind,
        propose,
    })
}

fn str_field(obj: &[(String, Value)], key: &str, line_no: usize) -> Result<String, String> {
    match obj.iter().find(|(k, _)| k == key) {
        Some((_, Value::Str(s))) => Ok(s.clone()),
        Some(_) => Err(format!("line {line_no}: field '{key}' not a string")),
        None => Err(format!("line {line_no}: missing field '{key}'")),
    }
}

fn u64_field(obj: &[(String, Value)], key: &str, line_no: usize) -> Result<u64, String> {
    match obj.iter().find(|(k, _)| k == key) {
        Some((_, Value::Num(n))) if *n >= 1.0 && n.fract() == 0.0 && *n <= 9.0e15 => Ok(*n as u64),
        Some(_) => Err(format!(
            "line {line_no}: field '{key}' not a positive integer"
        )),
        None => Err(format!("line {line_no}: missing field '{key}'")),
    }
}

/// Exact field-set law: same count, same names, no duplicates, no unknowns
/// (registry grammar precedent — a typo must never silently drop a row).
fn exact_fields(obj: &[(String, Value)], want: &[&str], line_no: usize) -> Result<(), String> {
    if obj.len() != want.len() {
        return Err(format!(
            "line {line_no}: expected {} fields, found {}",
            want.len(),
            obj.len()
        ));
    }
    let mut have: Vec<&str> = obj.iter().map(|(k, _)| k.as_str()).collect();
    have.sort_unstable();
    let mut w: Vec<&str> = want.to_vec();
    w.sort_unstable();
    if have != w {
        return Err(format!("line {line_no}: field set mismatch"));
    }
    Ok(())
}

fn is_hex16(s: &str) -> bool {
    s.len() == 16 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

// --- encode ------------------------------------------------------------------

fn json_str(s: &str, out: &mut String) {
    out.push_str(&json::to_string(&Value::Str(s.to_string())));
}

/// The propose row: flat, fixed field order, the card embedded as a JSON
/// string holding the VERBATIM [`registry::encode_card`] line (parsed back
/// by the one card parser — parse law in exactly one place).
fn propose_line(seq: u64, op: &EditOp, prior16: &str, actor: &str, actor_kind: &str) -> String {
    let id = format!("e{seq}");
    let mut o = String::new();
    o.push_str("{\"seq\":");
    o.push_str(&seq.to_string());
    o.push_str(",\"kind\":\"");
    o.push_str(KIND_PROPOSE);
    o.push_str("\",\"proposal_id\":");
    json_str(&id, &mut o);
    o.push_str(",\"op\":");
    json_str(op.op_word(), &mut o);
    o.push_str(",\"prior16\":");
    json_str(prior16, &mut o);
    o.push_str(",\"actor\":");
    json_str(actor, &mut o);
    o.push_str(",\"actor_kind\":");
    json_str(actor_kind, &mut o);
    o.push_str(",\"card\":");
    json_str(&registry::encode_card(&op.to_card()), &mut o);
    o.push_str("}\n");
    o
}

/// A confirm or refuse row.
fn resolve_line(
    seq: u64,
    kind_word: &str,
    proposal_id: &str,
    actor: &str,
    actor_kind: &str,
) -> String {
    let mut o = String::new();
    o.push_str("{\"seq\":");
    o.push_str(&seq.to_string());
    o.push_str(",\"kind\":\"");
    o.push_str(kind_word);
    o.push_str("\",\"proposal_id\":");
    json_str(proposal_id, &mut o);
    o.push_str(",\"actor\":");
    json_str(actor, &mut o);
    o.push_str(",\"actor_kind\":");
    json_str(actor_kind, &mut o);
    o.push_str("}\n");
    o
}

// ---------------------------------------------------------------------------
// Fold
// ---------------------------------------------------------------------------

/// Lifecycle of one proposal, folded from the journal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProposalState {
    Pending,
    Confirmed,
    Refused,
}

/// One proposal, folded: the propose row plus how (whether) it resolved.
#[derive(Debug, Clone, PartialEq)]
pub struct Proposal {
    pub id: String,
    pub seq: u64,
    pub op: EditOp,
    pub prior16: String,
    /// The transport identity that PROPOSED (F2: transport records only).
    pub actor: String,
    pub actor_kind: String,
    pub state: ProposalState,
    /// The transport identity that confirmed or refused, when resolved.
    pub resolved_by: Option<String>,
}

/// Fold journal rows into proposals (last resolution wins; a propose row
/// re-using a live id is a Defect — ids derive from unique seqs).
pub fn fold_journal(j: &Journal) -> Result<BTreeMap<String, Proposal>, EditErr> {
    let mut m: BTreeMap<String, Proposal> = BTreeMap::new();
    for r in &j.rows {
        match r.kind {
            RowKind::Propose => {
                let body = r.propose.as_ref().ok_or_else(|| {
                    EditErr::Defect(format!(
                        "line {}: propose row without a body (parser defect)",
                        r.line
                    ))
                })?;
                if m.contains_key(&r.proposal_id) {
                    return Err(EditErr::Defect(format!(
                        "line {}: proposal id {} proposed twice",
                        r.line, r.proposal_id
                    )));
                }
                m.insert(
                    r.proposal_id.clone(),
                    Proposal {
                        id: r.proposal_id.clone(),
                        seq: r.seq,
                        op: body.op.clone(),
                        prior16: body.prior16.clone(),
                        actor: r.actor.clone(),
                        actor_kind: r.actor_kind.clone(),
                        state: ProposalState::Pending,
                        resolved_by: None,
                    },
                );
            }
            RowKind::Confirm | RowKind::Refuse => {
                let word = match r.kind {
                    RowKind::Confirm => KIND_CONFIRM,
                    _ => KIND_REFUSE,
                };
                let p = m.get_mut(&r.proposal_id).ok_or_else(|| {
                    EditErr::Defect(format!(
                        "line {}: {word} references unknown proposal {}",
                        r.line, r.proposal_id
                    ))
                })?;
                if p.state != ProposalState::Pending {
                    return Err(EditErr::Defect(format!(
                        "line {}: proposal {} is already {:?} — double {word}",
                        r.line, r.proposal_id, p.state
                    )));
                }
                p.state = if r.kind == RowKind::Confirm {
                    ProposalState::Confirmed
                } else {
                    ProposalState::Refused
                };
                p.resolved_by = Some(r.actor.clone());
            }
        }
    }
    Ok(m)
}

// ---------------------------------------------------------------------------
// The lock (R6 append law)
// ---------------------------------------------------------------------------

struct LockGuard(PathBuf);

impl LockGuard {
    fn acquire(journal: &Path) -> Result<Self, EditErr> {
        let lock = journal.with_extension("lock");
        let deadline = Instant::now() + LOCK_WAIT;
        loop {
            match OpenOptions::new().write(true).create_new(true).open(&lock) {
                Ok(_) => return Ok(LockGuard(lock)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    if Instant::now() >= deadline {
                        return Err(EditErr::Defect(format!(
                            "journal lock busy at {}",
                            lock.display()
                        )));
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(e) => {
                    return Err(EditErr::Defect(format!(
                        "cannot create journal lock {}: {e}",
                        lock.display()
                    )))
                }
            }
        }
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Append one row under the R6 law. CALLER HOLDS THE LOCK — this function
/// is the second half of every critical section below, never standalone.
fn append_line_locked(journal: &Path, make_line: &dyn Fn(u64) -> String) -> Result<u64, EditErr> {
    if let Some(parent) = journal.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| {
                EditErr::Defect(format!(
                    "cannot create journal dir {}: {e}",
                    parent.display()
                ))
            })?;
        }
    }
    let loaded = journal_load(journal);
    // seq = max+1 over PARSED rows (model-voice lesson: never the line count).
    let seq = loaded.rows.iter().map(|r| r.seq).max().unwrap_or(0) + 1;
    let line = make_line(seq);
    if line.len() > ROW_CAP {
        return Err(EditErr::Defect(format!(
            "journal row would exceed the single-write cap ({}) — shorten actor",
            line.len()
        )));
    }
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(journal)
        .map_err(|e| EditErr::Defect(format!("cannot open journal {}: {e}", journal.display())))?;
    f.write_all(line.as_bytes()).map_err(|e| {
        EditErr::Defect(format!("cannot append journal {}: {e}", journal.display()))
    })?;
    f.sync_data()
        .map_err(|e| EditErr::Defect(format!("cannot sync journal {}: {e}", journal.display())))?;
    Ok(seq)
}

// ---------------------------------------------------------------------------
// Shared checks
// ---------------------------------------------------------------------------

/// The journal must be fully parseable and fold-consistent before any
/// write — a corrupted journal refuses new work (fail-closed).
fn ensure_clean(j: &Journal) -> Result<(), EditErr> {
    if !j.unparseable.is_empty() {
        return Err(EditErr::Defect(format!(
            "journal holds unparseable lines {:?} — repair before editing",
            j.unparseable
        )));
    }
    fold_journal(j)?;
    Ok(())
}

/// Read the stream; an absent or malformed stream is a Defect (the
/// registry must be seeded before it can be edited).
fn read_stream(path: &Path) -> Result<(String, Vec<Card>), EditErr> {
    let text = fs::read_to_string(path).map_err(|_| {
        EditErr::Defect(format!(
            "registry stream absent/unreadable at {} — seed it before editing",
            path.display()
        ))
    })?;
    let cards = registry::parse_stream(&text)
        .map_err(|e| EditErr::Defect(format!("registry stream malformed: {e}")))?;
    Ok((text, cards))
}

fn digest16(text: &str) -> String {
    registry::stream_digest(text)[..16].to_string()
}

/// The fold already equals this card for its key → a no-op (a ruling is a
/// change).
fn fold_is_noop(cards: &[Card], card: &Card) -> bool {
    let r = Registry::fold(cards);
    match card {
        Card::Provider(p) => r.providers.get(&p.id) == Some(p),
        Card::Seat(s) => r.seats.get(&s.id) == Some(s),
    }
}

/// Self-validation (router author law): what this module would WRITE must
/// LOAD — the encoded card re-parses to the identical card or the edit
/// never exists.
fn self_validate(card: &Card) -> Result<(), EditErr> {
    let line = registry::encode_card(card);
    let back = registry::parse_stream(&line)
        .map_err(|e| EditErr::Defect(format!("self-validation parse: {e}")))?;
    if back.as_slice() != [card.clone()] {
        return Err(EditErr::Defect(
            "self-validation: encoded card does not round-trip".into(),
        ));
    }
    Ok(())
}

fn key_str(card: &Card) -> String {
    let (class, id) = card.key();
    format!("{class}/{id}")
}

// ---------------------------------------------------------------------------
// The three verbs
// ---------------------------------------------------------------------------

/// PROPOSE an edit: validate, pin the current stream state, journal a
/// durable pending row (MV13). The stream is NOT touched — proposing is
/// read-only against the truth.
pub fn propose(
    stream_path: &Path,
    journal_path: &Path,
    op: EditOp,
    actor: &str,
    actor_kind: &str,
) -> Result<String, EditErr> {
    if actor.is_empty() || actor_kind.is_empty() {
        return Err(EditErr::Defect(
            "actor and actor_kind are transport-served and must be non-empty".into(),
        ));
    }
    let card = op.to_card();
    self_validate(&card)?;
    let _guard = LockGuard::acquire(journal_path)?;
    ensure_clean(&journal_load(journal_path))?;
    let (text, cards) = read_stream(stream_path)?;
    if fold_is_noop(&cards, &card) {
        return Err(EditErr::Noop {
            key: key_str(&card),
        });
    }
    let prior16 = digest16(&text);
    let seq = append_line_locked(journal_path, &|seq| {
        propose_line(seq, &op, &prior16, actor, actor_kind)
    })?;
    Ok(format!("e{seq}"))
}

/// What a successful confirm learned.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfirmOutcome {
    pub proposal_id: String,
    pub confirm_seq: u64,
    /// `class/id` of the card that landed in the stream.
    pub applied_key: String,
    /// The warden card id the ledger attested for the confirming actor.
    pub warden_card: String,
}

/// OPERATOR-CONFIRM a pending proposal. Order of law: journal consistency
/// → pending state → stale prior → no-op re-check → THE WARDEN GATE →
/// stream append → journal row (crash order: STREAM FIRST, JOURNAL LAST).
/// The whole critical section holds the journal lock, so racing confirms
/// cannot both pass the pending check.
pub fn confirm(
    stream_path: &Path,
    view_path: &Path,
    journal_path: &Path,
    proposal_id: &str,
    actor: &str,
    actor_kind: &str,
    warden_ledger_text: &str,
) -> Result<ConfirmOutcome, EditErr> {
    if actor.is_empty() || actor_kind.is_empty() {
        return Err(EditErr::Defect(
            "actor and actor_kind are transport-served and must be non-empty".into(),
        ));
    }
    let _guard = LockGuard::acquire(journal_path)?;
    let journal = journal_load(journal_path);
    ensure_clean(&journal)?;
    let proposals = fold_journal(&journal)?;
    let p = proposals.get(proposal_id).ok_or(EditErr::UnknownProposal {
        proposal_id: proposal_id.to_string(),
    })?;
    if p.state != ProposalState::Pending {
        return Err(EditErr::NotPending {
            proposal_id: proposal_id.to_string(),
            state: p.state,
        });
    }
    let (text, cards) = read_stream(stream_path)?;
    let actual16 = digest16(&text);
    if actual16 != p.prior16 {
        return Err(EditErr::Stale {
            proposal_id: proposal_id.to_string(),
            expected_prior16: p.prior16.clone(),
            actual_prior16: actual16,
        });
    }
    let card = p.op.to_card();
    if fold_is_noop(&cards, &card) {
        return Err(EditErr::Noop {
            key: key_str(&card),
        });
    }
    // THE WARDEN GATE — read-only derivation from the ledger the caller
    // supplies. unreadable > 0 means the ledger cannot attest either way:
    // fail-closed as a Defect, never a silent "closed".
    let cs = card_state::active_for(warden_ledger_text, actor);
    if cs.unreadable > 0 {
        return Err(EditErr::Defect(format!(
            "warden ledger holds {} unreadable rows — cannot attest a gate card for '{actor}'",
            cs.unreadable
        )));
    }
    let active = cs.active.ok_or(EditErr::GateClosed {
        actor: actor.to_string(),
    })?;
    // Crash order: the card lands FIRST (truth), the confirm row LAST.
    registry::append_card(stream_path, view_path, &card)?;
    let applied_key = key_str(&card);
    let warden_card = active.id;
    let confirm_seq = append_line_locked(journal_path, &|seq| {
        resolve_line(seq, KIND_CONFIRM, proposal_id, actor, actor_kind)
    })?;
    Ok(ConfirmOutcome {
        proposal_id: proposal_id.to_string(),
        confirm_seq,
        applied_key,
        warden_card,
    })
}

/// REFUSE a pending proposal: the operator's explicit NO, journaled so
/// the pending queue stays honest (MV13 durable — resolved, not dropped).
pub fn refuse(
    journal_path: &Path,
    proposal_id: &str,
    actor: &str,
    actor_kind: &str,
) -> Result<u64, EditErr> {
    if actor.is_empty() || actor_kind.is_empty() {
        return Err(EditErr::Defect(
            "actor and actor_kind are transport-served and must be non-empty".into(),
        ));
    }
    let _guard = LockGuard::acquire(journal_path)?;
    let journal = journal_load(journal_path);
    ensure_clean(&journal)?;
    let proposals = fold_journal(&journal)?;
    match proposals.get(proposal_id) {
        None => Err(EditErr::UnknownProposal {
            proposal_id: proposal_id.to_string(),
        }),
        Some(p) if p.state != ProposalState::Pending => Err(EditErr::NotPending {
            proposal_id: proposal_id.to_string(),
            state: p.state,
        }),
        Some(_) => append_line_locked(journal_path, &|seq| {
            resolve_line(seq, KIND_REFUSE, proposal_id, actor, actor_kind)
        }),
    }
}

// ---------------------------------------------------------------------------
// Status
// ---------------------------------------------------------------------------

/// The honest journal census — pending proposals (with their intent),
/// resolution counts, unparseable line numbers, the max parsed seq.
/// Read-only; never fails; orphan-pending detection (a confirmed card
/// whose stream moved is visible as a STALE refusal on retry, the
/// crash-order law above).
#[derive(Debug, Default, PartialEq)]
pub struct EditsStatus {
    pub pending: Vec<Proposal>,
    pub confirmed: usize,
    pub refused: usize,
    pub unparseable: Vec<usize>,
    pub max_seq: u64,
}

pub fn status(journal_path: &Path) -> EditsStatus {
    let j = journal_load(journal_path);
    let mut st = EditsStatus {
        unparseable: j.unparseable.clone(),
        max_seq: j.rows.iter().map(|r| r.seq).max().unwrap_or(0),
        ..EditsStatus::default()
    };
    if let Ok(proposals) = fold_journal(&j) {
        for p in proposals.values() {
            match p.state {
                ProposalState::Pending => st.pending.push(p.clone()),
                ProposalState::Confirmed => st.confirmed += 1,
                ProposalState::Refused => st.refused += 1,
            }
        }
    }
    st
}

#[cfg(test)]
#[path = "edits_tests.rs"]
mod tests;
