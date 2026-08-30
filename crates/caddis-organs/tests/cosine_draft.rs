//! cosine_draft.rs — CARD-0249 RED-first. The DRAFT layer of the
//! speculative-retrieval / verified-by-use loop.
//!
//! The math is a separate organ from the embedding (which is a HOST
//! act over the estate lanes). This module only ranks the vectors it
//! is handed. The cosine is the cheap DRAFT; the judge (CARD-0245
//! attention replay) is what confirms whether the host actually USED
//! anything this draft proposed.
//!
//! Two laws pinned here from CARD-0249 §EXECUTION:
//!
//! 1. Total on any input — zero vectors, mismatched lengths, NaN
//!    inputs all return 0.0. NaN never escapes; the draft must not
//!    poison its callers.
//! 2. Deterministic tie-break by candidate index — equal cosine scores
//!    resolve to the LOWER index first. One sort law.
//!
//! Today the test cannot even compile — the module and its two
//! functions do not exist. That is the RED: the type and reader do
//! not exist, so neither case can be expressed.

use caddis_organs::cosine_draft::{cosine, top_k_by_cosine};

/// RED: the total-on-any-input law. Zero vectors collapse to 0.0,
/// never NaN; mismatched lengths return 0.0; an input containing
/// NaN returns 0.0. The draft never poisons its callers.
#[test]
fn cosine_is_total_zero_input_returns_zero_not_nan() {
    let zero = vec![0.0f32; 4];
    let any = vec![1.0f32, 2.0, 3.0, 4.0];

    // Zero norm collapses to 0.0 (not NaN, not infinity).
    assert_eq!(cosine(&zero, &any), 0.0);
    assert_eq!(cosine(&any, &zero), 0.0);
    assert_eq!(cosine(&zero, &zero), 0.0);

    // NaN-bearing input collapses to 0.0 — the draft never poisons.
    let nan = vec![f32::NAN, 1.0, 2.0, 3.0];
    let s = cosine(&nan, &any);
    assert!(!s.is_nan(), "NaN must not escape; got {s}");
    assert_eq!(s, 0.0);
    let s = cosine(&any, &nan);
    assert!(!s.is_nan(), "NaN must not escape; got {s}");
    assert_eq!(s, 0.0);
}

/// RED: the total-on-any-input law continued. Mismatched length
/// returns 0.0 (and never panics, never NaN). The caller cannot be
/// trusted to filter.
#[test]
fn cosine_mismatched_length_returns_zero() {
    let a = vec![1.0f32, 2.0, 3.0];
    let b = vec![1.0f32, 2.0];
    let s = cosine(&a, &b);
    assert!(!s.is_nan(), "mismatched length must not be NaN");
    assert_eq!(s, 0.0);
}

/// RED: the well-known anchors. Identical vectors score 1.0; opposite
/// vectors score -1.0; orthogonal vectors score 0.0 (with the usual
/// float tolerance). These anchor the law across renames / reorderings.
#[test]
fn cosine_anchors_identical_opposite_orthogonal() {
    let v = vec![1.0f32, 0.0, 0.0];
    let same = vec![1.0f32, 0.0, 0.0];
    let opp = vec![-1.0f32, 0.0, 0.0];
    let orth = vec![0.0f32, 1.0, 0.0];

    let eps = 1e-6;
    assert!((cosine(&v, &same) - 1.0).abs() < eps, "identical -> 1.0");
    assert!((cosine(&v, &opp) - -1.0).abs() < eps, "opposite -> -1.0");
    assert!((cosine(&v, &orth) - 0.0).abs() < eps, "orthogonal -> 0.0");
}

/// RED: deterministic tie-break by candidate index. Three candidates
/// share an identical cosine score; the lower index must win. The
/// sort law is the law — a non-deterministic order here breaks every
/// ledger diff and every "same query, same answer" property the
/// host expects.
#[test]
fn top_k_tie_break_is_deterministic_by_index() {
    let query = vec![1.0f32, 0.0, 0.0];
    // All three are equally aligned with the query (cosine = 1.0).
    let candidates: Vec<Vec<f32>> = vec![
        vec![2.0f32, 0.0, 0.0], // index 0
        vec![5.0f32, 0.0, 0.0], // index 1
        vec![0.5f32, 0.0, 0.0], // index 2
    ];
    let k = 2;

    let top = top_k_by_cosine(&query, &candidates, k);

    assert_eq!(top.len(), 2, "top-k must return k entries");
    // Lower index wins the tie.
    assert_eq!(top[0].0, 0, "lower index must rank first on a tie");
    assert_eq!(top[1].0, 1, "next-lowest index on the same tie");
    // The score is the cosine — all 1.0 here.
    assert!((top[0].1 - 1.0).abs() < 1e-6);
    assert!((top[1].1 - 1.0).abs() < 1e-6);
}

/// RED: top-k is descending by score, with the tie-break law applied
/// within equal-score blocks. A clear winner is first, the rest
/// follow in cosine order; equal scores break by index.
#[test]
fn top_k_orders_by_score_then_index() {
    let query = vec![1.0f32, 0.0, 0.0];
    // Cosines: 1.0, 0.5, 1.0 (ties between idx 0 and idx 2), -0.5
    let candidates: Vec<Vec<f32>> = vec![
        vec![3.0f32, 0.0, 0.0],  // idx 0: cosine 1.0
        vec![1.0f32, 1.0, 0.0],  // idx 1: cosine 0.5
        vec![2.0f32, 0.0, 0.0],  // idx 2: cosine 1.0 (tie with idx 0)
        vec![-1.0f32, 1.0, 0.0], // idx 3: cosine -0.5
    ];
    let k = 3;

    let top = top_k_by_cosine(&query, &candidates, k);

    assert_eq!(top.len(), 3);
    // The 1.0-cosine tie resolves by index: idx 0 < idx 2.
    assert_eq!(top[0].0, 0, "first tie goes to lower index");
    assert_eq!(top[1].0, 2, "second tie goes to higher index");
    // Then idx 1 (cosine 0.5).
    assert_eq!(top[2].0, 1, "next-highest cosine follows the tie block");
}

/// RED: top-k is total on any input. k == 0 -> empty; k larger than
/// the candidate count -> all candidates; zero-norm candidates rank
/// 0.0 alongside the tie-break law; NaN-bearing candidates never
/// poison the result.
#[test]
fn top_k_is_total_on_edge_inputs() {
    let query = vec![1.0f32, 0.0, 0.0];

    // k = 0 -> empty.
    let candidates = vec![vec![1.0f32, 0.0, 0.0]];
    assert!(top_k_by_cosine(&query, &candidates, 0).is_empty());

    // k > candidate count -> all candidates, in score-then-index order.
    let candidates = vec![
        vec![1.0f32, 0.0, 0.0],  // idx 0
        vec![0.0f32, 1.0, 0.0],  // idx 1, orthogonal
        vec![-1.0f32, 0.0, 0.0], // idx 2, opposite
    ];
    let top = top_k_by_cosine(&query, &candidates, 99);
    assert_eq!(top.len(), 3);
    assert_eq!(top[0].0, 0);
    assert_eq!(top[1].0, 1);
    assert_eq!(top[2].0, 2);

    // NaN-bearing candidate scores 0.0; a normal candidate of equal
    // index-precedence should outrank it.
    let candidates = vec![
        vec![f32::NAN, 0.0, 0.0], // idx 0, NaN -> 0.0
        vec![1.0f32, 0.0, 0.0],   // idx 1, cosine 1.0
    ];
    let top = top_k_by_cosine(&query, &candidates, 2);
    assert_eq!(top.len(), 2);
    assert_eq!(top[0].0, 1, "real cosine must outrank NaN-bearing");
    assert!(!top[0].1.is_nan(), "NaN must not escape via top-k");
    assert!(!top[1].1.is_nan(), "NaN must not escape via top-k");
}
