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
//!   the P3 executor (vetted reserve pool, operator-approved overlap,
//!   logged). Reserve-pool selection is P3 dispatch work, never
//!   substrate magic (F9).
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

#[cfg(test)]
#[path = "disjoint_tests.rs"]
mod disjoint_tests;
