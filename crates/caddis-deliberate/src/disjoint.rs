//! disjoint.rs — P0 slice 3: the F9 STRICT quorum-pool disjointness law
//! (BUILD-QUEUE r2-organs-rewrite; quorum verdict F9, 2026-08-26: "strict
//! quorum-pool disjointness ALWAYS; degraded day -> pull from a VETTED
//! RESERVE pool; any overlap requires operator approval, logged. Silent
//! one-overlap soft-disjoint is DEAD.").
//!
//! What lives here and what deliberately does not:
//!
//! - [`select_quorum_pool`] is the same deliberately dumb-deterministic
//!   selection as [`crate::construct_panel`]: live-only (F10), free-first
//!   with `lane_id` ties (the shared [`crate::Seat::selection_key`]), take
//!   `size`, then validate. The only quorum-specific law is the DISJOINT
//!   FILTER: a candidate whose `lane_id` already sits on the council
//!   panel is skipped — one lane may never serve both bodies in one
//!   convening cycle.
//! - Overlap is an ERROR ([`DisjointErr::Overlap`]), never a silently
//!   accepted soft pool; a selected pool is re-proven disjoint before it
//!   is returned (fail-closed post-condition).
//! - The degraded day (fewer disjoint live seats than `size`) is an
//!   HONEST refusal ([`DisjointErr::InsufficientDisjointPool`]) carrying
//!   the day's shape — overlap skips vs non-live skips — as evidence for
//!   the P3 executor. P3 slice 3 delivers the F9 VETTED RESERVE pull
//!   here, beside the strict law: [`select_quorum_pool_with_reserve`] is
//!   the ONLY code path that can produce an overlapping pool — it opens
//!   exclusively on an explicit [`OperatorApproval`] (approved overlap
//!   lanes under a named id) and returns the [`ReserveAudit`] record the
//!   warden ledger cites, on every kind of day. A silent soft-disjoint
//!   stays DEAD: no approval, no overlap, ever (F9).
//! - The SAME [`crate::Floors`] data law (distinct families, then
//!   non-Chinese — fixed refusal order) applies to the pool: a quorum
//!   monoculture would defeat the floor's meaning. Floor VALUES are
//!   protocol data (P2); the quorum SIZE is a parameter because P0 owns
//!   no default for it (the brief's prior is 3; P2 rules it).
//! - Intra-pool duplicate `lane_id`s are assumed impossible by the F8
//!   1:1 card-lane registry law (P1); the substrate does not re-prove
//!   registry invariants.

use std::collections::BTreeSet;
use std::fmt;

use crate::{check_floor_laws, Floors, Panel, PanelErr, Seat};

/// A selected quorum pool: plain seats, NO roles. Roles are the council
/// panel's vocabulary; quorum seats are peers — the vote shape is P2
/// protocol data, not substrate structure.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(test, derive(serde::Serialize, serde::Deserialize))]
pub struct QuorumPool {
    pub seats: Vec<Seat>,
}

impl QuorumPool {
    /// Distinct family count (floor input) — same counting law as
    /// [`Panel::family_count`].
    pub fn family_count(&self) -> usize {
        self.seats
            .iter()
            .map(|s| s.family.as_str())
            .collect::<BTreeSet<_>>()
            .len()
    }

    /// Seats from OUTSIDE the Chinese cluster (floor input) — same
    /// counting law as [`Panel::non_chinese_count`].
    pub fn non_chinese_count(&self) -> usize {
        self.seats
            .iter()
            .filter(|s| !crate::is_chinese_family(&s.family))
            .count()
    }

    /// F9 STRICT: prove ZERO `lane_id` overlap against the council panel.
    /// Any shared lane is an [`DisjointErr::Overlap`] carrying the sorted
    /// offending lanes as evidence.
    pub fn check_disjoint_from(&self, council: &Panel) -> Result<(), DisjointErr> {
        let council_lanes: BTreeSet<&str> = council
            .seats
            .iter()
            .map(|ps| ps.seat.lane_id.as_str())
            .collect();
        let lanes: Vec<String> = self
            .seats
            .iter()
            .filter(|s| council_lanes.contains(s.lane_id.as_str()))
            .map(|s| s.lane_id.clone())
            .collect();
        if lanes.is_empty() {
            Ok(())
        } else {
            Err(DisjointErr::Overlap { lanes })
        }
    }

    /// The same floors law [`Panel::check_floors`] applies — distinct
    /// families first, then non-Chinese (fixed refusal order). Reuses the
    /// [`PanelErr`] floor vocabulary: one law, one error language.
    pub fn check_floors(&self, floors: &Floors) -> Result<(), PanelErr> {
        check_floor_laws(self.family_count(), self.non_chinese_count(), floors)
    }
}

/// Quorum-pool selection refusals. Fail-closed: F9 overlap and every
/// floor violation are REFUSALS, never a soft pool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisjointErr {
    /// `size` was 0 — a usage defect, not a seat problem.
    EmptyPool,
    /// F9 STRICT: lanes sitting on BOTH bodies. Never silently accepted.
    Overlap { lanes: Vec<String> },
    /// Degraded day: fewer disjoint live seats than `size`. The evidence
    /// fields carry the day's shape (overlap skips vs non-live skips) so
    /// the P3 executor and the operator see WHY — reserve pool is P3.
    InsufficientDisjointPool {
        have: usize,
        want: usize,
        skipped_overlap: usize,
        skipped_non_live: usize,
    },
    /// The pool violates the floors — [`PanelErr`] floor vocabulary.
    Floors(PanelErr),
}

impl fmt::Display for DisjointErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DisjointErr::EmptyPool => write!(f, "quorum pool size must be >= 1"),
            DisjointErr::Overlap { lanes } => write!(
                f,
                "F9 disjointness violated: lanes on both bodies: {}",
                lanes.join(", ")
            ),
            DisjointErr::InsufficientDisjointPool {
                have,
                want,
                skipped_overlap,
                skipped_non_live,
            } => write!(
                f,
                "degraded day: disjoint live seats {have} < wanted {want} \
                 (skipped: {skipped_overlap} overlap, {skipped_non_live} non-live); \
                 reserve pool is P3 dispatch work"
            ),
            DisjointErr::Floors(e) => write!(f, "quorum pool floors: {e}"),
        }
    }
}

impl std::error::Error for DisjointErr {}

/// Select a quorum pool disjoint from `council`. Pure, deterministic,
/// fail-closed. Order of law (fixed, deterministic replay):
///
/// 1. `size` must be >= 1 ([`DisjointErr::EmptyPool`]);
/// 2. only [`crate::SeatState::Live`] candidates compete (F10);
/// 3. candidates whose `lane_id` sits on the council panel are skipped
///    BEFORE ordering — skip-then-order, so an overlapped FREE seat never
///    displaces a disjoint costlier one from the pool;
/// 4. order by [`Seat::selection_key`] (free-first, `lane_id` ties) and
///    take `size`;
/// 5. fewer disjoint live seats than `size` = the degraded day — an
///    honest [`DisjointErr::InsufficientDisjointPool`], never a
///    short-handed or overlapping pool;
/// 6. validate floors on the SELECTED pool (families, then non-Chinese);
/// 7. re-prove F9 disjointness before returning — a filter bug is a
///    construction refusal, never a shipped overlap.
pub fn select_quorum_pool(
    council: &Panel,
    candidates: &[Seat],
    size: usize,
    floors: &Floors,
) -> Result<QuorumPool, DisjointErr> {
    if size == 0 {
        return Err(DisjointErr::EmptyPool);
    }
    let council_lanes: BTreeSet<&str> = council
        .seats
        .iter()
        .map(|ps| ps.seat.lane_id.as_str())
        .collect();
    let mut skipped_overlap = 0usize;
    let mut skipped_non_live = 0usize;
    let mut eligible: Vec<&Seat> = Vec::new();
    for seat in candidates {
        if !seat.state.selectable() {
            skipped_non_live += 1;
            continue;
        }
        if council_lanes.contains(seat.lane_id.as_str()) {
            skipped_overlap += 1;
            continue;
        }
        eligible.push(seat);
    }
    if eligible.len() < size {
        return Err(DisjointErr::InsufficientDisjointPool {
            have: eligible.len(),
            want: size,
            skipped_overlap,
            skipped_non_live,
        });
    }
    eligible.sort_by_key(|s| s.selection_key());
    eligible.truncate(size);
    let pool = QuorumPool {
        seats: eligible.into_iter().cloned().collect(),
    };
    pool.check_floors(floors).map_err(DisjointErr::Floors)?;
    pool.check_disjoint_from(council)?;
    Ok(pool)
}

// ---------------------------------------------------------------------------
// P3 slice 3: the F9 degraded-day VETTED RESERVE pull. MODE
// r2-organs-rewrite P3 slice 3 / quorum verdict F9: "degraded day ->
// pull from a VETTED RESERVE pool; any overlap requires operator
// approval, logged. Silent one-overlap soft-disjoint is DEAD."
// ---------------------------------------------------------------------------

/// The operator's explicit, lane-named approval for a reserve pull: the
/// ONLY artifact that may admit a lane onto BOTH bodies in one convening
/// cycle. `id` identifies the approval act for the warden ledger row —
/// blank means unauditable, which is exactly the silent path F9 killed,
/// so it is refused. The lanes are the VETTED reserve: live council
/// lanes the operator has ruled acceptable for double duty. Naming a
/// lane that never competes is harmless — an approval may cover more
/// than the day needs.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(test, derive(serde::Serialize, serde::Deserialize))]
pub struct OperatorApproval {
    pub id: String,
    pub approved_overlap_lanes: Vec<String>,
}

impl OperatorApproval {
    fn approved_set(&self) -> BTreeSet<&str> {
        self.approved_overlap_lanes
            .iter()
            .map(|s| s.as_str())
            .collect()
    }
}

/// The warden-ledger audit of a reserve-authorized selection: WHAT was
/// pulled, under WHOSE approval, and what the day looked like. Returned
/// on every outcome that yields a pool — including a healthy day's
/// (approval unspent) — so a reserve-authorized selection can never be
/// silently absent from the ledger. Lane lists follow POOL order (the
/// shared deterministic selection order; the reserve tail last).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(test, derive(serde::Serialize, serde::Deserialize))]
pub struct ReserveAudit {
    /// The approval act this selection cites (unspent on a healthy day).
    pub approval_id: String,
    /// Disjoint lanes seated — the strict tail of the pool.
    pub disjoint_lanes: Vec<String>,
    /// Lanes pulled from the vetted reserve (serving BOTH bodies). Empty
    /// whenever strict selection alone could fill the pool.
    pub reserve_lanes: Vec<String>,
    /// Non-live candidates skipped (F10) — the day's shape, for the row.
    pub skipped_non_live: usize,
    /// Live council-overlap lanes the approval did NOT name — the
    /// operator's next vetting decision, recorded whether or not it
    /// blocked this selection.
    pub unapproved_live_overlap: Vec<String>,
}

/// A quorum pool selected under [`OperatorApproval`] authority, with the
/// [`ReserveAudit`] the warden ledger cites. The pool may overlap the
/// council panel ONLY on the approved lanes — re-proven by
/// [`ReservePool::check_overlap_within_approval`] before return.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(test, derive(serde::Serialize, serde::Deserialize))]
pub struct ReservePool {
    pub pool: QuorumPool,
    pub audit: ReserveAudit,
}

impl ReservePool {
    /// F9 reserve post-condition: every pool lane that also sits on the
    /// council panel must be one of the approval's vetted lanes.
    /// `Ok(overlap)` returns exactly the double-duty lanes (pool order)
    /// — the audited reserve set; `Err`([`DisjointErr::Overlap`]) names
    /// any UNapproved overlap: a construction bug is a refusal, never a
    /// shipped pool (the same fail-closed post-condition law as the
    /// strict selector's [`QuorumPool::check_disjoint_from`]).
    pub fn check_overlap_within_approval(
        &self,
        council: &Panel,
        approval: &OperatorApproval,
    ) -> Result<Vec<String>, DisjointErr> {
        overlap_within_approval(&self.pool, council, approval)
    }
}

/// The shared post-condition core: the (pool-ordered) lanes serving on
/// both bodies, or the unapproved subset as [`DisjointErr::Overlap`].
fn overlap_within_approval(
    pool: &QuorumPool,
    council: &Panel,
    approval: &OperatorApproval,
) -> Result<Vec<String>, DisjointErr> {
    let council_lanes: BTreeSet<&str> = council
        .seats
        .iter()
        .map(|ps| ps.seat.lane_id.as_str())
        .collect();
    let approved = approval.approved_set();
    let mut overlap = Vec::new();
    let mut unapproved = Vec::new();
    for seat in &pool.seats {
        let lane = seat.lane_id.as_str();
        if council_lanes.contains(lane) {
            overlap.push(seat.lane_id.clone());
            if !approved.contains(lane) {
                unapproved.push(seat.lane_id.clone());
            }
        }
    }
    if unapproved.is_empty() {
        Ok(overlap)
    } else {
        Err(DisjointErr::Overlap { lanes: unapproved })
    }
}

/// Reserve-selector refusals. Fail-closed: the degraded day without a
/// sufficient approval is a REFUSAL, never a short-handed or silently
/// overlapping pool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReserveErr {
    /// `size` was 0 — a usage defect, not a seat problem.
    EmptyPool,
    /// Blank approval id — an unauditable overlap is exactly the silent
    /// path F9 killed. Refused on every kind of day.
    BlankApproval,
    /// The STRICT selector refused for a reason that is NOT the degraded
    /// day (floors, or its own post-condition). The reserve pull exists
    /// ONLY for [`DisjointErr::InsufficientDisjointPool`]; it never
    /// papers over any other strict refusal — reserve must never "fix"
    /// floors by swapping in an overlapping seat.
    Strict(DisjointErr),
    /// The degraded day exhausted: disjoint live seats + APPROVED reserve
    /// candidates still below `size`. `unapproved_live_overlap` carries
    /// the live council lanes the approval did not name — the operator's
    /// next vetting decision, as evidence.
    ReserveExhausted {
        have: usize,
        want: usize,
        skipped_non_live: usize,
        unapproved_live_overlap: Vec<String>,
    },
    /// The final (reserve) pool violates the floors — [`PanelErr`]
    /// vocabulary, the ONE floors law.
    Floors(PanelErr),
    /// Post-condition: the constructed pool overlaps outside the
    /// approval — a construction bug, refused, never shipped.
    Overlap { lanes: Vec<String> },
}

impl fmt::Display for ReserveErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReserveErr::EmptyPool => write!(f, "quorum pool size must be >= 1"),
            ReserveErr::BlankApproval => write!(
                f,
                "reserve pull requires a non-blank approval id — an unauditable \
                 overlap is the silent path F9 killed"
            ),
            ReserveErr::Strict(e) => write!(
                f,
                "strict selection refused (not the degraded day); the reserve \
                 pull never papers over it: {e}"
            ),
            ReserveErr::ReserveExhausted {
                have,
                want,
                skipped_non_live,
                unapproved_live_overlap,
            } => write!(
                f,
                "degraded day exhausted: disjoint live + approved reserve {have} \
                 < wanted {want} (skipped: {skipped_non_live} non-live; unapproved \
                 live overlap: {})",
                unapproved_live_overlap.join(", ")
            ),
            ReserveErr::Floors(e) => write!(f, "quorum pool floors: {e}"),
            ReserveErr::Overlap { lanes } => write!(
                f,
                "F9 reserve construction bug: overlap outside the approval: {} \
                 — refused, never shipped",
                lanes.join(", ")
            ),
        }
    }
}

impl std::error::Error for ReserveErr {}

/// The day's shape under one approval: a single walk classifies every
/// candidate (non-live skipped; live split disjoint / approved reserve /
/// unapproved overlap), then the two competing buckets take the shared
/// deterministic selection order. Private on purpose — the audit and the
/// refusals carry everything the ledger needs.
struct DayShape<'a> {
    disjoint: Vec<&'a Seat>,
    reserve: Vec<&'a Seat>,
    skipped_non_live: usize,
    unapproved_overlap: Vec<String>,
}

fn shape_day<'a>(
    council: &Panel,
    candidates: &'a [Seat],
    approval: &OperatorApproval,
) -> DayShape<'a> {
    let council_lanes: BTreeSet<&str> = council
        .seats
        .iter()
        .map(|ps| ps.seat.lane_id.as_str())
        .collect();
    let approved = approval.approved_set();
    let mut disjoint = Vec::new();
    let mut reserve = Vec::new();
    let mut unapproved: BTreeSet<String> = BTreeSet::new();
    let mut skipped_non_live = 0usize;
    for seat in candidates {
        if !seat.state.selectable() {
            skipped_non_live += 1;
            continue;
        }
        let lane = seat.lane_id.as_str();
        if !council_lanes.contains(lane) {
            disjoint.push(seat);
        } else if approved.contains(lane) {
            reserve.push(seat);
        } else {
            unapproved.insert(seat.lane_id.clone());
        }
    }
    disjoint.sort_by_key(|s| s.selection_key());
    reserve.sort_by_key(|s| s.selection_key());
    DayShape {
        disjoint,
        reserve,
        skipped_non_live,
        unapproved_overlap: unapproved.into_iter().collect(),
    }
}

/// Select a quorum pool for the DEGRADED day under an explicit operator
/// approval — the F9 VETTED RESERVE pull (P3 slice 3). Pure,
/// deterministic, fail-closed. Order of law:
///
/// 1. `size` >= 1 ([`ReserveErr::EmptyPool`]); the approval id must be
///    non-blank ([`ReserveErr::BlankApproval`]) — every pool this
///    selector returns cites that id in its audit;
/// 2. the STRICT law runs FIRST ([`select_quorum_pool`], the ONE strict
///    selector): a healthy day never spends the approval — the strict
///    pool returns with an honest unspent audit; a strict refusal that
///    is NOT the degraded day propagates as [`ReserveErr::Strict`];
/// 3. the degraded day fills from disjoint live seats first (selection
///    order), then pulls the shortfall from the VETTED reserve (the
///    same selection order) — never an unapproved lane, never a
///    non-live lane (F10);
/// 4. disjoint + approved reserve still below `size` = the honest
///    [`ReserveErr::ReserveExhausted`] carrying the unapproved live
///    overlap as the operator's next vetting decision;
/// 5. floors are validated on the FINAL pool (the ONE floors law —
///    a floor-breaking pull is a refusal, never a relaxed floor);
/// 6. the post-condition re-proves every pool-vs-council overlap is an
///    approved lane ([`ReserveErr::Overlap`] = construction bug, never
///    shipped) and the audit's `reserve_lanes` IS that re-proven set —
///    the ledger records the proven fact, not the construction's belief.
///
/// No other code path in the crate can produce an overlapping pool:
/// [`select_quorum_pool`] refuses overlap outright, and this selector
/// admits it only through the named approval, always audited.
pub fn select_quorum_pool_with_reserve(
    council: &Panel,
    candidates: &[Seat],
    size: usize,
    floors: &Floors,
    approval: &OperatorApproval,
) -> Result<ReservePool, ReserveErr> {
    if size == 0 {
        return Err(ReserveErr::EmptyPool);
    }
    if approval.id.trim().is_empty() {
        return Err(ReserveErr::BlankApproval);
    }
    match select_quorum_pool(council, candidates, size, floors) {
        Ok(pool) => {
            let shape = shape_day(council, candidates, approval);
            let disjoint_lanes = pool.seats.iter().map(|s| s.lane_id.clone()).collect();
            return Ok(ReservePool {
                pool,
                audit: ReserveAudit {
                    approval_id: approval.id.clone(),
                    disjoint_lanes,
                    reserve_lanes: Vec::new(),
                    skipped_non_live: shape.skipped_non_live,
                    unapproved_live_overlap: shape.unapproved_overlap,
                },
            });
        }
        // The degraded day — the ONLY strict refusal the reserve pull
        // exists for. Everything else propagates untouched.
        Err(DisjointErr::InsufficientDisjointPool { .. }) => {}
        Err(e) => return Err(ReserveErr::Strict(e)),
    }
    let shape = shape_day(council, candidates, approval);
    let mut disjoint = shape.disjoint;
    let mut reserve = shape.reserve;
    if disjoint.len() + reserve.len() < size {
        return Err(ReserveErr::ReserveExhausted {
            have: disjoint.len() + reserve.len(),
            want: size,
            skipped_non_live: shape.skipped_non_live,
            unapproved_live_overlap: shape.unapproved_overlap,
        });
    }
    disjoint.truncate(size);
    reserve.truncate(size - disjoint.len());
    let pool = QuorumPool {
        seats: disjoint.into_iter().chain(reserve).cloned().collect(),
    };
    pool.check_floors(floors).map_err(ReserveErr::Floors)?;
    // Post-condition BEFORE the audit: the overlap must be within the
    // approval, and the audit's reserve set IS the re-proven overlap.
    let reserve_lanes = match overlap_within_approval(&pool, council, approval) {
        Ok(lanes) => lanes,
        Err(DisjointErr::Overlap { lanes }) => return Err(ReserveErr::Overlap { lanes }),
        // Unreachable by construction (the core only returns Overlap);
        // total match keeps the refusal fail-closed anyway.
        Err(other) => return Err(ReserveErr::Strict(other)),
    };
    let disjoint_lanes = pool
        .seats
        .iter()
        .map(|s| s.lane_id.clone())
        .filter(|lane| !reserve_lanes.contains(lane))
        .collect();
    Ok(ReservePool {
        pool,
        audit: ReserveAudit {
            approval_id: approval.id.clone(),
            disjoint_lanes,
            reserve_lanes,
            skipped_non_live: shape.skipped_non_live,
            unapproved_live_overlap: shape.unapproved_overlap,
        },
    })
}

#[cfg(test)]
#[path = "disjoint_tests.rs"]
mod disjoint_tests;
