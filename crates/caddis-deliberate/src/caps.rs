//! caps.rs — P1 slice 2: per-provider concurrency caps (Ruling 7) and the
//! pure dispatch-order planner that proves a capped provider SERIALIZES
//! (plan P1 Done-When: "a capped provider serializes in dispatch order").
//!
//! Laws transcribed (brief RULING-7 NOTE, operator 2026-08-26):
//! - "Provider registry carries concurrency caps per provider.
//!   ollama/ollama-cloud: max 1 concurrent call (hard ceiling 2) —
//!   parallel calls kill the APIs."
//! - Dispatch (F4) enforces caps BEFORE invoking: seats sharing a capped
//!   provider serialize automatically, other providers proceed.
//!
//! This module is the LAW + the pure planner; the enforcing executor is
//! P3 dispatch work. The planner answers the only question P1 must prove:
//! given a wanted dispatch list and the registry, which requests may run
//! in the SAME concurrent wave — a capped provider's requests never share
//! a wave.
//!
//! - **DATA, not control flow** (F6 precedent): the ruled-caps table is a
//!   const; a new ruling edits the table (slice 3 edit path), never logic.
//! - **Fail-closed**: an unknown seat id or a seat whose provider card is
//!   missing is a REFUSAL — the planner never guesses a provider or a cap.
//! - **Deterministic**: input order is preserved; waves are built greedily
//!   in input order (same input ⇒ same plan, deterministic replay).

use crate::registry::{Registry, SeatCard};

/// Ruling 7 as DATA: `(provider id, ruled concurrent cap, hard ceiling)`.
/// The ruled cap is what the collector seeds and what the law accepts;
/// the hard ceiling is the MAXIMUM any later ruling may raise it to
/// (`check_provider_caps` refuses above it — "parallel calls kill the
/// APIs"). Providers not in the table carry the F4 default of 1
/// (serialized-by-default) with no specific ceiling.
pub const RULED_CAPS: &[(&str, u32, u32)] = &[("ollama", 1, 2), ("ollama-cloud", 1, 2)];

/// The F4 default for providers outside [`RULED_CAPS`]:
/// serialized-by-default (the wedge lesson — parallel lanes only behind a
/// per-lane circuit-breaker flag, which is P3 work).
pub const DEFAULT_CAPS: u32 = 1;

/// The ruled concurrent cap for a provider (table value, else
/// [`DEFAULT_CAPS`]). The collector seeds exactly this.
pub fn ruled_caps(provider: &str) -> u32 {
    RULED_CAPS
        .iter()
        .find(|(id, _, _)| *id == provider)
        .map(|&(_, caps, _)| caps)
        .unwrap_or(DEFAULT_CAPS)
}

/// The hard ceiling for a provider, if Ruling 7 names one (`None` = only
/// the `>= 1` law applies; raising still rides the slice-3 edit path).
pub fn hard_ceiling(provider: &str) -> Option<u32> {
    RULED_CAPS
        .iter()
        .find(|(id, _, _)| *id == provider)
        .map(|&(_, _, ceil)| ceil)
}

/// Cap-law refusals. Fail-closed: every violation is a refusal, never a
/// silently clamped value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapsErr {
    ZeroCaps {
        provider: String,
    },
    AboveHardCeiling {
        provider: String,
        caps: u32,
        ceiling: u32,
    },
    SeatAboveProvider {
        seat_id: String,
        seat_caps: u32,
        provider: String,
        provider_caps: u32,
    },
    UnknownSeat {
        seat_id: String,
    },
    SeatMissingProvider {
        seat_id: String,
        provider: String,
    },
}

impl std::fmt::Display for CapsErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CapsErr::ZeroCaps { provider } => {
                write!(f, "provider {provider:?}: caps must be >= 1 (a 0-cap provider can never dispatch)")
            }
            CapsErr::AboveHardCeiling { provider, caps, ceiling } => write!(
                f,
                "provider {provider:?}: caps {caps} exceeds the Ruling-7 hard ceiling {ceiling} (parallel calls kill the APIs)"
            ),
            CapsErr::UnknownSeat { seat_id } => {
                write!(f, "unknown seat {seat_id:?} — the planner never guesses a lane")
            }
            CapsErr::SeatAboveProvider { seat_id, seat_caps, provider, provider_caps } => write!(
                f,
                "seat {seat_id:?}: caps {seat_caps} exceeds provider {provider:?} caps {provider_caps} — registry drift, refuse loudly"
            ),
            CapsErr::SeatMissingProvider { seat_id, provider } => write!(
                f,
                "seat {seat_id:?} references provider {provider:?} with no provider card — registry is inconsistent"
            ),
        }
    }
}

/// The Ruling-7 law over one provider card row: caps >= 1 always; caps
/// <= the hard ceiling where the ruling names one. Called by the slice-3
/// edit path on every propose (and by tests on the collector seed).
pub fn check_provider_caps(provider: &str, caps: u32) -> Result<(), CapsErr> {
    if caps == 0 {
        return Err(CapsErr::ZeroCaps {
            provider: provider.into(),
        });
    }
    if let Some(ceiling) = hard_ceiling(provider) {
        if caps > ceiling {
            return Err(CapsErr::AboveHardCeiling {
                provider: provider.into(),
                caps,
                ceiling,
            });
        }
    }
    Ok(())
}

/// Validate the whole registry against the cap law: every provider row,
/// every seat row (seat caps may not exceed its provider's caps — the
/// effective cap is a MIN, so a seat row above its provider is drift the
/// law reports loudly instead of clamping silently).
pub fn validate_registry(reg: &Registry) -> Result<(), CapsErr> {
    for (id, p) in &reg.providers {
        check_provider_caps(id, p.caps)?;
    }
    for (id, s) in &reg.seats {
        let p = reg
            .providers
            .get(&s.provider)
            .ok_or_else(|| CapsErr::SeatMissingProvider {
                seat_id: id.clone(),
                provider: s.provider.clone(),
            })?;
        if s.caps > p.caps {
            return Err(CapsErr::SeatAboveProvider {
                seat_id: id.clone(),
                seat_caps: s.caps,
                provider: s.provider.clone(),
                provider_caps: p.caps,
            });
        }
    }
    Ok(())
}

/// The cap a seat's dispatches are actually limited by: the MIN of the
/// seat row and its provider row (both are >= 1 in a validated registry).
pub fn effective_caps(seat: &SeatCard, provider: &crate::registry::ProviderCard) -> u32 {
    seat.caps.min(provider.caps)
}

/// The pure dispatch planner (P1 Done-When proof). `wanted` is the
/// dispatch list in DISPATCH ORDER (caller's order is the law). Returns
/// WAVES: requests in the same wave may run concurrently; a request in
/// wave N+1 starts only after wave N completes. Within one wave no
/// provider's concurrent usage exceeds its effective cap — a capped
/// provider SERIALIZES, everything else proceeds.
///
/// Greedy in input order: a request joins the open wave while its
/// provider has headroom, else the wave closes and the request opens the
/// next one. Same input ⇒ same waves (deterministic replay). Unknown
/// seats or missing provider cards refuse the WHOLE plan (fail-closed —
/// a half-planned dispatch set is a defect, not progress).
pub fn plan_batches(wanted: &[&str], reg: &Registry) -> Result<Vec<Vec<String>>, CapsErr> {
    let mut waves: Vec<Vec<String>> = Vec::new();
    // Open-wave usage: provider id -> seats placed in the CURRENT wave.
    let mut usage: std::collections::BTreeMap<String, u32> = std::collections::BTreeMap::new();

    for seat_id in wanted {
        let seat = reg
            .seats
            .get(*seat_id)
            .ok_or_else(|| CapsErr::UnknownSeat {
                seat_id: seat_id.to_string(),
            })?;
        let provider =
            reg.providers
                .get(&seat.provider)
                .ok_or_else(|| CapsErr::SeatMissingProvider {
                    seat_id: seat_id.to_string(),
                    provider: seat.provider.clone(),
                })?;
        let cap = effective_caps(seat, provider).max(1);

        let used = usage.entry(seat.provider.clone()).or_insert(0);
        if *used < cap {
            *used += 1;
            match waves.last_mut() {
                Some(wave) => wave.push(seat_id.to_string()),
                None => waves.push(vec![seat_id.to_string()]),
            }
        } else {
            // Provider at cap in the open wave: close it, open the next.
            usage.clear();
            usage.insert(seat.provider.clone(), 1);
            waves.push(vec![seat_id.to_string()]);
        }
    }
    Ok(waves)
}

#[cfg(test)]
#[path = "caps_tests.rs"]
mod tests;
