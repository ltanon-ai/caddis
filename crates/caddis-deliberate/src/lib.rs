//! caddis-deliberate — COUNCIL + QUORUM DELIBERATION ORGAN, P0 substrate
//! (BUILD-QUEUE r2-organs-rewrite; rung-6 plan
//! state/briefs/caddis-deliberate-organ-plan-2026-08-28.md; quorum verdict
//! quorum-r2-organs-rewrite/VERDICT.md, 2026-08-26, SHIP-WITH-CHANGES).
//!
//! P0 slice 1 = the PURE substrate only: [`Seat`], [`Panel`], [`Floors`]
//! and [`construct_panel`]. Zero I/O, zero daemon knowledge, zero warden
//! writes. Ruling provenance per piece:
//!
//! - **F1/R1** pure crate — the substrate never dispatches, never probes,
//!   never touches a ledger. It classifies DATA and constructs values; the
//!   P3 executor is a later slice, in another module, under warden gates.
//! - **Ruling 5** lane types are [`LaneType::Http`] | [`LaneType::Bridge`] |
//!   [`LaneType::Cli`] — CLI agents are first-class seats, not adapters
//!   bolted on later.
//! - **F10** [`SeatState`] carries the TTL state-machine vocabulary; P0
//!   selection is Live-only. The Expired→Failed/Retired TTL transitions and
//!   re-probe cadence are P1 registry work.
//! - **Floors are DATA** (router F6 precedent): minimum distinct families
//!   and minimum non-Chinese seats are [`Floors`] fields with prior
//!   defaults (2 / 1), never scattered magic numbers. Floor changes are
//!   operator rulings, versioned with the protocol (P2).
//! - **Fail-closed**: an unsatisfiable floor is a [`PanelErr`] refusal,
//!   never a silently degraded panel (F9 killed silent soft-disjoint; the
//!   same law kills silent floor relaxation).
//! - **Free-first** ordering (operator global request 4) is transcribed as
//!   [`CostClass::rank`]: Free < Mid < Premium, ties by `lane_id`.
//! - Selection is deliberately dumb-deterministic (order, take N, validate
//!   floors). Seat-swap tuning — pulling a costlier family-diverse seat in
//!   when the cheapest N violate a floor — is P3 reserve-pool dispatch
//!   work, NOT substrate logic.
//!
//! [`CHINESE_FAMILIES`] is DATA: the table behind the monoculture floor
//! (at least one seat from OUTSIDE the Chinese free-provider cluster). The
//! operator may re-rule it; the floor's meaning never lives in control flow.

use std::collections::BTreeSet;
use std::fmt;
use std::time::SystemTime;

/// Ruling 5: how a seat is reached. CLI lanes are first-class seats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(serde::Serialize, serde::Deserialize))]
pub enum LaneType {
    Http,
    Bridge,
    Cli,
}

/// Cost class for free-first ordering (operator global request 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(serde::Serialize, serde::Deserialize))]
pub enum CostClass {
    Free,
    Mid,
    Premium,
}

impl CostClass {
    /// Selection rank: Free before Mid before Premium (ties break on
    /// `lane_id`, lexicographic — deterministic replay, F1).
    pub fn rank(self) -> u8 {
        match self {
            CostClass::Free => 0,
            CostClass::Mid => 1,
            CostClass::Premium => 2,
        }
    }
}

/// F10 seat lifecycle state. P0: only [`SeatState::Live`] is selectable;
/// TTL-driven transitions are P1 registry work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(serde::Serialize, serde::Deserialize))]
pub enum SeatState {
    Live,
    Expired,
    RateLimited,
    Retired,
    Probing,
    Failed,
}

impl SeatState {
    /// May this seat be placed on a panel right now?
    pub fn selectable(self) -> bool {
        matches!(self, SeatState::Live)
    }
}

/// One deliberation seat (P0 shape; P1 adds registry provenance).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(test, derive(serde::Serialize, serde::Deserialize))]
pub struct Seat {
    pub lane_id: String,
    pub lane_type: LaneType,
    /// Family grouping (router registry Q4 vocabulary). Floors count
    /// DISTINCT families — the monoculture guard.
    pub family: String,
    pub provider: String,
    pub model: String,
    pub cost_class: CostClass,
    pub state: SeatState,
    /// Max concurrent dispatches this seat accepts. P0 carries it as data;
    /// per-provider cap ENFORCEMENT is P3 dispatch work (Ruling 7).
    pub caps: u32,
    /// Last liveness probe, if any. DATA only — P0 never probes (F1).
    pub last_probe: Option<SystemTime>,
}

/// DATA: families inside the Chinese free-provider cluster. The
/// min-non-Chinese floor counts seats OUTSIDE this table. Re-ruling the
/// table is an operator decision, versioned with the protocol (P2).
pub const CHINESE_FAMILIES: &[&str] = &[
    "zai", "zhipu", "bigmodel", "deepseek", "qwen", "alibaba", "moonshot", "kimi", "minimax",
    "01ai", "baichuan", "doubao", "ernie", "baidu", "hunyuan", "tencent",
];

/// Is `family` inside the Chinese-provider cluster? ASCII-case-insensitive
/// on purpose: family ids are human-authored registry rows.
pub fn is_chinese_family(family: &str) -> bool {
    let lower = family.to_ascii_lowercase();
    CHINESE_FAMILIES.contains(&lower.as_str())
}

/// Panel role. [`ROLE_ORDER`] fixes both assignment order and the maximum
/// panel size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(serde::Serialize, serde::Deserialize))]
pub enum Role {
    Chair,
    Synthesist,
    Critic,
    LogicChecker,
}

/// The role ladder, in assignment order.
pub const ROLE_ORDER: &[Role] = &[
    Role::Chair,
    Role::Synthesist,
    Role::Critic,
    Role::LogicChecker,
];

/// Panel construction floors — DATA, never magic numbers (router F6
/// precedent). Defaults are the brief's priors; changes are operator
/// rulings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(serde::Serialize, serde::Deserialize))]
pub struct Floors {
    /// Seats on the panel (roles assigned in [`ROLE_ORDER`] order).
    pub panel_size: usize,
    /// Minimum DISTINCT families on the constructed panel.
    pub min_families: usize,
    /// Minimum seats from OUTSIDE [`CHINESE_FAMILIES`] (monoculture floor).
    pub min_non_chinese: usize,
}

impl Default for Floors {
    fn default() -> Self {
        Floors {
            panel_size: ROLE_ORDER.len(),
            min_families: 2,
            min_non_chinese: 1,
        }
    }
}

/// One seated role.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(test, derive(serde::Serialize, serde::Deserialize))]
pub struct PanelSeat {
    pub role: Role,
    pub seat: Seat,
}

/// A constructed panel: ordered seats with their roles.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(test, derive(serde::Serialize, serde::Deserialize))]
pub struct Panel {
    pub seats: Vec<PanelSeat>,
}

impl Panel {
    /// Distinct family count on this panel (floor input).
    pub fn family_count(&self) -> usize {
        self.seats
            .iter()
            .map(|ps| ps.seat.family.as_str())
            .collect::<BTreeSet<_>>()
            .len()
    }

    /// Seats from OUTSIDE the Chinese cluster (floor input).
    pub fn non_chinese_count(&self) -> usize {
        self.seats
            .iter()
            .filter(|ps| !is_chinese_family(&ps.seat.family))
            .count()
    }
}

/// Panel construction refusals. Fail-closed: every floor violation is a
/// REFUSAL, never a degraded panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PanelErr {
    /// `floors.panel_size` is 0 or above [`ROLE_ORDER`]'s length — a
    /// malformed floor, not a seat problem.
    PanelSizeOutOfRange {
        given: usize,
        max: usize,
    },
    NotEnoughLiveSeats {
        have: usize,
        need: usize,
    },
    FamiliesFloor {
        have: usize,
        want: usize,
    },
    NonChineseFloor {
        have: usize,
        want: usize,
    },
}

impl fmt::Display for PanelErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PanelErr::PanelSizeOutOfRange { given, max } => {
                write!(f, "panel_size {given} out of range 1..={max} (ROLE_ORDER)")
            }
            PanelErr::NotEnoughLiveSeats { have, need } => {
                write!(f, "not enough live seats: have {have}, need {need}")
            }
            PanelErr::FamiliesFloor { have, want } => write!(
                f,
                "families floor violated: have {have}, want {want} distinct families"
            ),
            PanelErr::NonChineseFloor { have, want } => write!(
                f,
                "non-Chinese floor violated: have {have}, want {want} seats outside CHINESE_FAMILIES"
            ),
        }
    }
}

impl std::error::Error for PanelErr {}

/// Construct a panel from a seat set. Pure, deterministic, fail-closed.
///
/// Policy (deliberately minimal — F1 substrate):
/// 1. only [`SeatState::Live`] seats compete (F10);
/// 2. order Free < Mid < Premium, ties by `lane_id` (free-first, global
///    request 4);
/// 3. take the first `floors.panel_size` seats;
/// 4. validate floors on the CONSTRUCTED panel — distinct families first,
///    then non-Chinese (fixed refusal order, deterministic replay);
/// 5. assign roles in [`ROLE_ORDER`] order.
///
/// Seat-swap tuning to satisfy floors from a wider pool is P3 reserve-pool
/// dispatch work, not here (plan P3).
pub fn construct_panel(candidates: &[Seat], floors: &Floors) -> Result<Panel, PanelErr> {
    let max = ROLE_ORDER.len();
    if floors.panel_size == 0 || floors.panel_size > max {
        return Err(PanelErr::PanelSizeOutOfRange {
            given: floors.panel_size,
            max,
        });
    }
    let mut live: Vec<&Seat> = candidates.iter().filter(|s| s.state.selectable()).collect();
    if live.len() < floors.panel_size {
        return Err(PanelErr::NotEnoughLiveSeats {
            have: live.len(),
            need: floors.panel_size,
        });
    }
    live.sort_by(|a, b| {
        a.cost_class
            .rank()
            .cmp(&b.cost_class.rank())
            .then_with(|| a.lane_id.cmp(&b.lane_id))
    });
    let panel = Panel {
        seats: live[..floors.panel_size]
            .iter()
            .zip(ROLE_ORDER)
            .map(|(seat, role)| PanelSeat {
                role: *role,
                seat: (*seat).clone(),
            })
            .collect(),
    };
    let families = panel.family_count();
    if families < floors.min_families {
        return Err(PanelErr::FamiliesFloor {
            have: families,
            want: floors.min_families,
        });
    }
    let non_chinese = panel.non_chinese_count();
    if non_chinese < floors.min_non_chinese {
        return Err(PanelErr::NonChineseFloor {
            have: non_chinese,
            want: floors.min_non_chinese,
        });
    }
    Ok(panel)
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod lib_tests;
