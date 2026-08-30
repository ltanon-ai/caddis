//! cosine_draft.rs — CARD-0249. The DRAFT layer of the
//! speculative-retrieval / verified-by-use loop.
//!
//! Pure math, zero GPU dependency. The organ NEVER embeds; the host
//! hands it vectors and it ranks them. The cosine (a·b / ‖a‖‖b‖) is
//! the cheap DRAFT that proposes what to inject; the expensive
//! judge — did the model actually USE it (CARD-0245 attention
//! replay) — verifies. Speculation at microseconds, verification
//! batched and rare; the MTP asymmetry applied to RETRIEVAL.
//!
//! Two laws pinned by `tests/cosine_draft.rs`:
//!
//! 1. Total on any input. Zero vectors, mismatched lengths, NaN
//!    inputs all collapse to 0.0. NaN never escapes; the draft must
//!    not poison its callers.
//! 2. Deterministic tie-break by candidate index. Equal cosine
//!    scores resolve to the lower index first. One sort law — the
//!    host's "same query, same answer" property depends on it.

/// Cosine similarity `a·b / (‖a‖·‖b‖)`. Total on any input:
/// mismatched lengths, zero vectors, and any NaN-bearing input
/// collapse to `0.0`. NaN never escapes.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }

    let mut dot = 0.0f64;
    let mut na = 0.0f64;
    let mut nb = 0.0f64;
    let mut any_nan = false;

    for (&x, &y) in a.iter().zip(b.iter()) {
        if x.is_nan() || y.is_nan() {
            any_nan = true;
            break;
        }
        let xf = x as f64;
        let yf = y as f64;
        dot += xf * yf;
        na += xf * xf;
        nb += yf * yf;
    }

    if any_nan || na == 0.0 || nb == 0.0 {
        return 0.0;
    }

    let score = dot / (na.sqrt() * nb.sqrt());
    if score.is_nan() {
        return 0.0;
    }
    score as f32
}

/// Top-k candidates by cosine similarity to `query`. Returns up to
/// `k` `(candidate_index, score)` pairs ordered by score descending,
/// with the deterministic tie-break: equal scores resolve to the
/// lower candidate index first.
///
/// Total on any input: empty query / empty candidates / `k == 0`
/// return empty; `k` larger than the candidate count returns every
/// candidate; NaN-bearing and zero-norm candidates score `0.0` and
/// rank alongside the tie-break law — NaN never escapes.
pub fn top_k_by_cosine(query: &[f32], candidates: &[Vec<f32>], k: usize) -> Vec<(usize, f32)> {
    if k == 0 || candidates.is_empty() {
        return Vec::new();
    }

    // Score every candidate. Index is the tie-break key — preserved
    // here so the sort is the one place the order is decided.
    let mut scored: Vec<(usize, f32)> = candidates
        .iter()
        .enumerate()
        .map(|(idx, cand)| (idx, cosine(query, cand)))
        .collect();

    // One sort law: descending by score, ascending by index on ties.
    // `f32` is not `Ord`; a partial_cmp that collapses NaN into a
    // defined total is unnecessary — `cosine` already guarantees no
    // NaN escapes, so `unwrap_or(Equal)` is purely defensive.
    scored.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });

    scored.truncate(k);
    scored
}
