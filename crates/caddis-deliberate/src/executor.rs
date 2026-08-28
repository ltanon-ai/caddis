//! executor.rs — P3 slice 1: the DISPATCH ENGINE (plan P3, R4/F4/Ruling 7).
//! The executor runs a Convening under a Protocol: it CONSUMES the
//! council card's stages as mechanics — `dispatch` (as planned waves),
//! `collect`, `integrate`, `verdict`, `ledger` — plus the R4 session
//! cards. Nothing here re-plans, re-gates, or re-pins: every law lives in
//! its ONE home and this module only executes under them.
//!
//! Laws transcribed:
//! - **F4 via Ruling 7 — waves, never improvisation**: the wave plan IS
//!   [`council::dispatch_plan`] ([`crate::caps::plan_batches`], the ONE
//!   planner). Wave N+1 legs start only after wave N fully joined; WITHIN
//!   a wave legs run concurrently (`std::thread::scope`).
//!   Serialized-by-default is the REGISTRY's own default caps (1) —
//!   parallelism appears exactly where a raised cap allows it, and caps
//!   move only through the warden-gated edit path: the registry row IS
//!   the per-lane circuit-breaker flag's one home. The executor adds no
//!   second flag, ever.
//! - **F3 before every wave**: the protocol card is re-read through the
//!   caller's snapshot closure each wave (re-derive-per-use, the registry
//!   view law) and [`council::check_pin`]ed; `Moved` returns
//!   [`ExecErr::PinMoved`] carrying the session BACK — the F11
//!   choreography ([`council::pause_and_re_dispatch`]) starts from that
//!   outcome, never a hard abort (a hard abort is a DoS vector).
//! - **Fail-closed**: a lane transport failure is [`ExecErr::LaneRefused`]
//!   — nothing verdicts on a partial bundle (the collect law); a missing
//!   lane or a lane-type crossing is a [`ExecErr::Defect`] (wiring, exit
//!   2); a lane thread panic is a Defect. Usage rows are appended for
//!   answered seats as their wave completes, so a mid-run refusal leaves
//!   open + partial usage and NO close — an auditable hole.
//! - **Identity**: lanes are resolved against the SESSION's pinned panel
//!   (the registry plans the waves; the panel is the identity truth) and
//!   the lane's declared type must equal the seated type.
//! - **Provenance**: the usage row's model is the TRANSPORT-served model
//!   from the lane's own output — never the seat's registered
//!   self-report.
//! - **P0 seam filled**: each answered leg appends a
//!   [`crate::protocol::DispatchEntry`] (stage `dispatch`, sha256 of the
//!   exact payload) to the convening's dispatch log — a convening is
//!   auditable from birth through execution.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::sync::Arc;

use crate::council::{self, Bundled, CouncilErr, CouncilSession, DisagreementMap, Position, Reply};
use crate::protocol::{DispatchEntry, PinMismatch, Protocol, Verdict};
use crate::registry::Registry;
use crate::sessions::{SessionClose, SessionOpen, SessionRow, SessionUsage};
use crate::sha256;

/// One lane's answer, normalized by the lane adapter: the position the
/// transport's answer maps onto (adapter's job — the estate's verdict
/// vocabulary), the model the TRANSPORT says it served, and the usage
/// counts from the transport record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneOutput {
    pub transport_served_model: String,
    pub position: Position,
    pub tokens_in: u64,
    pub tokens_out: u64,
}

/// A lane transport failure — the seat did not answer. Refusal class:
/// nothing verdict-landed, and retries are later dispatch work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneErr {
    pub lane_id: String,
    pub reason: String,
}

/// How a seat is reached at execution time (Ruling 5 lane types are
/// first-class; the adapters for real transports are later slices — this
/// slice runs stub fixtures against the trait).
pub trait Lane: Send + Sync {
    fn lane_id(&self) -> &str;
    fn lane_type(&self) -> crate::LaneType;
    fn invoke(&self, task: &str) -> Result<LaneOutput, LaneErr>;
}

/// The lane implementations an executor run may reach, keyed by lane id.
#[derive(Default)]
pub struct LaneSet {
    lanes: BTreeMap<String, Arc<dyn Lane>>,
}

impl fmt::Debug for LaneSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.lanes.keys()).finish()
    }
}

impl LaneSet {
    pub fn new() -> LaneSet {
        LaneSet::default()
    }

    /// Builder add.
    pub fn with(mut self, lane: Arc<dyn Lane>) -> LaneSet {
        self.lanes.insert(lane.lane_id().to_string(), lane);
        self
    }

    pub fn get(&self, lane_id: &str) -> Option<&Arc<dyn Lane>> {
        self.lanes.get(lane_id)
    }

    pub fn len(&self) -> usize {
        self.lanes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lanes.is_empty()
    }
}

/// Honest failure taxonomy (edits law): `is_refusal()` = exit 1 — nothing
/// verdict-landed. Defects (exit 2): wiring, identity crossings, I/O on
/// the session stream, panicked lane threads.
#[derive(Debug, Clone, PartialEq)]
pub enum ExecErr {
    /// F3: the protocol moved before a wave; the session rides back
    /// (boxed — the enum stays small for every other path) for the F11
    /// pause→re-dispatch choreography.
    PinMoved {
        session: Box<CouncilSession>,
        mismatch: PinMismatch,
    },
    /// A lane transport failed — the run refuses (no verdict on partial).
    LaneRefused { lane_id: String, reason: String },
    /// A council law refused inside the executor (dispatch plan refusal,
    /// collect defects, card problems).
    Council(CouncilErr),
    /// Defect: missing lane, lane-type crossing, session-stream I/O,
    /// panicked lane thread.
    Defect(String),
}

impl ExecErr {
    pub fn is_refusal(&self) -> bool {
        match self {
            ExecErr::PinMoved { .. } | ExecErr::LaneRefused { .. } => true,
            ExecErr::Council(c) => c.is_refusal(),
            ExecErr::Defect(_) => false,
        }
    }
}

impl fmt::Display for ExecErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExecErr::PinMoved { mismatch, .. } => write!(f, "{mismatch}"),
            ExecErr::LaneRefused { lane_id, reason } => {
                write!(f, "lane '{lane_id}' did not answer: {reason}")
            }
            ExecErr::Council(c) => write!(f, "{c}"),
            ExecErr::Defect(m) => write!(f, "executor defect: {m}"),
        }
    }
}

impl std::error::Error for ExecErr {}

impl From<CouncilErr> for ExecErr {
    fn from(c: CouncilErr) -> Self {
        ExecErr::Council(c)
    }
}

/// One completed executor run: the session with its dispatch log filled,
/// every council artifact, and every session-card line appended.
#[derive(Debug, Clone, PartialEq)]
pub struct Executed {
    pub session: CouncilSession,
    pub bundled: Bundled,
    pub map: DisagreementMap,
    pub verdict: Verdict,
    pub ledger_row: String,
    pub session_rows: Vec<String>,
}

/// Run a council session end-to-end: dispatch waves → collect →
/// integrate → verdict → ledger row, with R4 session cards on the
/// `sessions_stream` path.
///
/// `current_protocol` is re-read BEFORE EVERY WAVE (F3) — pass
/// `|| card.clone()` for the static case; a closure that re-reads the
/// card from its home models the real mid-flight edit.
pub fn run_council(
    session: CouncilSession,
    current_protocol: impl Fn() -> Protocol,
    reg: &Registry,
    lanes: &LaneSet,
    sessions_stream: &Path,
) -> Result<Executed, ExecErr> {
    // STAGE dispatch (planning — the ONE planner, never re-planned here).
    let waves = council::dispatch_plan(&session, reg)?;

    // Identity: resolve every panel seat against the LaneSet BEFORE any
    // dispatch. The panel is the pinned identity truth.
    for ps in &session.convening.panel.seats {
        let lane = lanes.get(&ps.seat.lane_id).ok_or_else(|| {
            ExecErr::Defect(format!(
                "panel seats lane '{}' but the LaneSet carries no such lane",
                ps.seat.lane_id
            ))
        })?;
        if lane.lane_type() != ps.seat.lane_type {
            return Err(ExecErr::Defect(format!(
                "lane '{}' is a {:?} lane but the panel seated it as {:?} — identity crossing",
                ps.seat.lane_id,
                lane.lane_type(),
                ps.seat.lane_type
            )));
        }
    }

    let mut session = session;
    let mut session_rows: Vec<String> = Vec::new();
    let mut replies: Vec<Reply> = Vec::new();
    let payload_digest = sha256::hex(session.convening.task.as_bytes());

    for (wi, wave) in waves.iter().enumerate() {
        // F3 before every wave (the re-read is the caller's; the check is
        // the ONE pin law).
        let proto = current_protocol();
        if let council::PinOutcome::Moved(m) = council::check_pin(&session, &proto)? {
            return Err(ExecErr::PinMoved {
                session: Box::new(session),
                mismatch: m,
            });
        }

        // R4: the open row lands after the first pin proof, before the
        // first leg.
        if wi == 0 {
            let row = SessionRow::Open(SessionOpen {
                conv: session.convening.id.clone(),
                kind: "council".to_string(),
                pin: session.convening.pinned_protocol.clone(),
                stakes: session.stakes.as_str().to_string(),
                rerun_of: session.rerun_of.clone().unwrap_or_default(),
                actor: session.gate.actor.clone(),
                warden_card: session.gate.warden_card.clone(),
            });
            append_session_row(sessions_stream, &row, &mut session_rows)?;
        }

        // One wave: legs run concurrently, the wave joins before the next.
        let results = run_wave(&session.convening.task, wave, lanes);

        // Panel-order usage + dispatch-log appends for every answered
        // leg, THEN the first failure refuses (crash-honest partial).
        for (lane_id, res) in results {
            match res {
                Ok(out) => {
                    let seat = session
                        .convening
                        .panel
                        .seats
                        .iter()
                        .find(|ps| ps.seat.lane_id == lane_id)
                        .expect("wave lanes come from the panel");
                    let row = SessionRow::Usage(SessionUsage {
                        conv: session.convening.id.clone(),
                        lane: lane_id.clone(),
                        lane_type: seat.seat.lane_type,
                        provider: seat.seat.provider.clone(),
                        model: out.transport_served_model.clone(),
                        cost_class: seat.seat.cost_class,
                        tokens_in: out.tokens_in,
                        tokens_out: out.tokens_out,
                    });
                    append_session_row(sessions_stream, &row, &mut session_rows)?;
                    session.convening.dispatch_log.push(DispatchEntry {
                        stage: "dispatch".to_string(),
                        lane_id: lane_id.clone(),
                        payload_digest: payload_digest.clone(),
                    });
                    replies.push(Reply {
                        lane_id: lane_id.clone(),
                        transport_served_model: out.transport_served_model,
                        position: out.position,
                    });
                }
                Err(e) => {
                    return Err(ExecErr::LaneRefused {
                        lane_id: e.lane_id,
                        reason: e.reason,
                    })
                }
            }
        }
    }

    // STAGES collect → integrate → verdict → ledger (the ONE laws).
    let bundled = council::collect(&session.convening, &replies)?;
    let map = council::integrate(&bundled);
    let verdict = council::verdict(&session, &bundled, &map);
    let ledger_row = council::ledger_row(&session, &verdict, &map);
    let digest = sha256::hex(council::canonical_verdict_bytes(&verdict, &map).as_bytes());
    let close = SessionRow::Close(SessionClose {
        conv: session.convening.id.clone(),
        verdict_digest: digest,
        ship: map.holding(Position::Ship).len() as u64,
        ship_with_changes: map.holding(Position::ShipWithChanges).len() as u64,
        do_not_ship: map.holding(Position::DoNotShip).len() as u64,
    });
    append_session_row(sessions_stream, &close, &mut session_rows)?;

    Ok(Executed {
        session,
        bundled,
        map,
        verdict,
        ledger_row,
        session_rows,
    })
}

/// One wave-leg failure (internal; folded into [`ExecErr`] by the caller).
struct WaveErr {
    lane_id: String,
    reason: String,
}

/// Dispatch one wave: every leg on its own thread (F4 — concurrency
/// within a wave is exactly what the cap law allowed into it), join all,
/// results in wave order (= panel order — the planner preserves it).
/// A panicked lane thread is reported as a Defect, never swallowed.
fn run_wave(
    task: &str,
    wave: &[String],
    lanes: &LaneSet,
) -> Vec<(String, Result<LaneOutput, WaveErr>)> {
    let joined: Vec<(String, Option<Result<LaneOutput, LaneErr>>)> = std::thread::scope(|s| {
        let handles: Vec<_> = wave
            .iter()
            .map(|lane_id| {
                let lane = &lanes.lanes[lane_id.as_str()];
                s.spawn(move || (lane_id.clone(), lane.invoke(task)))
            })
            .collect();
        handles
            .into_iter()
            .map(|h| match h.join() {
                Ok((id, r)) => (id, Some(r)),
                Err(_) => (String::new(), None),
            })
            .collect()
    });
    joined
        .into_iter()
        .zip(wave.iter())
        .map(|((id, r), want)| {
            let id = if id.is_empty() { want.clone() } else { id };
            let out = match r {
                Some(inner) => inner.map_err(|e| WaveErr {
                    lane_id: e.lane_id,
                    reason: e.reason,
                }),
                None => Err(WaveErr {
                    lane_id: want.clone(),
                    reason: "lane thread panicked".to_string(),
                }),
            };
            (id, out)
        })
        .collect()
}

fn append_session_row(
    path: &Path,
    row: &SessionRow,
    rows: &mut Vec<String>,
) -> Result<(), ExecErr> {
    crate::sessions::append_row(path, row)
        .map_err(|e| ExecErr::Defect(format!("session stream append failed: {e}")))?;
    rows.push(crate::sessions::encode_row(row));
    Ok(())
}

#[cfg(test)]
#[path = "executor_tests.rs"]
mod tests;
