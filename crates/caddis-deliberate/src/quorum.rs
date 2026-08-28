//! quorum.rs — P2 slice 2: the QUORUM protocol card v1 (plan P2, R3).
//!
//! Laws transcribed:
//! - **The card is DATA** (plan P2): [`protocol_v1`] is the versioned
//!   quorum card — the seven canonical stages `convene → pool → dispatch
//!   → collect → integrate → verdict → ledger`. Stage two seats the
//!   POOL, not a panel: quorum seats are PEERS ([`crate::disjoint`]
//!   law); roles are the council's vocabulary and would lie here.
//! - **F9 reuse, never a second law**: pool selection IS
//!   [`crate::disjoint::select_quorum_pool`] — the STRICT disjoint
//!   filter against the council panel, live-only, free-first, floors
//!   validated on the selected pool, zero-overlap re-proven before
//!   return. This module adds no selection logic of its own.
//! - **The ladder link** (charter: quorum comes AFTER the council
//!   integrated): [`convene`] takes the council SESSION, inherits its
//!   task — the SAME staged questions — verbatim, selects the pool
//!   disjoint from THAT council's panel, and records the council
//!   convening id on the session. The re-dispatch path re-proves the
//!   council identity (a quorum re-run under a different council is a
//!   Defect).
//! - **F1 reuse**: the gate is [`crate::council::gate`] — the ONE
//!   read-only `active_for` law (unreadable rows = Defect, no active
//!   card = GateClosed). This module only translates the error language.
//! - **F3 reuse, never a second pin**: the pin is [`Protocol::pin`] —
//!   sha256 over canonical bytes, implemented exactly once. Quorum seats
//!   are peers, so the session is not a [`crate::protocol::Convening`]
//!   (that type carries a role-assigned Panel); the pin travels on
//!   [`QuorumSession::pinned_protocol`] and [`check_pin`] reports through
//!   the same [`PinMismatch`] evidence the substrate already defines.
//! - **Floor 2/3 as the MAJORITY law**: [`decision_floor`] =
//!   `pool_size / 2 + 1` — a strict majority of the FULL pool; the v1
//!   pool of three yields 2. Derived, never a magic constant: an
//!   operator re-floor to a five-seat pool (new card version + quorum
//!   sign-off) yields floor 3 with zero code change.
//! - **Asterisk under degradation**: a missing seat is TOLERATED only
//!   while the floor still holds — the verdict lands with
//!   `degraded = true`, the ruling text carries a literal `*`, and the
//!   missing lanes are listed in the VERDICT.md artifact and the ledger
//!   row. Degradation is never hidden; it is also never invented.
//! - **Fail-closed on missing floor**: fewer answers than the floor =
//!   [`QuorumErr::CollectIncomplete`] refusal; a split with no position
//!   at the floor = [`QuorumErr::FloorUnmet`] refusal. No quorum verdict
//!   lands below the floor, ever.
//! - **VERDICT.md artifact**: [`verdict_md`] renders the deterministic
//!   markdown the estate's quorum verdicts already speak — verdict line,
//!   floor fraction, council link, per-seat table with TRANSPORT-served
//!   models, missing list. No timestamps (MV11: the warden ledger owns
//!   times).
//! - **One digest law**: the verdict digest is
//!   [`crate::council::canonical_verdict_bytes`] — the same canonical
//!   framing for both bodies. There is deliberately no second digest
//!   format.
//! - **Ledger row**: ONE flat exact-field JSON line + ONE parser (parse
//!   law in one place), registry grammar as the council row, plus the
//!   quorum evidence: ruled position, floor fraction, missing count,
//!   degraded flag — with the majority law enforced on READ (`floor`
//!   numerator must equal the majority of its denominator).
//! - **F11**: [`pause_and_re_dispatch`] — the same choreography as the
//!   council card: both cards validate, strict version bump, pin must
//!   differ, gate re-runs, pool re-selects under the NEW floors against
//!   the SAME council panel, rerun id `{original}#r{version}`, archived
//!   original returned immutable.
//!
//! P3 (the executor) persists and dispatches under these gates; nothing
//! here performs I/O.

use std::fmt;

use crate::council::{
    self, CouncilErr, CouncilSession, DisagreementMap, GateReceipt, PinOutcome, Position, Reply,
    Stakes,
};
use crate::disjoint::{select_quorum_pool, DisjointErr, QuorumPool};
use crate::protocol::{DispatchEntry, PinMismatch, Protocol, ProtocolKind, ProvenanceRow, Verdict};
use crate::registry::Registry;
use crate::{caps, sha256, Floors, Seat};

/// The seven canonical quorum stages, in execution order. Stage two is
/// `pool` — the peers-not-roles law ([`crate::disjoint`]); a quorum card
/// without exactly these stages, in this order, does not validate.
pub const QUORUM_STAGES: &[&str] = &[
    "convene",
    "pool",
    "dispatch",
    "collect",
    "integrate",
    "verdict",
    "ledger",
];

/// The v1 quorum pool size (brief prior: 3 seats from the disjoint pool).
pub const QUORUM_POOL_SIZE: usize = 3;

/// The quorum protocol card v1: kind Quorum, version 1, the canonical
/// seven stages, floors `panel_size = 3` (the POOL size — the field names
/// the body size for both cards), families 2 / non-Chinese 1 (the same
/// diversity priors the council card carries). Floor changes are operator
/// rulings and land as NEW versions through quorum sign-off, never as
/// edits to v1.
pub fn protocol_v1() -> Protocol {
    Protocol {
        version: 1,
        kind: ProtocolKind::Quorum,
        stages: QUORUM_STAGES.iter().map(|s| s.to_string()).collect(),
        floors: Floors {
            panel_size: QUORUM_POOL_SIZE,
            min_families: 2,
            min_non_chinese: 1,
        },
    }
}

/// The decision floor: a STRICT MAJORITY of the full pool
/// (`pool_size / 2 + 1`). For the v1 pool of three that is 2 — the
/// "floor 2/3" ruling as arithmetic, not a constant. Derived on purpose:
/// re-flooring the pool through a new card version moves the floor with
/// it and no branch anywhere else needs editing.
pub fn decision_floor(pool_size: usize) -> usize {
    pool_size / 2 + 1
}

/// Mechanical card validation (plan P2 Done-When: "both protocol cards
/// validate mechanically"). Structural, not policy: kind must be Quorum,
/// stages exactly [`QUORUM_STAGES`], version >= 1, floors coherent
/// (positive, minima that fit inside the pool), pool >= 2 — a one-seat
/// pool cannot carry a majority floor (a solo "quorum" is an oxymoron).
pub fn validate_card(p: &Protocol) -> Result<(), QuorumErr> {
    if p.kind != ProtocolKind::Quorum {
        return Err(QuorumErr::CardInvalid(format!(
            "kind is {} — this module only carries quorum cards",
            p.kind.as_str()
        )));
    }
    if p.version == 0 {
        return Err(QuorumErr::CardInvalid(
            "version 0 — cards start at 1".into(),
        ));
    }
    if p.stages.len() != QUORUM_STAGES.len()
        || p.stages
            .iter()
            .zip(QUORUM_STAGES)
            .any(|(have, want)| have != want)
    {
        return Err(QuorumErr::CardInvalid(format!(
            "stages must be exactly {:?} in order — have {:?}",
            QUORUM_STAGES, p.stages
        )));
    }
    let f = &p.floors;
    if f.panel_size == 0 || f.min_families == 0 || f.min_non_chinese == 0 {
        return Err(QuorumErr::CardInvalid(format!(
            "floors must be positive — panel_size={}, min_families={}, min_non_chinese={}",
            f.panel_size, f.min_families, f.min_non_chinese
        )));
    }
    if f.min_families > f.panel_size || f.min_non_chinese > f.panel_size {
        return Err(QuorumErr::CardInvalid(format!(
            "floor minima exceed the pool — panel_size={}, min_families={}, min_non_chinese={}",
            f.panel_size, f.min_families, f.min_non_chinese
        )));
    }
    if f.panel_size < 2 {
        return Err(QuorumErr::CardInvalid(format!(
            "pool of {} seats cannot carry a majority floor — two seats minimum",
            f.panel_size
        )));
    }
    Ok(())
}

/// One quorum convening: the peers pool (NO roles — the disjoint law),
/// the F3 pin, the ladder link to the council convening, the F1 receipt,
/// the carried stakes, and the F11 re-run flag (`Some(archived id)` =
/// this session re-dispatched after a mid-flight protocol edit).
#[derive(Debug, Clone, PartialEq)]
pub struct QuorumSession {
    pub id: String,
    /// The SAME task the linked council answered — copied verbatim at
    /// convene; the quorum votes the same staged questions, never a
    /// rewording.
    pub task: String,
    /// sha256 of the protocol AT CONVENING TIME (F3) —
    /// [`Protocol::pin`], the ONE hash law, stored never caller-supplied.
    pub pinned_protocol: String,
    pub pool: QuorumPool,
    /// The council convening this quorum deliberates after (ladder link).
    pub council_convening: String,
    pub stakes: Stakes,
    pub gate: GateReceipt,
    pub rerun_of: Option<String>,
    /// Per-answered-leg dispatch audit (P0 seam — the executor fills it;
    /// a convening is auditable from birth through execution, the
    /// council law).
    pub dispatch_log: Vec<DispatchEntry>,
}

/// Honest failure taxonomy (council CouncilErr law): `is_refusal()`
/// = exit 1 — nothing was written: the gate was closed, the card
/// invalid, the pool unsatisfiable, the answers below the floor, the
/// split without a ruling, the version not bumped. Everything else is a
/// Defect (exit 2) — malformed input, unreadable ledger, identity
/// crossings, a wrong council on re-dispatch.
#[derive(Debug, Clone, PartialEq)]
pub enum QuorumErr {
    /// The protocol card fails mechanical validation.
    CardInvalid(String),
    /// F1: no active warden card for the convening actor.
    GateClosed { actor: String },
    /// Pool selection refusal (F9 overlap, degraded day, floors).
    Pool(DisjointErr),
    /// Fewer answers than the decision floor — nothing can land.
    CollectIncomplete {
        missing: Vec<String>,
        have: usize,
        floor: usize,
    },
    /// The votes split with no position at the floor — fail-closed, no
    /// verdict lands as if decided.
    FloorUnmet { counts: String, floor: usize },
    /// F11: the re-dispatch card's version is not a strict bump.
    VersionNotBumped { have: u32, try_use: u32 },
    /// F11: the re-dispatch card hashes identical to the archived pin.
    SameCard { pin: String },
    /// Dispatch planning failure (pool/registry mismatch, zero caps).
    Dispatch(String),
    /// Defect: malformed input, unreadable ledger, identity crossing.
    Defect(String),
}

impl QuorumErr {
    /// Refusal (exit 1) vs Defect (exit 2) — the edits-law split.
    pub fn is_refusal(&self) -> bool {
        matches!(
            self,
            QuorumErr::CardInvalid(_)
                | QuorumErr::GateClosed { .. }
                | QuorumErr::Pool(_)
                | QuorumErr::CollectIncomplete { .. }
                | QuorumErr::FloorUnmet { .. }
                | QuorumErr::VersionNotBumped { .. }
                | QuorumErr::SameCard { .. }
        )
    }
}

impl fmt::Display for QuorumErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            QuorumErr::CardInvalid(why) => write!(f, "quorum card invalid: {why}"),
            QuorumErr::GateClosed { actor } => write!(
                f,
                "warden gate closed: no active card for '{actor}' — F1 refuses the convening"
            ),
            QuorumErr::Pool(e) => write!(f, "pool selection refused: {e}"),
            QuorumErr::CollectIncomplete {
                missing,
                have,
                floor,
            } => write!(
                f,
                "answers below the decision floor: {have} of the pool answered, floor is {floor} — missing: {}",
                missing.join(", ")
            ),
            QuorumErr::FloorUnmet { counts, floor } => write!(
                f,
                "votes split with no position at the floor {floor} — table {counts}; no verdict lands"
            ),
            QuorumErr::VersionNotBumped { have, try_use } => write!(
                f,
                "re-dispatch card version {try_use} is not a bump over {have} — F11 mandates a strict version bump"
            ),
            QuorumErr::SameCard { pin } => {
                write!(f, "re-dispatch card pins identical to the archived protocol ({pin}) — a re-dispatch is a change")
            }
            QuorumErr::Dispatch(why) => write!(f, "dispatch planning failed: {why}"),
            QuorumErr::Defect(why) => write!(f, "defect: {why}"),
        }
    }
}

impl std::error::Error for QuorumErr {}

impl From<DisjointErr> for QuorumErr {
    fn from(e: DisjointErr) -> Self {
        QuorumErr::Pool(e)
    }
}

/// The F1 gate — [`crate::council::gate`], the ONE law, wrapped only to
/// translate the error language into this module's taxonomy.
fn gate(warden_ledger_text: &str, actor: &str) -> Result<GateReceipt, QuorumErr> {
    council::gate(warden_ledger_text, actor).map_err(|e| match e {
        CouncilErr::GateClosed { actor } => QuorumErr::GateClosed { actor },
        other => QuorumErr::Defect(other.to_string()),
    })
}

/// STAGE `convene` (+ `pool`): validate the card, pass the F1 gate,
/// select the pool DISJOINT from the linked council's panel (F9 — the
/// one selection law, reused), and pin the protocol (F3 — the one hash
/// law, stored on the session).
///
/// Order of law: card → gate → pool → session. The task is the council's
/// task verbatim — the SAME staged questions (ladder link). Nothing is
/// written by this function; a [`QuorumSession`] is a value; the P3
/// executor persists and dispatches under these same gates.
pub fn convene(
    id: impl Into<String>,
    council_session: &CouncilSession,
    protocol: &Protocol,
    candidates: &[Seat],
    actor: &str,
    warden_ledger_text: &str,
) -> Result<QuorumSession, QuorumErr> {
    let id = id.into();
    if id.is_empty() || council_session.convening.task.is_empty() {
        return Err(QuorumErr::Defect(
            "convening id and the linked council task must be non-empty".into(),
        ));
    }
    validate_card(protocol)?;
    let gate_receipt = gate(warden_ledger_text, actor)?;
    let pool = select_quorum_pool(
        &council_session.convening.panel,
        candidates,
        protocol.floors.panel_size,
        &protocol.floors,
    )?;
    Ok(QuorumSession {
        pinned_protocol: protocol.pin(),
        id,
        task: council_session.convening.task.clone(),
        pool,
        council_convening: council_session.convening.id.clone(),
        stakes: council_session.stakes,
        gate: gate_receipt,
        rerun_of: None,
        dispatch_log: Vec::new(),
    })
}

/// STAGE `dispatch` (as DATA — the P3 executor runs it): the serialized
/// wave plan for the pool, in pool order, under the registry's caps law
/// (Ruling 7: a capped provider never shares a wave). Same law as the
/// council card — [`caps::plan_batches`] — over a different body.
pub fn dispatch_plan(
    session: &QuorumSession,
    reg: &Registry,
) -> Result<Vec<Vec<String>>, QuorumErr> {
    let wanted: Vec<&str> = session
        .pool
        .seats
        .iter()
        .map(|s| s.lane_id.as_str())
        .collect();
    caps::plan_batches(&wanted, reg).map_err(|e| QuorumErr::Dispatch(e.to_string()))
}

/// The collected votes: one reply per ANSWERING pool seat, in POOL order,
/// plus the pool seats that did not answer (sorted) — the degradation
/// evidence the verdict's asterisk and the ledger row carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Votes {
    pub replies: Vec<Reply>,
    pub missing: Vec<String>,
}

/// STAGE `collect`: fold raw replies into the votes. Fail-closed laws:
/// no duplicates (Defect), no replies from lanes outside the pool
/// (Defect — identity crossing), no empty transport-served model
/// (Defect — provenance has no blank form). MISSING seats are tolerated
/// — recorded honestly — while the floor remains reachable; fewer
/// answers than [`decision_floor`] is a [`QuorumErr::CollectIncomplete`]
/// refusal (fail-closed on the missing floor).
pub fn collect(session: &QuorumSession, replies: &[Reply]) -> Result<Votes, QuorumErr> {
    let mut by_lane: std::collections::BTreeMap<&str, &Reply> = std::collections::BTreeMap::new();
    for r in replies {
        if r.transport_served_model.is_empty() {
            return Err(QuorumErr::Defect(format!(
                "reply from '{}' carries an empty transport_served_model — provenance has no blank form",
                r.lane_id
            )));
        }
        if by_lane.insert(r.lane_id.as_str(), r).is_some() {
            return Err(QuorumErr::Defect(format!(
                "duplicate reply from '{}' — one reply per seat, exactly",
                r.lane_id
            )));
        }
    }
    let pool_lanes: std::collections::BTreeSet<&str> = session
        .pool
        .seats
        .iter()
        .map(|s| s.lane_id.as_str())
        .collect();
    for lane in by_lane.keys() {
        if !pool_lanes.contains(lane) {
            return Err(QuorumErr::Defect(format!(
                "reply from '{lane}' — that lane is not seated in this pool (identity crossing)"
            )));
        }
    }
    let mut missing: Vec<String> = pool_lanes
        .iter()
        .filter(|l| !by_lane.contains_key(**l))
        .map(|l| l.to_string())
        .collect();
    missing.sort();
    let floor = decision_floor(session.pool.seats.len());
    let answering = by_lane.len();
    if answering < floor {
        return Err(QuorumErr::CollectIncomplete {
            missing,
            have: answering,
            floor,
        });
    }
    // Pool order, deterministic.
    let ordered = session
        .pool
        .seats
        .iter()
        .filter_map(|s| by_lane.get(s.lane_id.as_str()).map(|r| (*r).clone()))
        .collect();
    Ok(Votes {
        replies: ordered,
        missing,
    })
}

/// STAGE `integrate`: cluster the votes — the council's ONE clustering
/// law ([`council::integrate`]), reused through an adapter, never a
/// second implementation. All three positions always present, fixed
/// cluster order, lane ids sorted.
pub fn integrate(votes: &Votes) -> DisagreementMap {
    council::integrate(&council::Bundled {
        replies: votes.replies.clone(),
    })
}

/// The ruling: the position that reached the decision floor, the lanes
/// holding it, the floor met, and the full pool size. DATA for the
/// VERDICT.md artifact and the ledger row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuorumRuling {
    pub position: Position,
    pub agreeing: Vec<String>,
    pub floor: usize,
    pub pool_size: usize,
}

/// STAGE `verdict`: the structured ruling. The position holding at least
/// [`decision_floor`] of the FULL pool wins; no such position =
/// [`QuorumErr::FloorUnmet`] refusal (fail-closed — a split never lands
/// as if decided). Two positions at the floor is arithmetically
/// impossible (two strict majorities would exceed the pool), so the
/// winner is unique by construction.
///
/// Degradation: any missing seat flips `degraded = true` AND appends a
/// literal `*` to the ruling text — the asterisk travels inside the
/// digest, the VERDICT.md artifact, and the ledger row. The council card
/// always rules `degraded = false` (its collect refuses partials); the
/// quorum's higher tolerance is bounded by exactly this asterisk.
pub fn verdict(
    session: &QuorumSession,
    votes: &Votes,
    map: &DisagreementMap,
) -> Result<(Verdict, QuorumRuling), QuorumErr> {
    let pool_size = session.pool.seats.len();
    let floor = decision_floor(pool_size);
    let winner = map
        .clusters
        .iter()
        .find(|c| c.lane_ids.len() >= floor)
        .ok_or_else(|| QuorumErr::FloorUnmet {
            counts: map.summary(),
            floor,
        })?;
    debug_assert!(
        map.clusters
            .iter()
            .filter(|c| c.lane_ids.len() >= floor)
            .count()
            <= 1,
        "two strict majorities cannot coexist inside one pool"
    );
    let degraded = !votes.missing.is_empty();
    let ruling = format!(
        "{}{}",
        winner.position.as_str(),
        if degraded { "*" } else { "" }
    );
    let verdict = Verdict {
        convening_id: session.id.clone(),
        ruling,
        provenance: votes
            .replies
            .iter()
            .map(|r| ProvenanceRow {
                lane_id: r.lane_id.clone(),
                lane_type: session
                    .pool
                    .seats
                    .iter()
                    .find(|s| s.lane_id == r.lane_id)
                    .map(|s| s.lane_type)
                    .expect("collect proved every reply lane is pooled"),
                transport_served_model: r.transport_served_model.clone(),
            })
            .collect(),
        degraded,
    };
    Ok((
        verdict,
        QuorumRuling {
            position: winner.position,
            agreeing: winner.lane_ids.clone(),
            floor,
            pool_size,
        },
    ))
}

/// STAGE `verdict` artifact: the VERDICT.md text the estate's quorum
/// verdicts already speak — deterministic, no timestamps (MV11). The
/// asterisk rides the verdict line; the missing list is explicit
/// (`none` when the full pool answered).
pub fn verdict_md(
    session: &QuorumSession,
    verdict: &Verdict,
    ruling: &QuorumRuling,
    map: &DisagreementMap,
    votes: &Votes,
) -> String {
    let mut md = String::with_capacity(320);
    md.push_str("# VERDICT — ");
    md.push_str(&session.id);
    md.push_str("\n\nverdict: ");
    md.push_str(&verdict.ruling);
    md.push_str(&format!(
        "\nfloor: {}/{}",
        ruling.agreeing.len(),
        ruling.pool_size
    ));
    md.push_str("\ncouncil: ");
    md.push_str(&session.council_convening);
    md.push_str("\ndegraded: ");
    md.push_str(if verdict.degraded { "true" } else { "false" });
    md.push_str("\ntable: ");
    md.push_str(&map.summary());
    md.push_str("\nseats:");
    for r in &votes.replies {
        let position = map
            .position_of(&r.lane_id)
            .map(|p| p.as_str())
            .unwrap_or("unmapped");
        md.push_str(&format!(
            "\n- {} -> {} (served by {})",
            r.lane_id, position, r.transport_served_model
        ));
    }
    md.push_str("\nmissing: ");
    if votes.missing.is_empty() {
        md.push_str("none");
    } else {
        md.push_str(&votes.missing.join(", "));
    }
    md.push('\n');
    md
}

/// The exact ledger-row field set (flat grammar, registry law).
const ROW_FIELDS: &[&str] = &[
    "conv",
    "kind",
    "pin",
    "council",
    "stakes",
    "rerun_of",
    "actor",
    "warden_card",
    "ship",
    "ship_with_changes",
    "do_not_ship",
    "missing",
    "ruled",
    "floor",
    "degraded",
    "verdict_digest",
];

/// One parsed quorum ledger row (STAGE `ledger`, read side).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuorumRow {
    pub conv: String,
    pub pin: String,
    pub council: String,
    pub stakes: String,
    pub rerun_of: String,
    pub actor: String,
    pub warden_card: String,
    pub ship: u64,
    pub ship_with_changes: u64,
    pub do_not_ship: u64,
    pub missing: u64,
    pub ruled: String,
    pub floor: String,
    pub degraded: bool,
    pub verdict_digest: String,
}

/// STAGE `ledger`: render the ruling as ONE flat exact-field JSON line.
/// Deterministic bytes — no timestamps (MV11), no secrets, fixed field
/// order. `verdict_digest` = sha256 over
/// [`council::canonical_verdict_bytes`] — the ONE digest law, shared
/// with the council row; the quorum row is tamper-evident against the
/// verdict it summarizes, asterisk included.
pub fn ledger_row(
    session: &QuorumSession,
    verdict: &Verdict,
    ruling: &QuorumRuling,
    map: &DisagreementMap,
    votes: &Votes,
) -> String {
    let digest = sha256::hex(council::canonical_verdict_bytes(verdict, map).as_bytes());
    let mut out = String::with_capacity(384);
    out.push('{');
    out.push_str("\"conv\":");
    json_str(&session.id, &mut out);
    out.push_str(",\"kind\":\"quorum\",\"pin\":");
    json_str(&session.pinned_protocol, &mut out);
    out.push_str(",\"council\":");
    json_str(&session.council_convening, &mut out);
    out.push_str(",\"stakes\":\"");
    out.push_str(session.stakes.as_str());
    out.push_str("\",\"rerun_of\":");
    json_str(session.rerun_of.as_deref().unwrap_or(""), &mut out);
    out.push_str(",\"actor\":");
    json_str(&session.gate.actor, &mut out);
    out.push_str(",\"warden_card\":");
    json_str(&session.gate.warden_card, &mut out);
    out.push_str(",\"ship\":");
    out.push_str(&map.holding(Position::Ship).len().to_string());
    out.push_str(",\"ship_with_changes\":");
    out.push_str(&map.holding(Position::ShipWithChanges).len().to_string());
    out.push_str(",\"do_not_ship\":");
    out.push_str(&map.holding(Position::DoNotShip).len().to_string());
    out.push_str(",\"missing\":");
    out.push_str(&votes.missing.len().to_string());
    out.push_str(",\"ruled\":\"");
    out.push_str(ruling.position.as_str());
    out.push_str("\",\"floor\":");
    json_str(
        &format!("{}/{}", ruling.agreeing.len(), ruling.pool_size),
        &mut out,
    );
    out.push_str(",\"degraded\":");
    out.push_str(if verdict.degraded {
        "\"true\""
    } else {
        "\"false\""
    });
    out.push_str(",\"verdict_digest\":");
    json_str(&digest, &mut out);
    out.push('}');
    out
}

/// JSON-string append (vendored json writer, registry precedent).
fn json_str(s: &str, out: &mut String) {
    out.push_str(&crate::json::to_string(&crate::json::Value::Str(
        s.to_string(),
    )));
}

/// Parse one quorum ledger row. Exact field law (same count, same names,
/// no duplicates, no unknowns), `kind` must be `quorum`, counts
/// non-negative integers, `floor` must be `N/M` with `N == M/2 + 1` (the
/// majority law enforced on READ), counts + missing must sum to the pool
/// `M`, the ruled position's own count must meet the floor, `degraded`
/// must be exactly the missing-ness, and `verdict_digest` must be 64
/// lowercase hex chars. THE parser — the encode side above is its only
/// producer.
pub fn parse_ledger_row(line: &str) -> Result<QuorumRow, QuorumErr> {
    let v = crate::json::parse(line)
        .map_err(|e| QuorumErr::Defect(format!("ledger row is not JSON: {e:?}")))?;
    let obj = v
        .as_obj()
        .ok_or_else(|| QuorumErr::Defect("ledger row must be a flat object".into()))?;
    let mut names: Vec<&str> = obj.iter().map(|(k, _)| k.as_str()).collect();
    names.sort_unstable();
    let mut want: Vec<&str> = ROW_FIELDS.to_vec();
    want.sort_unstable();
    if names != want {
        return Err(QuorumErr::Defect(format!(
            "ledger row fields must be exactly {want:?} — have {names:?}"
        )));
    }
    let get_str = |key: &str| -> Result<String, QuorumErr> {
        v.get(key)
            .and_then(|x| x.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| QuorumErr::Defect(format!("field '{key}' must be a string")))
    };
    let kind = get_str("kind")?;
    if kind != "quorum" {
        return Err(QuorumErr::Defect(format!(
            "kind must be 'quorum' — have '{kind}'"
        )));
    }
    let get_count = |key: &str| -> Result<u64, QuorumErr> {
        let n = v
            .get(key)
            .and_then(|x| x.as_f64())
            .ok_or_else(|| QuorumErr::Defect(format!("field '{key}' must be a number")))?;
        if n < 0.0 || n.fract() != 0.0 {
            return Err(QuorumErr::Defect(format!(
                "field '{key}' must be a non-negative integer"
            )));
        }
        Ok(n as u64)
    };
    let ship = get_count("ship")?;
    let ship_with_changes = get_count("ship_with_changes")?;
    let do_not_ship = get_count("do_not_ship")?;
    let missing = get_count("missing")?;
    let ruled = get_str("ruled")?;
    let ruled_count = match ruled.as_str() {
        "ship" => ship,
        "ship_with_changes" => ship_with_changes,
        "do_not_ship" => do_not_ship,
        other => {
            return Err(QuorumErr::Defect(format!(
                "ruled must be ship|ship_with_changes|do_not_ship — have '{other}'"
            )))
        }
    };
    let floor = get_str("floor")?;
    let (agreeing, pool_size) = parse_floor(&floor)?;
    if agreeing < 1 || pool_size < 2 {
        return Err(QuorumErr::Defect(format!(
            "floor '{floor}': the numerator must be >= 1 and the pool >= 2"
        )));
    }
    if agreeing < pool_size / 2 + 1 {
        return Err(QuorumErr::Defect(format!(
            "floor '{floor}': {agreeing} agreeing seats do not meet the strict majority of the pool ({})",
            pool_size / 2 + 1
        )));
    }
    if ship + ship_with_changes + do_not_ship + missing != pool_size {
        return Err(QuorumErr::Defect(format!(
            "counts sum to {} with missing {} — the pool is {pool_size}",
            ship + ship_with_changes + do_not_ship,
            missing
        )));
    }
    if ruled_count < agreeing {
        return Err(QuorumErr::Defect(format!(
            "ruled '{ruled}' holds {ruled_count} seats — below the floor {agreeing}"
        )));
    }
    let degraded_str = get_str("degraded")?;
    let degraded = match degraded_str.as_str() {
        "true" => true,
        "false" => false,
        other => {
            return Err(QuorumErr::Defect(format!(
                "degraded must be true|false — have '{other}'"
            )))
        }
    };
    if degraded != (missing > 0) {
        return Err(QuorumErr::Defect(format!(
            "degraded={degraded} but missing={missing} — degradation is exactly missing-ness"
        )));
    }
    let verdict_digest = get_str("verdict_digest")?;
    if verdict_digest.len() != 64
        || !verdict_digest
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    {
        return Err(QuorumErr::Defect(
            "verdict_digest must be 64 lowercase hex chars".into(),
        ));
    }
    Ok(QuorumRow {
        conv: get_str("conv")?,
        pin: get_str("pin")?,
        council: get_str("council")?,
        stakes: get_str("stakes")?,
        rerun_of: get_str("rerun_of")?,
        actor: get_str("actor")?,
        warden_card: get_str("warden_card")?,
        ship,
        ship_with_changes,
        do_not_ship,
        missing,
        ruled,
        floor,
        degraded,
        verdict_digest,
    })
}

/// `N/M` floor fraction — both parts numeric, no sign, no whitespace.
fn parse_floor(floor: &str) -> Result<(u64, u64), QuorumErr> {
    let (n, m) = floor
        .split_once('/')
        .ok_or_else(|| QuorumErr::Defect(format!("floor '{floor}' must be N/M — majority/pool")))?;
    let parse = |part: &str, name: &str| -> Result<u64, QuorumErr> {
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return Err(QuorumErr::Defect(format!(
                "floor '{floor}': the {name} must be a non-negative integer"
            )));
        }
        part.parse::<u64>()
            .map_err(|_| QuorumErr::Defect(format!("floor '{floor}': {name} overflows")))
    };
    Ok((parse(n, "numerator")?, parse(m, "denominator")?))
}

/// F3 check via the ONE pin law ([`Protocol::pin`] — never a second
/// hashing here). `Intact` = still the exact protocol the session
/// convened under; `Moved` = the protocol moved and the F11 choreography
/// ([`pause_and_re_dispatch`]) starts FROM this outcome.
pub fn check_pin(session: &QuorumSession, current: &Protocol) -> Result<PinOutcome, QuorumErr> {
    let actual = current.pin();
    if actual == session.pinned_protocol {
        Ok(PinOutcome::Intact)
    } else {
        Ok(PinOutcome::Moved(PinMismatch {
            pinned: session.pinned_protocol.clone(),
            actual,
        }))
    }
}

/// F11 outcome: the archived original (immutable — moved, never mutated)
/// and the live re-dispatched session.
#[derive(Debug, Clone, PartialEq)]
pub struct PausedReDispatch {
    pub archived: QuorumSession,
    pub re_dispatched: QuorumSession,
}

/// F11: mid-flight protocol edit → PAUSE + version bump + RE-DISPATCH,
/// original archived. Never a hard abort (DoS vector; protocol edits are
/// warden-gated so re-dispatch is safe). The same choreography the
/// council card ships, over the pool body:
///
/// Laws: both cards must validate as QUORUM cards; the archived card's
/// pin must equal the session's stored pin (the pin is the F3 truth;
/// sha256 is one-way, so the version law reads the card, never the
/// hash); the supplied council must BE the council the session convened
/// after (F9 disjointness is proven against THAT panel — swapping
/// councils mid-flight is a Defect); the new version must be STRICTLY
/// greater; the new pin must differ (no-op law); the F1 gate re-runs
/// (EVERY convening is gated); the pool re-selects under the NEW floors;
/// the re-run id derives deterministically (`{original}#r{version}`) and
/// `rerun_of` points at the archived id.
pub fn pause_and_re_dispatch(
    session: QuorumSession,
    archived_protocol: &Protocol,
    new_protocol: &Protocol,
    council_session: &CouncilSession,
    candidates: &[Seat],
    actor: &str,
    warden_ledger_text: &str,
) -> Result<PausedReDispatch, QuorumErr> {
    validate_card(archived_protocol)?;
    if archived_protocol.pin() != session.pinned_protocol {
        return Err(QuorumErr::Defect(
            "supplied archived card does not hash to the session's pinned protocol — the session ran under a different card".into(),
        ));
    }
    if council_session.convening.id != session.council_convening {
        return Err(QuorumErr::Defect(format!(
            "re-dispatch must re-prove disjointness against the SAME council — session convened after '{}', supplied council is '{}'",
            session.council_convening, council_session.convening.id
        )));
    }
    validate_card(new_protocol)?;
    if new_protocol.version <= archived_protocol.version {
        return Err(QuorumErr::VersionNotBumped {
            have: archived_protocol.version,
            try_use: new_protocol.version,
        });
    }
    let new_pin = new_protocol.pin();
    if new_pin == session.pinned_protocol {
        return Err(QuorumErr::SameCard { pin: new_pin });
    }
    let gate_receipt = gate(warden_ledger_text, actor)?;
    let pool = select_quorum_pool(
        &council_session.convening.panel,
        candidates,
        new_protocol.floors.panel_size,
        &new_protocol.floors,
    )?;
    let re_dispatched = QuorumSession {
        id: format!("{}#r{}", session.id, new_protocol.version),
        task: session.task.clone(),
        pinned_protocol: new_pin,
        pool,
        dispatch_log: Vec::new(),
        council_convening: session.council_convening.clone(),
        stakes: session.stakes,
        gate: gate_receipt,
        rerun_of: Some(session.id.clone()),
    };
    Ok(PausedReDispatch {
        archived: session,
        re_dispatched,
    })
}

#[cfg(test)]
#[path = "quorum_tests.rs"]
mod tests;
