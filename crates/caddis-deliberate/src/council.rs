//! council.rs — P2 slice 1: the COUNCIL protocol card v1 (plan P2, R3).
//!
//! Laws transcribed:
//! - **The card is DATA** (plan P2): [`protocol_v1`] is the versioned
//!   council card — the seven canonical stages `convene → panel → dispatch
//!   → collect → integrate → verdict → ledger` (the stage vocabulary
//!   [`crate::protocol`] already pins) with the brief's floor PRIORS
//!   ([`crate::Floors::default`] — no new numbers invented here).
//!   [`validate_card`] is the mechanical check: kind Council, the exact
//!   seven stages in order, version >= 1, floors coherent. Changing any
//!   behavioral field = new card version + quorum sign-off (house law);
//!   the F3 pin covers every field either way, so a forgotten bump still
//!   fails [`crate::protocol::Convening::verify_pin`].
//! - **F1 (warden wraps deliberation, no trust bypass)**: EVERY convening
//!   requires an ACTIVE warden card for the convening actor — including
//!   the F11 re-dispatch, which is a NEW convening. The gate is the same
//!   law [`crate::edits`] already ships: derived READ-ONLY from the warden
//!   ledger through [`caddis_warden::card_state::active_for`], caller
//!   match EXACT, unreadable rows = Defect (an answer that cannot be
//!   proven must not look like "no"), no active card = [`CouncilErr::GateClosed`]
//!   refusal with nothing written. The receipt ([`GateReceipt`]) records
//!   WHO convened under WHICH card — F1 evidence travels with the session.
//! - **F3 reuse, never a second pin**: the pin lives in
//!   [`crate::protocol`] ([`crate::protocol::Protocol::pin`], stored by
//!   `Convening::open`). This module only CALLS `verify_pin` via
//!   [`check_pin`] — there is deliberately no second hashing or pin field.
//! - **F11 (mid-flight edit → PAUSE + version bump + RE-DISPATCH, original
//!   archived)**: [`pause_and_re_dispatch`] never hard-aborts (a hard
//!   abort is a DoS vector; protocol edits are warden-gated so re-dispatch
//!   is safe). The new card must validate, its version must be STRICTLY
//!   greater (the bump is mandatory — an edit that changes floors but
//!   forgets the version bump is refused), the pin must therefore differ,
//!   the F1 gate re-runs, the panel re-constructs under the NEW floors,
//!   and the re-run convening carries `rerun_of = Some(original id)` with
//!   id `{original}#r{version}`. The archived original is returned
//!   IMMUTABLE — moved, never mutated.
//! - **Serialized dispatch as DATA** (plan P3 preview): [`dispatch_plan`]
//!   is [`crate::caps::plan_batches`] over the panel in panel order —
//!   Ruling 7 enforced before any lane is touched; the executor that
//!   actually dispatches is P3 and refuses unpinned/mismatched
//!   convenings.
//! - **Collect is fail-closed**: a council verdict NEVER lands on a
//!   partial bundle — a missing seat reply is [`CouncilErr::CollectIncomplete`]
//!   (a refusal; late lanes and retries are P3 dispatch work), a reply
//!   from a lane outside the panel is a Defect (identity crossing), a
//!   duplicate reply is a Defect, an empty `transport_served_model` is a
//!   Defect (the provenance law has no blank form).
//! - **Integration maps disagreement, NEVER averages** (plan P2):
//!   [`integrate`] builds [`DisagreementMap`] — position clusters with
//!   sorted lane ids, fixed cluster order, all three positions always
//!   present (fixed table shape). There is deliberately NO score, mean,
//!   or majority resolution anywhere in this module: the council is
//!   ADVISORY (charter: read-only advisory, the deciding human rules),
//!   so the map IS the integration and the deciding reader sees exactly
//!   who stands where.
//! - **Provenance from TRANSPORT records only** (brief lesson):
//!   [`Reply::transport_served_model`] is named for its source; there is
//!   no self-report field.
//! - **Ledger row** (the final stage): [`ledger_row`] renders the verdict
//!   table as ONE flat exact-field JSON line (registry grammar: same
//!   count, same names, no duplicates, no unknowns), deterministic bytes,
//!   no timestamps (MV11: the warden ledger owns times), plus a sha256
//!   digest over the canonical verdict bytes. [`parse_ledger_row`] is the
//!   one parser (parse law in exactly one place).
//!
//! Quorum card v1 (3 seats from the DISJOINT pool, floor 2/3, asterisk
//! under degradation) is P2 slice 2 — NOT this module.

use std::fmt;

use crate::protocol::{
    Convening, ConveningErr, PinMismatch, Protocol, ProtocolKind, ProvenanceRow, Verdict,
};
use crate::registry::Registry;
use crate::{caps, construct_panel, sha256, Floors, PanelErr, Seat};
use caddis_warden::card_state;

/// The seven canonical council stages, in execution order (plan P2; the
/// vocabulary [`crate::protocol`] documents). A council card without
/// exactly these stages, in this order, does not validate.
pub const COUNCIL_STAGES: &[&str] = &[
    "convene",
    "panel",
    "dispatch",
    "collect",
    "integrate",
    "verdict",
    "ledger",
];

/// The council protocol card v1: kind Council, version 1, the canonical
/// seven stages, floor PRIORS ([`Floors::default`] — panel 4 / families 2
/// / non-Chinese 1). Floor changes are operator rulings and land as NEW
/// versions through quorum sign-off, never as edits to v1.
pub fn protocol_v1() -> Protocol {
    Protocol {
        version: 1,
        kind: ProtocolKind::Council,
        stages: COUNCIL_STAGES.iter().map(|s| s.to_string()).collect(),
        floors: Floors::default(),
    }
}

/// Mechanical card validation (plan P2 Done-When: "both protocol cards
/// validate mechanically"). Structural, not policy: kind must be Council,
/// stages must be exactly [`COUNCIL_STAGES`], version >= 1, floors
/// coherent (positive sizes, minima that fit inside the panel). An
/// operator ruling may re-floor or re-version — never re-shape the
/// pipeline through the back door.
pub fn validate_card(p: &Protocol) -> Result<(), CouncilErr> {
    if p.kind != ProtocolKind::Council {
        return Err(CouncilErr::CardInvalid(format!(
            "kind is {} — this module only carries council cards",
            p.kind.as_str()
        )));
    }
    if p.version == 0 {
        return Err(CouncilErr::CardInvalid(
            "version 0 — cards start at 1".into(),
        ));
    }
    if p.stages.len() != COUNCIL_STAGES.len()
        || p.stages
            .iter()
            .zip(COUNCIL_STAGES)
            .any(|(have, want)| have != want)
    {
        return Err(CouncilErr::CardInvalid(format!(
            "stages must be exactly {:?} in order — have {:?}",
            COUNCIL_STAGES, p.stages
        )));
    }
    let f = &p.floors;
    if f.panel_size == 0 || f.min_families == 0 || f.min_non_chinese == 0 {
        return Err(CouncilErr::CardInvalid(format!(
            "floors must be positive — panel_size={}, min_families={}, min_non_chinese={}",
            f.panel_size, f.min_families, f.min_non_chinese
        )));
    }
    if f.min_families > f.panel_size || f.min_non_chinese > f.panel_size {
        return Err(CouncilErr::CardInvalid(format!(
            "floor minima exceed the panel — panel_size={}, min_families={}, min_non_chinese={}",
            f.panel_size, f.min_families, f.min_non_chinese
        )));
    }
    Ok(())
}

/// The charter decision-ladder weight the caller convenes under. DATA the
/// organ records; the ladder itself (which stakes convene what) belongs
/// to the caller, never the organ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stakes {
    Small,
    Medium,
    Complex,
}

impl Stakes {
    pub fn as_str(self) -> &'static str {
        match self {
            Stakes::Small => "small",
            Stakes::Medium => "medium",
            Stakes::Complex => "complex",
        }
    }
}

/// F1 evidence: which actor convened under which warden card. Travels with
/// the session and lands in the ledger row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateReceipt {
    pub actor: String,
    pub warden_card: String,
}

/// One council convening plus its F1 receipt and stakes (plan P2:
/// convene(task, stakes)). `rerun_of` is the F11 re-run flag:
/// `Some(archived convening id)` = this session re-dispatched after a
/// mid-flight protocol edit; `None` = first convening.
#[derive(Debug, Clone, PartialEq)]
pub struct CouncilSession {
    pub convening: Convening,
    pub stakes: Stakes,
    pub gate: GateReceipt,
    pub rerun_of: Option<String>,
}

/// Honest failure taxonomy (router AuthorErr / edits EditErr law):
/// `is_refusal()` = exit 1 — nothing was written: the gate was closed, the
/// card invalid, the panel floors unsatisfiable, the bundle incomplete,
/// the version not bumped. Everything else is a Defect (exit 2) —
/// malformed input, unreadable ledger, identity crossings.
#[derive(Debug, Clone, PartialEq)]
pub enum CouncilErr {
    /// The protocol card fails mechanical validation.
    CardInvalid(String),
    /// F1: no active warden card for the convening actor.
    GateClosed { actor: String },
    /// Panel construction refusal (degraded day = honest refusal, F9 law).
    Panel(PanelErr),
    /// Convening construction refusal (size mismatch; defensive).
    Convening(ConveningErr),
    /// The bundle is missing replies from panel seats (P3 retries; this
    /// organ refuses to integrate partials).
    CollectIncomplete { missing: Vec<String> },
    /// F11: the re-dispatch card's version is not a strict bump.
    VersionNotBumped { have: u32, try_use: u32 },
    /// F11: the re-dispatch card hashes identical to the archived pin
    /// (no-op law — a re-dispatch is a change).
    SameCard { pin: String },
    /// Dispatch planning failure (panel/registry mismatch, zero caps).
    Dispatch(String),
    /// Defect: malformed input, unreadable ledger, identity crossing.
    Defect(String),
}

impl CouncilErr {
    /// Refusal (exit 1) vs Defect (exit 2) — the edits-law split.
    pub fn is_refusal(&self) -> bool {
        matches!(
            self,
            CouncilErr::CardInvalid(_)
                | CouncilErr::GateClosed { .. }
                | CouncilErr::Panel(_)
                | CouncilErr::Convening(_)
                | CouncilErr::CollectIncomplete { .. }
                | CouncilErr::VersionNotBumped { .. }
                | CouncilErr::SameCard { .. }
        )
    }
}

impl fmt::Display for CouncilErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CouncilErr::CardInvalid(why) => write!(f, "council card invalid: {why}"),
            CouncilErr::GateClosed { actor } => write!(
                f,
                "warden gate closed: no active card for '{actor}' — F1 refuses the convening"
            ),
            CouncilErr::Panel(e) => write!(f, "panel construction refused: {e}"),
            CouncilErr::Convening(e) => write!(f, "convening refused: {e}"),
            CouncilErr::CollectIncomplete { missing } => write!(
                f,
                "bundle incomplete — missing replies from {}",
                missing.join(", ")
            ),
            CouncilErr::VersionNotBumped { have, try_use } => write!(
                f,
                "re-dispatch card version {try_use} is not a bump over {have} — F11 mandates a strict version bump"
            ),
            CouncilErr::SameCard { pin } => {
                write!(f, "re-dispatch card pins identical to the archived protocol ({pin}) — a re-dispatch is a change")
            }
            CouncilErr::Dispatch(why) => write!(f, "dispatch planning failed: {why}"),
            CouncilErr::Defect(why) => write!(f, "defect: {why}"),
        }
    }
}

impl std::error::Error for CouncilErr {}

impl From<PanelErr> for CouncilErr {
    fn from(e: PanelErr) -> Self {
        CouncilErr::Panel(e)
    }
}

impl From<ConveningErr> for CouncilErr {
    fn from(e: ConveningErr) -> Self {
        CouncilErr::Convening(e)
    }
}

/// The F1 gate — the exact law `edits::confirm` ships: derive the active
/// card READ-ONLY from the caller-supplied ledger text; unreadable rows
/// fail CLOSED as a Defect; no active card is a GateClosed refusal.
/// `pub(crate)`: the quorum card (P2 slice 2) reuses THIS ONE gate law —
/// a second copy of the F1 gate is banned.
pub(crate) fn gate(warden_ledger_text: &str, actor: &str) -> Result<GateReceipt, CouncilErr> {
    if actor.is_empty() {
        return Err(CouncilErr::Defect(
            "actor is transport-served and must be non-empty".into(),
        ));
    }
    let cs = card_state::active_for(warden_ledger_text, actor);
    if cs.unreadable > 0 {
        return Err(CouncilErr::Defect(format!(
            "warden ledger holds {} unreadable rows — cannot attest a gate card for '{actor}'",
            cs.unreadable
        )));
    }
    let active = cs.active.ok_or(CouncilErr::GateClosed {
        actor: actor.to_string(),
    })?;
    Ok(GateReceipt {
        actor: actor.to_string(),
        warden_card: active.id,
    })
}

/// STAGE `convene` (+ `panel`): validate the card, pass the F1 gate,
/// construct the panel under the card's floors, and open the convening
/// (which PINS the protocol — F3, stored never caller-supplied).
///
/// Order of law: card → gate → panel → convening. Nothing is written by
/// this function — a [`CouncilSession`] is a value; the P3 executor
/// persists and dispatches under these same gates.
pub fn convene(
    id: impl Into<String>,
    task: impl Into<String>,
    stakes: Stakes,
    protocol: &Protocol,
    candidates: &[Seat],
    actor: &str,
    warden_ledger_text: &str,
) -> Result<CouncilSession, CouncilErr> {
    let id = id.into();
    let task = task.into();
    if id.is_empty() || task.is_empty() {
        return Err(CouncilErr::Defect(
            "convening id and task must be non-empty".into(),
        ));
    }
    validate_card(protocol)?;
    let gate_receipt = gate(warden_ledger_text, actor)?;
    let panel = construct_panel(candidates, &protocol.floors)?;
    let convening = Convening::open(id, task, protocol, panel)?;
    Ok(CouncilSession {
        convening,
        stakes,
        gate: gate_receipt,
        rerun_of: None,
    })
}

/// STAGE `dispatch` (as DATA — the P3 executor runs it): the serialized
/// wave plan for the seated panel, in panel order, under the registry's
/// caps law (Ruling 7: a capped provider never shares a wave).
pub fn dispatch_plan(
    session: &CouncilSession,
    reg: &Registry,
) -> Result<Vec<Vec<String>>, CouncilErr> {
    let wanted: Vec<&str> = session
        .convening
        .panel
        .seats
        .iter()
        .map(|ps| ps.seat.lane_id.as_str())
        .collect();
    caps::plan_batches(&wanted, reg).map_err(|e| CouncilErr::Dispatch(e.to_string()))
}

/// A seat's position on the task — the estate's verdict vocabulary
/// (VERDICT.md language: SHIP / SHIP-WITH-CHANGES / DO-NOT-SHIP),
/// transcribed, not invented.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Position {
    Ship,
    ShipWithChanges,
    DoNotShip,
}

impl Position {
    /// Canonical wire token; fixed cluster order in [`DisagreementMap`].
    pub fn as_str(self) -> &'static str {
        match self {
            Position::Ship => "ship",
            Position::ShipWithChanges => "ship_with_changes",
            Position::DoNotShip => "do_not_ship",
        }
    }
}

/// One bundled reply. `transport_served_model` comes from the TRANSPORT
/// record only (brief lesson) — there is no self-report constructor or
/// field anywhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub lane_id: String,
    pub transport_served_model: String,
    pub position: Position,
}

/// The collected bundle: one reply per seated lane, in PANEL order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bundled {
    pub replies: Vec<Reply>,
}

/// STAGE `collect`: fold raw replies into the bundle. Fail-closed laws:
/// every panel seat exactly one reply (missing = [`CouncilErr::CollectIncomplete`]
/// refusal listing the missing lanes), no replies from lanes outside the
/// panel (Defect — identity crossing), no duplicates (Defect), no empty
/// transport-served model (Defect — provenance has no blank form).
pub fn collect(convening: &Convening, replies: &[Reply]) -> Result<Bundled, CouncilErr> {
    let mut by_lane: std::collections::BTreeMap<&str, &Reply> = std::collections::BTreeMap::new();
    for r in replies {
        if r.transport_served_model.is_empty() {
            return Err(CouncilErr::Defect(format!(
                "reply from '{}' carries an empty transport_served_model — provenance has no blank form",
                r.lane_id
            )));
        }
        if by_lane.insert(r.lane_id.as_str(), r).is_some() {
            return Err(CouncilErr::Defect(format!(
                "duplicate reply from '{}' — one reply per seat, exactly",
                r.lane_id
            )));
        }
    }
    let panel_lanes: std::collections::BTreeSet<&str> = convening
        .panel
        .seats
        .iter()
        .map(|ps| ps.seat.lane_id.as_str())
        .collect();
    for lane in by_lane.keys() {
        if !panel_lanes.contains(lane) {
            return Err(CouncilErr::Defect(format!(
                "reply from '{lane}' — that lane is not seated on this panel (identity crossing)"
            )));
        }
    }
    let mut missing: Vec<String> = panel_lanes
        .iter()
        .filter(|l| !by_lane.contains_key(**l))
        .map(|l| l.to_string())
        .collect();
    if !missing.is_empty() {
        missing.sort();
        return Err(CouncilErr::CollectIncomplete { missing });
    }
    // Panel order, deterministic.
    let ordered = convening
        .panel
        .seats
        .iter()
        .map(|ps| by_lane[ps.seat.lane_id.as_str()].clone())
        .collect();
    Ok(Bundled { replies: ordered })
}

/// One position cluster: the position and the lanes holding it, sorted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PositionCluster {
    pub position: Position,
    pub lane_ids: Vec<String>,
}

/// STAGE `integrate`: the disagreement MAP — who stands where. All three
/// positions are ALWAYS present (fixed table shape), clusters in fixed
/// position order, lane ids sorted. There is deliberately no score, mean,
/// or majority resolution: the council is ADVISORY and the deciding
/// human reads exactly this map (never-averaging law, plan P2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisagreementMap {
    pub clusters: Vec<PositionCluster>,
}

impl DisagreementMap {
    /// Did the panel disagree? (Distinct occupied positions > 1.)
    pub fn disagrees(&self) -> bool {
        self.clusters
            .iter()
            .filter(|c| !c.lane_ids.is_empty())
            .count()
            > 1
    }

    /// Lanes holding `position` (empty slice if none).
    pub fn holding(&self, position: Position) -> &[String] {
        self.clusters
            .iter()
            .find(|c| c.position == position)
            .map(|c| c.lane_ids.as_slice())
            .unwrap_or(&[])
    }

    /// The TABLE summary, deterministic: `ship=2,ship_with_changes=1,do_not_ship=0`.
    /// This is the ruling text a council verdict carries — counts, never
    /// an average, never a silent majority.
    pub fn summary(&self) -> String {
        self.clusters
            .iter()
            .map(|c| format!("{}={}", c.position.as_str(), c.lane_ids.len()))
            .collect::<Vec<_>>()
            .join(",")
    }

    /// The position a lane holds (identity lookup for the ledger digest).
    pub fn position_of(&self, lane_id: &str) -> Option<Position> {
        self.clusters
            .iter()
            .find(|c| c.lane_ids.iter().any(|l| l == lane_id))
            .map(|c| c.position)
    }
}

/// STAGE `integrate` (constructor): cluster the bundle. Pure,
/// deterministic: fixed cluster order ([`Position`] declaration order),
/// lane ids sorted lexicographically inside each cluster.
pub fn integrate(bundled: &Bundled) -> DisagreementMap {
    let mut map = DisagreementMap {
        clusters: [
            Position::Ship,
            Position::ShipWithChanges,
            Position::DoNotShip,
        ]
        .iter()
        .map(|&p| PositionCluster {
            position: p,
            lane_ids: Vec::new(),
        })
        .collect(),
    };
    for r in &bundled.replies {
        let cluster = map
            .clusters
            .iter_mut()
            .find(|c| c.position == r.position)
            .expect("all three positions are pre-seeded");
        cluster.lane_ids.push(r.lane_id.clone());
    }
    for c in &mut map.clusters {
        c.lane_ids.sort();
    }
    map
}

/// STAGE `verdict`: the structured ruling. The ruling text is the
/// disagreement-map summary (the table, never an average); provenance is
/// per-seat TRANSPORT-served model; `degraded` is always FALSE here —
/// council collect refuses partial bundles (fail-closed above), so a
/// council verdict landing at all means the full panel answered. The
/// degradation asterisk is the QUORUM seam (P2 slice 2).
pub fn verdict(session: &CouncilSession, bundled: &Bundled, map: &DisagreementMap) -> Verdict {
    Verdict {
        convening_id: session.convening.id.clone(),
        ruling: map.summary(),
        provenance: bundled
            .replies
            .iter()
            .map(|r| ProvenanceRow {
                lane_id: r.lane_id.clone(),
                lane_type: session
                    .convening
                    .panel
                    .seats
                    .iter()
                    .find(|ps| ps.seat.lane_id == r.lane_id)
                    .map(|ps| ps.seat.lane_type)
                    .expect("collect proved every reply lane is seated"),
                transport_served_model: r.transport_served_model.clone(),
            })
            .collect(),
        degraded: false,
    }
}

/// Canonical verdict bytes — the digest input. Deterministic: convening
/// id, ruling table, degradation flag, then `lane|lane_type|model|position`
/// per provenance row in provenance order.
pub fn canonical_verdict_bytes(verdict: &Verdict, map: &DisagreementMap) -> String {
    let rows = verdict
        .provenance
        .iter()
        .map(|p| {
            let position = map
                .position_of(&p.lane_id)
                .map(|pos| pos.as_str())
                .unwrap_or("unmapped");
            format!(
                "{}|{}|{}|{}",
                p.lane_id,
                match p.lane_type {
                    crate::LaneType::Http => "http",
                    crate::LaneType::Bridge => "bridge",
                    crate::LaneType::Cli => "cli",
                },
                p.transport_served_model,
                position
            )
        })
        .collect::<Vec<_>>()
        .join(";");
    format!(
        "{}|{}|{}|{}",
        verdict.convening_id, verdict.ruling, verdict.degraded, rows
    )
}

/// The exact ledger-row field set (flat grammar, registry law).
const ROW_FIELDS: &[&str] = &[
    "conv",
    "kind",
    "pin",
    "stakes",
    "rerun_of",
    "actor",
    "warden_card",
    "ship",
    "ship_with_changes",
    "do_not_ship",
    "verdict_digest",
];

/// One parsed council ledger row (STAGE `ledger`, read side).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CouncilRow {
    pub conv: String,
    pub pin: String,
    pub stakes: String,
    pub rerun_of: String,
    pub actor: String,
    pub warden_card: String,
    pub ship: u64,
    pub ship_with_changes: u64,
    pub do_not_ship: u64,
    pub verdict_digest: String,
}

/// STAGE `ledger`: render the verdict table as ONE flat exact-field JSON
/// line. Deterministic bytes — no timestamps (MV11: the warden ledger owns
/// times), no secrets, fixed field order. `verdict_digest` = sha256 over
/// [`canonical_verdict_bytes`] — the row is tamper-evident against the
/// verdict it summarizes.
pub fn ledger_row(session: &CouncilSession, verdict: &Verdict, map: &DisagreementMap) -> String {
    let digest = sha256::hex(canonical_verdict_bytes(verdict, map).as_bytes());
    let mut out = String::with_capacity(256);
    out.push('{');
    out.push_str("\"conv\":");
    json_str(&session.convening.id, &mut out);
    out.push_str(",\"kind\":\"council\",\"pin\":");
    json_str(&session.convening.pinned_protocol, &mut out);
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

/// Parse one council ledger row. Exact field law (same count, same names,
/// no duplicates, no unknowns), `kind` must be `council`, counts must be
/// non-negative integers summing to at least one seat, `verdict_digest`
/// must be 64 lowercase hex chars. THE parser — the encode side above is
/// its only producer.
pub fn parse_ledger_row(line: &str) -> Result<CouncilRow, CouncilErr> {
    let v = crate::json::parse(line)
        .map_err(|e| CouncilErr::Defect(format!("ledger row is not JSON: {e:?}")))?;
    let obj = v
        .as_obj()
        .ok_or_else(|| CouncilErr::Defect("ledger row must be a flat object".into()))?;
    let mut names: Vec<&str> = obj.iter().map(|(k, _)| k.as_str()).collect();
    names.sort_unstable();
    let mut want: Vec<&str> = ROW_FIELDS.to_vec();
    want.sort_unstable();
    if names != want {
        return Err(CouncilErr::Defect(format!(
            "ledger row fields must be exactly {want:?} — have {names:?}"
        )));
    }
    let get_str = |key: &str| -> Result<String, CouncilErr> {
        v.get(key)
            .and_then(|x| x.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| CouncilErr::Defect(format!("field '{key}' must be a string")))
    };
    let kind = get_str("kind")?;
    if kind != "council" {
        return Err(CouncilErr::Defect(format!(
            "kind must be 'council' — have '{kind}'"
        )));
    }
    let get_count = |key: &str| -> Result<u64, CouncilErr> {
        let n = v
            .get(key)
            .and_then(|x| x.as_f64())
            .ok_or_else(|| CouncilErr::Defect(format!("field '{key}' must be a number")))?;
        if n < 0.0 || n.fract() != 0.0 {
            return Err(CouncilErr::Defect(format!(
                "field '{key}' must be a non-negative integer"
            )));
        }
        Ok(n as u64)
    };
    let ship = get_count("ship")?;
    let ship_with_changes = get_count("ship_with_changes")?;
    let do_not_ship = get_count("do_not_ship")?;
    if ship + ship_with_changes + do_not_ship == 0 {
        return Err(CouncilErr::Defect(
            "position counts sum to zero — a council verdict has at least one seat".into(),
        ));
    }
    let verdict_digest = get_str("verdict_digest")?;
    if verdict_digest.len() != 64
        || !verdict_digest
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    {
        return Err(CouncilErr::Defect(
            "verdict_digest must be 64 lowercase hex chars".into(),
        ));
    }
    Ok(CouncilRow {
        conv: get_str("conv")?,
        pin: get_str("pin")?,
        stakes: get_str("stakes")?,
        rerun_of: get_str("rerun_of")?,
        actor: get_str("actor")?,
        warden_card: get_str("warden_card")?,
        ship,
        ship_with_changes,
        do_not_ship,
        verdict_digest,
    })
}

/// F3 check via the ONE pin ([`Convening::verify_pin`] — never a second
/// hashing here). `Intact` = still the exact protocol the convening
/// opened under; `Moved` = the protocol moved and the F11 choreography
/// ([`pause_and_re_dispatch`]) starts FROM this outcome.
#[derive(Debug, Clone, PartialEq)]
pub enum PinOutcome {
    Intact,
    Moved(PinMismatch),
}

/// The F3 gate the P3 executor calls before every dispatch leg.
pub fn check_pin(session: &CouncilSession, current: &Protocol) -> Result<PinOutcome, CouncilErr> {
    match session.convening.verify_pin(current) {
        Ok(()) => Ok(PinOutcome::Intact),
        Err(m) => Ok(PinOutcome::Moved(m)),
    }
}

/// F11 outcome: the archived original (immutable — moved, never mutated)
/// and the live re-dispatched session.
#[derive(Debug, Clone, PartialEq)]
pub struct PausedReDispatch {
    pub archived: CouncilSession,
    pub re_dispatched: CouncilSession,
}

/// F11: mid-flight protocol edit → PAUSE + version bump + RE-DISPATCH,
/// original archived. Never a hard abort (DoS vector; protocol edits are
/// warden-gated so re-dispatch is safe).
///
/// The caller supplies the card the session ran under
/// (`archived_protocol`) — its pin is PROVEN equal to the session's
/// pinned protocol before any bump logic runs (the pin is the F3 truth;
/// sha256 is one-way, so the version law reads the card, never the hash).
///
/// Laws: both cards must validate; the new version must be STRICTLY
/// greater than the archived one (an edit that changes floors but
/// forgets the bump is refused); the new pin must differ (no-op law —
/// implied by the bump through the canonical bytes, refused explicitly
/// anyway); the F1 gate re-runs for the re-dispatch (EVERY convening is
/// gated, F1); the panel re-constructs under the NEW floors; the re-run
/// convening id derives deterministically (`{original}#r{version}`) and
/// `rerun_of` points at the archived id.
pub fn pause_and_re_dispatch(
    session: CouncilSession,
    archived_protocol: &Protocol,
    new_protocol: &Protocol,
    candidates: &[Seat],
    actor: &str,
    warden_ledger_text: &str,
) -> Result<PausedReDispatch, CouncilErr> {
    validate_card(archived_protocol)?;
    if archived_protocol.pin() != session.convening.pinned_protocol {
        return Err(CouncilErr::Defect(
            "supplied archived card does not hash to the session's pinned protocol — the session ran under a different card".into(),
        ));
    }
    validate_card(new_protocol)?;
    if new_protocol.version <= archived_protocol.version {
        return Err(CouncilErr::VersionNotBumped {
            have: archived_protocol.version,
            try_use: new_protocol.version,
        });
    }
    let new_pin = new_protocol.pin();
    if new_pin == session.convening.pinned_protocol {
        return Err(CouncilErr::SameCard { pin: new_pin });
    }
    let gate_receipt = gate(warden_ledger_text, actor)?;
    let panel = construct_panel(candidates, &new_protocol.floors)?;
    let new_id = format!("{}#r{}", session.convening.id, new_protocol.version);
    let convening = Convening::open(new_id, session.convening.task.clone(), new_protocol, panel)?;
    let re_dispatched = CouncilSession {
        convening,
        stakes: session.stakes,
        gate: gate_receipt,
        rerun_of: Some(session.convening.id.clone()),
    };
    Ok(PausedReDispatch {
        archived: session,
        re_dispatched,
    })
}

#[cfg(test)]
#[path = "council_tests.rs"]
mod tests;
