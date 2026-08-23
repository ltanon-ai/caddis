//! card_h2.rs — sections may sit at H1 or H2 (CARD-0097).
//!
//! markdownlint's MD025 demands a single H1 per document, so a card that
//! is also a publishable doc must demote its sections to H2. The schema
//! follows the document rather than fighting the linter. Additive: an
//! H1-only card parses exactly as before, and headings deeper than H2
//! stay section BODY, never new sections.

use caddis_card::Card;

const H2_CARD: &str = "---\nid: T\nclass: test\nowner: t\n---\n\n# T\n\n\
                      ## Done-When\n\n- x passes\n\n\
                      ## RED-TEST\n\n- x fails today\n";

#[test]
fn h2_sections_satisfy_the_v1_contract() {
    let card = Card::parse(H2_CARD).unwrap();
    assert!(card.validate().is_ok(), "Done-When + RED-TEST at H2 count");
    assert_eq!(card.section("Done-When").unwrap().start_line, 9);
}

#[test]
fn strict_accepts_an_h2_execution_section() {
    let text = format!(
        "{H2_CARD}## EXECUTION\n\nlevel: L1\nblast: 1\nclaims-forbidden: true\n\
         anchors:\n  - path: a\n    content: |\n      x\nallowlist:\n  - edit a\n"
    );
    let card = Card::parse(&text).unwrap();
    assert!(card.validate_strict().is_ok(), "h2 EXECUTION parses");
}

#[test]
fn h1_cards_and_deeper_headings_are_unchanged() {
    let mixed = "---\nid: T\nclass: test\nowner: t\n---\n\n# T\n\n\
                 ## Done-When\n\n- x\n\n### detail stays body\n\n\
                 # RED-TEST\n\n- y\n";
    let card = Card::parse(mixed).unwrap();
    assert!(card.validate().is_ok());
    assert!(
        card.section("Done-When").unwrap().body.contains("### detail stays body"),
        "H3 never opens a section"
    );
    assert!(card.section("RED-TEST").unwrap().body.contains("- y"));
}
