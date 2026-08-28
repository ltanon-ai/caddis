//! F3: the card IS the task profile. The router never classifies free text —
//! it reads frontmatter keys + the two mandatory structured sections through
//! caddis-card's own parser. Read surface is minimal by law: `id` + `class`
//! from frontmatter, `Done-When` + `RED-TEST` bodies. `owner` is deliberately
//! UNREAD (routing keys on class, not personnel); every other section is
//! invisible to the router, which is what keeps the injection surface closed.

use caddis_card::Card;

/// What the router may know about a task (F3 read surface, nothing else).
#[derive(Debug, Clone, PartialEq)]
pub struct TaskProfile {
    pub card_id: String,
    pub class: String,
    pub done_when: String,
    pub red_test: String,
}

#[derive(Debug, PartialEq)]
pub enum ProfileErr {
    MissingFrontmatter(&'static str),
    MissingSection(&'static str),
}

/// Extract the routing profile from a parsed card. Deterministic, allocation
/// limited to the four fields. Rejects a card missing any of the four inputs:
/// a task without stakes (RED-TEST) has no falsifiable outcome to route on.
pub fn profile_from_card(card: &Card) -> Result<TaskProfile, ProfileErr> {
    let card_id = card
        .frontmatter
        .get("id")
        .ok_or(ProfileErr::MissingFrontmatter("id"))?
        .clone();
    let class = card
        .frontmatter
        .get("class")
        .ok_or(ProfileErr::MissingFrontmatter("class"))?
        .clone();
    let done_when = card
        .section("Done-When")
        .ok_or(ProfileErr::MissingSection("Done-When"))?
        .body
        .clone();
    let red_test = card
        .section("RED-TEST")
        .ok_or(ProfileErr::MissingSection("RED-TEST"))?
        .body
        .clone();
    Ok(TaskProfile {
        card_id,
        class,
        done_when,
        red_test,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CARD_TEXT: &str = "---\nid: C-42\nclass: chair\nowner: loop\n---\n# Done-When\n\nThe tray feed renders.\n\n# RED-TEST\n\ne2e-tray 17/17 green.\n";

    #[test]
    fn profile_reads_the_f3_surface_and_nothing_else() {
        let card = caddis_card::Card::parse(CARD_TEXT).expect("parses");
        let p = profile_from_card(&card).expect("profile");
        assert_eq!(p.card_id, "C-42");
        assert_eq!(p.class, "chair");
        assert!(p.done_when.contains("tray feed"));
        assert!(p.red_test.contains("17/17"));
        // `owner` is deliberately unread: routing keys on class, not
        // personnel — assert the surface never grew it.
        assert!(!format!("{p:?}").contains("owner"));
    }

    #[test]
    fn card_without_stakes_is_rejected() {
        let no_red = "---\nid: C-43\nclass: chair\n---\n# Done-When\n\nShips.\n";
        let card = caddis_card::Card::parse(no_red).expect("parses");
        assert_eq!(
            profile_from_card(&card),
            Err(ProfileErr::MissingSection("RED-TEST"))
        );

        let no_class = "---\nid: C-44\n---\n# Done-When\n\nShips.\n\n# RED-TEST\n\nnpm green.\n";
        let card = caddis_card::Card::parse(no_class).expect("parses");
        assert_eq!(
            profile_from_card(&card),
            Err(ProfileErr::MissingFrontmatter("class"))
        );
    }
}
