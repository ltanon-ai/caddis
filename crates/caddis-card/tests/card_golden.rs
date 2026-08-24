//! card_golden.rs — the SHIPPED calibration packs are the proving input
//! for H2 sections + fenced EXECUTION yaml (CARD-0097, sentinel round 2:
//! "the files that motivated heading_title are not the proving input").
//! If a pack stops validating, the schema and the pack drifted apart.

use caddis_card::Card;

const L1A: &str = include_str!("../../../skills/caddis/calibration/L1-a.md");
const L1B: &str = include_str!("../../../skills/caddis/calibration/L1-b.md");
const L1C: &str = include_str!("../../../skills/caddis/calibration/L1-c.md");
const L2A: &str = include_str!("../../../skills/caddis/calibration/L2-a.md");
const L2B: &str = include_str!("../../../skills/caddis/calibration/L2-b.md");
const L3: &str = include_str!("../../../skills/caddis/calibration/L3.md");
const REVIEW_A: &str = include_str!("../../../skills/caddis/calibration/plan/review-a.md");
const REVIEW_B: &str = include_str!("../../../skills/caddis/calibration/plan/review-b.md");

#[test]
fn every_exec_calibration_pack_is_strict_valid() {
    for (name, text) in [
        ("L1-a", L1A),
        ("L1-b", L1B),
        ("L1-c", L1C),
        ("L2-a", L2A),
        ("L2-b", L2B),
        ("L3", L3),
    ] {
        let card = Card::parse(text).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert!(
            card.validate_strict().is_ok(),
            "{name} must be strict-valid"
        );
    }
}

#[test]
fn plan_review_packs_hold_the_v1_contract() {
    // The reviewer prompts carry Done-When + RED-TEST at H2 but no
    // EXECUTION (a different oracle); they must satisfy v1, not strict.
    for (name, text) in [("review-a", REVIEW_A), ("review-b", REVIEW_B)] {
        let card = Card::parse(text).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert!(card.validate().is_ok(), "{name} must hold the v1 contract");
        assert!(
            card.section("GOAL").is_some() && card.section("OUTPUT").is_some(),
            "{name} keeps its reviewer sections"
        );
    }
}

#[test]
fn plan_review_fences_keep_the_nested_plan_in_one_section() {
    // The wrapper's ```text fence embeds a whole plan card whose
    // `# CHILDREN` / `# REVIEW` must never leak as wrapper sections.
    for (name, text) in [("review-a", REVIEW_A), ("review-b", REVIEW_B)] {
        let card = Card::parse(text).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        let plan = card
            .section("PLAN")
            .unwrap_or_else(|| panic!("{name}: PLAN"));
        assert!(
            plan.body.contains("# CHILDREN"),
            "{name}: nested children intact"
        );
        assert!(
            plan.body.contains("# REVIEW"),
            "{name}: nested review intact"
        );
        assert!(
            card.section("CHILDREN").is_none(),
            "{name}: no leaked section"
        );
        assert!(
            card.section("REVIEW").is_none(),
            "{name}: no leaked section"
        );
    }
}

#[test]
fn golden_l3_anchors_are_exact_verbatim_fixtures() {
    // parse_anchors must keep the blank lines inside a `content: |`
    // literal: the anchor body is the fixture file, byte for byte.
    const L3_APP: &str = include_str!("../../../skills/caddis/calibration/fixtures/l3_app.py");
    let card = Card::parse(L3).unwrap();
    assert!(card.validate_strict().is_ok());
    let exec = card.execution().expect("L3 parses an EXECUTION contract");
    assert_eq!(exec.anchors.len(), 2, "one change, two anchored files");
    assert_eq!(exec.anchors[0].content, "FACTOR = 1\n");
    let want: String = L3_APP.replace("\r\n", "\n");
    assert_eq!(
        exec.anchors[1].content, want,
        "blank lines survive the yaml round-trip"
    );
}
