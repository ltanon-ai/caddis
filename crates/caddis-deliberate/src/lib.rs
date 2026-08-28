//! caddis-deliberate — COUNCIL + QUORUM DELIBERATION ORGAN (P0 substrate + P1 registry)
//! (BUILD-QUEUE r2-organs-rewrite; rung-6 plan
//! state/briefs/caddis-deliberate-organ-plan-2026-08-28.md; quorum verdict
//! quorum-r2-organs-rewrite/VERDICT.md, 2026-08-26, SHIP-WITH-CHANGES).
//!
//! P0 slice 1 = the PURE substrate only: [`Seat`], [`Panel`], [`Floors`]
//! and [`construct_panel`]. P0 slice 2 = [`protocol`]: the versioned
//! [`protocol::Protocol`] with its F3 PIN (sha256 over canonical bytes,
//! [`protocol::Convening`] storing it at open time, `verify_pin`
//! rejecting any later drift) and [`protocol::Verdict`] provenance
//! (transport-served model, never self-report). P0 slice 3 = [`disjoint`]:
//! the F9 STRICT quorum-pool law — selection with the disjoint filter,
//! the zero-overlap proof, the degraded-day honest refusal — with floors
//! as DATA proven by a day table. Zero I/O,
//! zero daemon knowledge, zero warden writes.
//! P1 slice 1 = [`registry`] + [`collector`]: the seat registry as an
//! APPEND-ONLY CARD STREAM (`seats.jsonl`, flat one-object-per-line
//! cards, exact field law) with a sha256-verified CACHED JSON VIEW
//! re-synced per row (F2), plus the seed collector from the desktop
//! models.json provider catalog (13 providers) — deterministic bytes, no
//! secrets, no taste: cost classes derive from measured cost, states
//! seed `probing`, and anything judgment-shaped stays an operator
//! ruling through the P1-slice-3 edit path.
//! P1 slice 2 = [`caps`] + [`ttl`]: the Ruling-7 per-provider
//! concurrency law (ollama/ollama-cloud = 1 concurrent, hard ceiling 2)
//! with a pure dispatch planner that SERIALIZES a capped provider's
//! requests into separate waves, and the F10 TTL state machine — the
//! Expired quota-cooldown → Failed (renewable) / Retired (not), per-state
//! re-probe cadence as DATA, renewable free lanes never auto-retire.
//! P1 slice 3 = [`edits`]: the registry EDIT PATH — warden-gated
//! propose→operator-confirm (F2). Durable pending proposals in an
//! append-only journal (MV13), prior16 optimistic concurrency (router
//! author law), no-op refusals, and THE WARDEN GATE: confirm requires an
//! ACTIVE warden card for the confirming actor, derived READ-ONLY from
//! the warden ledger through `caddis_warden::card_state` (the ONE
//! card-state law; this crate's single workspace path dep). Crash order
//! is STREAM FIRST, JOURNAL LAST — an orphan pending never double-applies.
//! P2 slice 1 = [`council`]: the COUNCIL protocol card v1 — the seven
//! canonical stages as DATA with mechanics per stage: F1 per-convening
//! warden gate-card (the same read-only `active_for` law as [`edits`]),
//! serialized dispatch PLANNING via [`caps::plan_batches`], fail-closed
//! collect, integration as a disagreement MAP (never averaging), the
//! verdict table, its flat exact-field ledger row, and the F11
//! mid-flight-edit → pause → version-bump → re-dispatch choreography
//! (archived original, never a hard abort). The F3 pin is [`protocol`]'s
//! — reused, never a second pin.
//! P2 slice 2 = [`quorum`]: the QUORUM protocol card v1 — the ladder's
//! deciding body. Pool selection IS [`disjoint::select_quorum_pool`]
//! (F9 STRICT reuse — this slice also WIRES the P0-slice-3 `disjoint`
//! module into the crate; it was authored but never registered, so its
//! law and tests had never compiled). Floor 2/3 as the strict majority
//! of the FULL pool, derived never constant; a missing seat is
//! tolerated only while the floor holds — the ruling carries a literal
//! `*`, `degraded = true`, and the missing lanes in the VERDICT.md
//! artifact and the ledger row; below the floor or split = refusal. The
//! gate, the pin, the clustering, and the verdict digest are the
//! council card's ONE laws, reused — never second copies.
//! P3 slice 1 = [`executor`] + [`sessions`]: the DISPATCH ENGINE and the
//! R4 session cards. The executor runs a council Convening end-to-end —
//! waves from the ONE planner ([`caps::plan_batches`] via
//! [`council::dispatch_plan`]), F3 pin re-checked before every wave
//! through the caller's card snapshot (Moved → the session rides back
//! for the F11 choreography), legs of one wave concurrent while the wave
//! joins before the next (serialized-by-default is the registry's own
//! caps law — a raised, warden-gated cap is the only parallel door),
//! collect → integrate → verdict → ledger reused as the ONE laws, and
//! every answered leg lands a `class: session` row (open before the
//! first leg, usage per answered seat, close with the verdict-digest
//! link — the model-visibility feed's one mechanism).
//! P4 slice 3 = [`seed`]: the F13 SIGNED SEED ARTIFACT + verify-gate.
//! Export signs the home's stream (HMAC-SHA256 over a born-once
//! `seed.key` minted beside it, the caddis-router warden law vendored);
//! verify is the supply-chain gate (strict shape + stream digest + rows
//! + fingerprint + signature, findings name the broken law); restore
//! CONSTRUCTS a home on any machine ONLY after the gate is clean —
//! tampered seed = refused with nothing written, a diverged target is
//! never clobbered. Honest boundary: symmetric attestation, the key
//! travels with the owner (`--key`), never inside the artifact.
//! - **F1/R1** pure crate — the substrate never dispatches, never probes,
//!   never touches a ledger. It classifies DATA and constructs values; the
//!   P3 executor is a later slice, in another module, under warden gates.
//! - **Ruling 5** lane types are [`LaneType::Http`] | [`LaneType::Bridge`] |
//!   [`LaneType::Cli`] — CLI agents are first-class seats, not adapters
//!   bolted on later.
//! - **F10** [`SeatState`] carries the TTL state-machine vocabulary; P0
//!   selection is Live-only. The Expired→Failed/Retired TTL transitions
//!   and re-probe cadence are P1 slice 2 ([`ttl`]) — Live-only selection
//!   is the law every state honors.
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

pub mod caps;
pub mod collector;
pub mod council;
pub mod disjoint;
pub mod edits;
pub mod executor;
pub mod json;
pub mod protocol;
pub mod quorum;
pub mod registry;
pub mod seed;
pub mod sessions;
pub mod sha256;
pub mod ttl;

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

impl Seat {
    /// The ONE selection order: free-first ([`CostClass::rank`]), ties
    /// by `lane_id` (deterministic replay, F1). Shared by council panel
    /// construction ([`construct_panel`]) and quorum-pool selection
    /// ([`disjoint::select_quorum_pool`]) — two bodies, one law.
    pub fn selection_key(&self) -> (u8, &str) {
        (self.cost_class.rank(), self.lane_id.as_str())
    }
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

    /// Validate the PANEL-level floors (distinct families first, then
    /// non-Chinese — fixed refusal order, deterministic replay) on this
    /// constructed panel. The same law [`construct_panel`] applies at
    /// construction; exposed so a [`crate::protocol::Convening`] can
    /// re-prove its seated panel against the floors of the protocol it
    /// pins (slice 2) and slice-3 data-driven tests can drive it directly.
    /// `panel_size` is deliberately NOT checked here: it is a
    /// construction-time constraint (take exactly N), not a panel shape.
    pub fn check_floors(&self, floors: &Floors) -> Result<(), PanelErr> {
        check_floor_laws(self.family_count(), self.non_chinese_count(), floors)
    }
}

/// The floors law over ANY seated collection — council panel or quorum
/// pool: distinct families first, then non-Chinese (fixed refusal order,
/// deterministic replay). One law, one error language; the counts come
/// from the caller's counting methods.
pub(crate) fn check_floor_laws(
    families: usize,
    non_chinese: usize,
    floors: &Floors,
) -> Result<(), PanelErr> {
    if families < floors.min_families {
        return Err(PanelErr::FamiliesFloor {
            have: families,
            want: floors.min_families,
        });
    }
    if non_chinese < floors.min_non_chinese {
        return Err(PanelErr::NonChineseFloor {
            have: non_chinese,
            want: floors.min_non_chinese,
        });
    }
    Ok(())
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
    live.sort_by_key(|s| s.selection_key());
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
    panel.check_floors(floors)?;
    Ok(panel)
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod lib_tests;
