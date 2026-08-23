//! CARD-0003 DW testai (falsifikuojami).
use caddis_card::Card;

const GOOD: &str = r#"---
id: CARD-0003
class: structural
owner: warden
---
# Done-When
parse + validate žali; be sekcijos -> klaida.

# RED-TEST
Išjungus validate -> testai krinta.

# Žingsniai
Skeletas.
"#;

#[test]
fn dw1_parse_sections_and_frontmatter() {
    let c = Card::parse(GOOD).unwrap();
    assert_eq!(c.frontmatter.get("id").unwrap(), "CARD-0003");
    assert_eq!(c.sections.len(), 3);
    assert!(c.section("done-when").is_some(), "case-insensitive paieška");
    assert_eq!(c.section("Done-When").unwrap().start_line, 6);
}

#[test]
fn dw2_validate_requires_red_test() {
    // 26 įstatymas kortos parser'yje
    let c = Card::parse(GOOD).unwrap();
    assert!(c.validate().is_ok());
    let bad = GOOD.replace("# RED-TEST\n", "# Kažkas\n");
    let c2 = Card::parse(&bad).unwrap();
    assert_eq!(
        c2.validate(),
        Err(caddis_card::CardErr::MissingSection("RED-TEST"))
    );
}

#[test]
fn dw3_missing_frontmatter_key() {
    let bad = GOOD.replace("id: CARD-0003\n", "");
    let c = Card::parse(&bad).unwrap();
    assert!(c.validate().is_err());
}
