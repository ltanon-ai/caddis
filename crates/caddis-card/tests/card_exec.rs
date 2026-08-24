//! card_exec.rs — CARD-SCHEMA-2: the EXECUTION section and strict validation.
//!
//! The quorum (2026-08-23, card-ladder) pinned the field contract before any
//! code: anchors are EXACT-verbatim (relocatable anchors are a later card);
//! blast is an integer 1..=3, a HARD error outside the range (a legitimate
//! 4-path card is a new class, never an override); level is L1|L2|L3 with
//! absent-or-invalid
//! falling back to L1 (never an error — the ladder defaults LOW); strict mode
//! is ADDITIVE — an existing Done-When + RED-TEST card keeps validating.
//! The CONTINUATION annex may carry context but may never broaden
//! allowlist, blast, or level of anything it continues.

use caddis_card::Card;

const BASE: &str = "---\nid: T\nclass: test\nowner: t\n---\n\n# T\n\n# Done-When\n\n- x passes\n\n# RED-TEST\n\n- x fails today\n";

fn with_exec(exec: &str) -> String {
    format!("{BASE}\n# EXECUTION\n\n{exec}\n")
}

#[test]
fn base_card_still_validates_without_strict() {
    let card = Card::parse(BASE).expect("parse");
    assert!(card.validate().is_ok(), "v1 contract is untouched");
    assert!(card.validate_strict().is_err(), "strict demands EXECUTION");
}

#[test]
fn strict_accepts_a_well_formed_execution_section() {
    let text = with_exec(
        "level: L2\nblast: 2\nclaims-forbidden: true\nanchors:\n  - path: a.py\n    content: |\n      def f():\n          pass\nallowlist:\n  - edit a.py\n",
    );
    let card = Card::parse(&text).expect("parse");
    let exec = card.validate_strict().expect("strict ok");
    assert_eq!(exec.level, "L2");
    assert_eq!(exec.blast, 2);
    assert!(exec.claims_forbidden);
    assert_eq!(exec.anchors.len(), 1);
    assert_eq!(exec.allowlist.len(), 1);
}

#[test]
fn blast_above_three_is_a_hard_error_not_a_default() {
    let text = with_exec("level: L1\nblast: 4\nclaims-forbidden: true\nanchors:\n  - path: a\n    content: |\n      x\nallowlist:\n  - edit a\n");
    let card = Card::parse(&text).expect("parse");
    let err = card.validate_strict().expect_err("blast 4 must hard-fail");
    assert!(
        format!("{err:?}").to_lowercase().contains("blast"),
        "the error names blast, got {err:?}"
    );
}

#[test]
fn blast_zero_is_not_a_range_the_quorum_pinned() {
    // The quorum pinned blast as an integer 1..=3; 0 touched-paths is
    // not a card. Found by the doc-reality sentinel round (CARD-0099).
    let text = with_exec("level: L1\nblast: 0\nclaims-forbidden: true\nanchors:\n  - path: a\n    content: |\n      x\nallowlist:\n  - edit a\n");
    let card = Card::parse(&text).expect("parse");
    let err = card.validate_strict().expect_err("blast 0 must hard-fail");
    assert!(
        format!("{err:?}").to_lowercase().contains("blast"),
        "the error names blast, got {err:?}"
    );
}

#[test]
fn absent_level_defaults_to_l1_like_garbage_does() {
    let text = with_exec(
        "blast: 1\nclaims-forbidden: true\nanchors:\n  - path: a\n    content: |\n      x\nallowlist:\n  - edit a\n",
    );
    let card = Card::parse(&text).expect("parse");
    let exec = card
        .validate_strict()
        .expect("absent level defaults LOW, never an error");
    assert_eq!(exec.level, "L1", "the absent key defaults like garbage");
}

#[test]
fn anchor_bytes_never_retune_the_contract() {
    // A verbatim fixture mentioning contract keys inside content:| is
    // DATA, not a declaration (doc-reality sentinel round).
    let text = with_exec(
        "level: L1\nblast: 1\nclaims-forbidden: true\nanchors:\n  - path: cfg.py\n    content: |\n      level: L3\n      blast: 4\n      claims-forbidden: false\nallowlist:\n  - edit cfg.py\n",
    );
    let card = Card::parse(&text).expect("parse");
    let exec = card.validate_strict().expect("the declared contract holds");
    assert_eq!(exec.level, "L1");
    assert_eq!(exec.blast, 1);
    assert!(exec.claims_forbidden);
}

#[test]
fn anchors_boundary_survives_trailing_space() {
    // `anchors: ` with trailing whitespace must still stop field parsing;
    // fixture bytes below it are data, not a declaration.
    let text = with_exec(
        "level: L1\nblast: 1\nclaims-forbidden: true\nanchors: \n  - path: cfg.py\n    content: |\n      blast: 4\nallowlist:\n  - edit cfg.py\n",
    );
    let card = Card::parse(&text).expect("parse");
    let exec = card
        .validate_strict()
        .expect("fixture blast never overrides the declaration");
    assert_eq!(exec.blast, 1);
}

#[test]
fn invalid_or_absent_level_defaults_to_l1_never_errors() {
    let text = with_exec(
        "level: L9\nblast: 1\nclaims-forbidden: true\nanchors:\n  - path: a\n    content: |\n      x\nallowlist:\n  - edit a\n",
    );
    let card = Card::parse(&text).expect("parse");
    let exec = card.validate_strict().expect("level garbage is not fatal");
    assert_eq!(exec.level, "L1", "the ladder defaults LOW");
}

#[test]
fn continuation_annex_may_carry_context_but_never_broaden() {
    let text = format!(
        "{BASE}\n# EXECUTION\n\nlevel: L1\nblast: 1\nclaims-forbidden: true\nanchors:\n  - path: a\n    content: |\n      x\nallowlist:\n  - edit a\n\n# CONTINUATION\n\nparent: CARD-P\ncarries: the failing diff context\nblast-cap: 1\n"
    );
    let card = Card::parse(&text).expect("parse");
    let ann = card.continuation().expect("annex parsed");
    assert_eq!(ann.parent, "CARD-P");
    assert_eq!(ann.blast_cap, Some(1), "the cap is stated, not implied");
}

#[test]
fn continuation_blast_cap_above_parent_blast_is_rejected() {
    let text = format!(
        "{BASE}\n# EXECUTION\n\nlevel: L1\nblast: 1\nclaims-forbidden: true\nanchors:\n  - path: a\n    content: |\n      x\nallowlist:\n  - edit a\n\n# CONTINUATION\n\nparent: CARD-P\ncarries: x\nblast-cap: 3\n"
    );
    let card = Card::parse(&text).expect("parse");
    assert!(
        card.validate_strict().is_err(),
        "an annex may never broaden what it continues"
    );
}

#[test]
fn a_split_child_marks_its_parent_and_place_in_the_sequence() {
    // A card too thick for the executor is SPLIT by the orchestrator into
    // ordered children (operator directive 2026-08-23: the model can split
    // cards automatically). Each child is a full strict card of its own;
    // the marker only states parentage and order.
    let text = format!(
        "{BASE}\n# EXECUTION\n\nlevel: L1\nblast: 1\nclaims-forbidden: true\nanchors:\n  - path: a\n    content: |\n      x\nallowlist:\n  - edit a\n\n# SPLIT\n\nparent: CARD-BIG\norder: 2\nof: 3\n"
    );
    let card = Card::parse(&text).expect("parse");
    let split = card.split().expect("split parsed");
    assert_eq!(split.parent, "CARD-BIG");
    assert_eq!((split.order, split.of), (2, 3));
}

#[test]
fn a_split_child_with_order_beyond_of_is_rejected() {
    let text = format!(
        "{BASE}\n# EXECUTION\n\nlevel: L1\nblast: 1\nclaims-forbidden: true\nanchors:\n  - path: a\n    content: |\n      x\nallowlist:\n  - edit a\n\n# SPLIT\n\nparent: CARD-BIG\norder: 4\nof: 3\n"
    );
    let card = Card::parse(&text).expect("parse");
    assert!(
        card.validate_strict().is_err(),
        "order > of is a malformed split"
    );
}
