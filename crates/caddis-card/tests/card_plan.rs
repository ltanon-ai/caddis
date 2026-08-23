//! card_plan.rs — BC1: the PLAN card class and its gates (quorum-folded).
//!
//! Rulings baked in: PLAN never passes validate_strict (different oracle —
//! the plan's truth is intent review, not execution shape); the CHILDREN
//! section carries ids+order+paths/symbols; the REVIEW receipt proves a
//! review HAPPENED (presence+shape, never that it was right); parentage
//! reuses the SPLIT parent/of/order encoding — a depth-1 child's SPLIT
//! parses against the plan id; children paths are pairwise disjoint here,
//! and repo-reality checks (existence, greppability) live in the walker
//! where the repo is — the crate validates structure only.

use caddis_card::Card;

fn plan_card(children: &str, review: &str) -> String {
    format!(
        "---\nid: PLAN-T\nclass: plan\nowner: t\n---\n\n# T\n\n# Done-When\n\n\
         - children exist and validate\n\n# RED-TEST\n\n- children missing today\n\n\
         # CHILDREN\n\n{children}\n\n# REVIEW\n\n{review}\n"
    )
}

const GOOD_CHILDREN: &str = "- id: CARD-A\n  order: 1\n  paths: a.py\n  symbols: foo\n\
                            - id: CARD-B\n  order: 2\n  paths: b.py\n  symbols: bar";

const GOOD_REVIEW: &str = "reviewer: strong-lane\nverdict: accepted\n\
                           checks: A targets the right seam; B matches the contract";

#[test]
fn a_well_formed_plan_validates() {
    let card = Card::parse(&plan_card(GOOD_CHILDREN, GOOD_REVIEW)).unwrap();
    let plan = card.validate_plan().expect("plan ok");
    assert_eq!(plan.children.len(), 2);
    assert_eq!(plan.children[0].id, "CARD-A");
    assert_eq!((plan.children[0].order, plan.children[1].order), (1, 2));
    assert_eq!(plan.review.verdict, "accepted");
    assert_eq!(plan.review.reviewer, "strong-lane");
}

#[test]
fn a_plan_must_never_pass_validate_strict() {
    // Different oracle: strict demands EXECUTION; a plan card has none and
    // must be rejected there — routing plans through strict would demand
    // execution anchors of a document whose job is decomposition.
    let card = Card::parse(&plan_card(GOOD_CHILDREN, GOOD_REVIEW)).unwrap();
    assert!(card.validate_strict().is_err());
}

#[test]
fn missing_review_receipt_is_rejected() {
    let text = plan_card(GOOD_CHILDREN, "").replace("# REVIEW\n\n\n", "");
    let card = Card::parse(&text).unwrap();
    assert!(card.validate_plan().is_err(), "no receipt, no plan");
}

#[test]
fn receipt_with_unknown_verdict_or_empty_checks_is_rejected() {
    let bad = "reviewer: strong-lane\nverdict: maybe\nchecks:";
    let card = Card::parse(&plan_card(GOOD_CHILDREN, bad)).unwrap();
    assert!(card.validate_plan().is_err());
    let empty = "reviewer: strong-lane\nverdict: accepted\nchecks:  \n";
    let card = Card::parse(&plan_card(GOOD_CHILDREN, empty)).unwrap();
    assert!(
        card.validate_plan().is_err(),
        "an empty checks line is a checkbox"
    );
}

#[test]
fn duplicate_child_ids_or_broken_orders_are_rejected() {
    let dup = "- id: CARD-A\n  order: 1\n  paths: a.py\n\
                - id: CARD-A\n  order: 2\n  paths: b.py";
    let card = Card::parse(&plan_card(dup, GOOD_REVIEW)).unwrap();
    assert!(card.validate_plan().is_err());
    let order = "- id: CARD-A\n  order: 2\n  paths: a.py\n\
                 - id: CARD-B\n  order: 1\n  paths: b.py";
    let card = Card::parse(&plan_card(order, GOOD_REVIEW)).unwrap();
    assert!(card.validate_plan().is_err(), "orders must be 1..N");
}

#[test]
fn overlapping_child_paths_are_rejected_here() {
    let overlap = "- id: CARD-A\n  order: 1\n  paths: a.py, shared.py\n  symbols: foo\n\
                   - id: CARD-B\n  order: 2\n  paths: shared.py\n  symbols: bar";
    let card = Card::parse(&plan_card(overlap, GOOD_REVIEW)).unwrap();
    assert!(
        card.validate_plan().is_err(),
        "one path, one child — overlap needs CONTINUATION, never duplication"
    );
}

#[test]
fn depth1_plan_parentage_parses_like_split() {
    // One parentage encoding: the child carries a SPLIT marker whose parent
    // is the plan's id; Split::parse must read it exactly as for exec cards.
    let child = "---\nid: CARD-A\nclass: fix\nowner: t\n---\n\n# A\n\n\
                 # Done-When\n\n- x\n\n# RED-TEST\n\n- y\n\n\
                 # EXECUTION\n\nlevel: L1\nblast: 1\nclaims-forbidden: true\n\
                 anchors:\n  - path: a.py\n    content: |\n      x\n\
                 allowlist:\n  - edit a.py\n\n# SPLIT\n\nparent: PLAN-T\norder: 1\nof: 2\n";
    let card = Card::parse(child).unwrap();
    let split = card.split().expect("split parses for a plan child");
    assert_eq!(split.parent, "PLAN-T");
    assert_eq!((split.order, split.of), (1, 2));
}
