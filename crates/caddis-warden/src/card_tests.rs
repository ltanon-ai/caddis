//! Direct tests for the pure halves of the card subcommand (CARD-0110).
//!
//! `tests/card_cli.rs` drives the whole thing through the real binary. These
//! reach the two decisions that are pure functions of the card's own text, so a
//! regression names the rule instead of a stderr substring.

use super::*;

/// A strict card with a declared EXECUTION contract. The anchors block is last
/// because `parse_fields` STOPS at `anchors:` — a fixture line inside an anchor
/// body must never retune the contract above it.
fn strict(blast: u32, allowlist: &[&str]) -> caddis_card::Card {
    let items: String = allowlist.iter().map(|p| format!("  - {p}\n")).collect();
    let text = format!(
        "---\nid: CARD-X\nclass: fix\nowner: t\n---\n\
         # x\n\n# Done-When\n- done\n\n# RED-TEST\nred\n\n\
         # EXECUTION\nlevel: L1\nblast: {blast}\nclaims-forbidden: true\n\
         allowlist:\n{items}anchors:\n  - path: a.rs\n      content: |\n        x\n"
    );
    caddis_card::Card::parse(&text).expect("the fixture parses as a card")
}

fn v1() -> caddis_card::Card {
    caddis_card::Card::parse(
        "---\nid: CARD-V1\nclass: fix\nowner: t\n---\n\
         # x\n\n# Done-When\n- done\n\n# RED-TEST\nred\n",
    )
    .expect("the fixture parses as a card")
}

#[test]
fn a_card_naming_more_paths_than_its_blast_allows_is_self_contradictory() {
    // CARDS.md defines blast as the paths the card may touch and allowlist as
    // the exact editable paths, and nothing in code ever related the two — so
    // this shape shipped happily before CARD-0110.
    let why = allowlist_exceeds_blast(&strict(1, &["a.rs", "b.rs", "c.rs"]))
        .expect("blast 1 with three paths must be caught");
    assert!(why.contains("blast 1"), "{why}");
    assert!(why.contains("3 distinct"), "{why}");
}

#[test]
fn a_card_within_its_blast_passes() {
    assert_eq!(allowlist_exceeds_blast(&strict(3, &["a.rs", "b.rs"])), None);
    assert_eq!(allowlist_exceeds_blast(&strict(1, &["a.rs"])), None);
}

#[test]
fn repeated_paths_count_once_because_blast_counts_distinct_paths() {
    // Otherwise a copy-paste slip in the allowlist reads as a blast violation,
    // and the author "fixes" a real contract to satisfy a counting bug.
    assert_eq!(
        allowlist_exceeds_blast(&strict(1, &["a.rs", "a.rs", "a.rs"])),
        None
    );
}

#[test]
fn a_v1_card_has_no_execution_contract_so_there_is_nothing_to_check() {
    // Not an error: a v1 card is a legitimate card and most of this
    // repository's own cards are v1. It simply bounds nothing.
    assert_eq!(allowlist_exceeds_blast(&v1()), None);
}

#[test]
fn the_open_note_says_whether_the_card_actually_bounds_anything() {
    let bounded = bound_note(&strict(2, &["a.rs", "b.rs"]));
    assert!(
        bounded.contains("bounded: 2 allowlist path(s)"),
        "{bounded}"
    );
    assert!(bounded.contains("blast 2"), "{bounded}");

    // THE LINE THAT KEEPS THIS HONEST. A card with no allowlist gives the gate
    // nothing to bound writes with, and saying so at `open` is the difference
    // between a mechanism and a reassuring noise.
    let unbounded = bound_note(&v1());
    assert!(unbounded.contains("NOT BOUNDED"), "{unbounded}");
    assert!(
        unbounded.contains("recorded, not restricted"),
        "{unbounded}"
    );
}

#[test]
fn the_card_id_comes_from_frontmatter_and_falls_back_to_the_path() {
    assert_eq!(card_id(&v1(), "_card_x.md"), "CARD-V1");
    let no_id = caddis_card::Card::parse(
        "---\nclass: fix\nowner: t\n---\n# x\n\n# Done-When\n- d\n\n# RED-TEST\nr\n",
    )
    .expect("parses");
    // A card without an id is still traceable to a file rather than to "".
    assert_eq!(card_id(&no_id, "_card_x.md"), "_card_x.md");
}
