//! caddis-card — kortos schema + parser v0 (CARD-0003). Korta = darbo vienetas su
//! nuoankstyvu įrodymu (26 red-first) ir mirties planu (28 §1). v0: frontmatter
//! (raktas: reikšmė eilutėmis) + sekcijos (# Antraštė) + eilučių inkarai.
use std::collections::BTreeMap;

mod execution;
pub use execution::{Anchor, Continuation, Execution, Split};

mod plan;
pub use plan::{Plan, PlanChild, PlanReview};

#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    pub frontmatter: BTreeMap<String, String>,
    pub sections: Vec<Section>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub title: String,
    pub start_line: usize, // 1-based
    pub body: String,
}

#[derive(Debug, PartialEq)]
pub enum CardErr {
    MissingSection(&'static str),
    MissingFrontmatter(&'static str),
}

/// Section headings sit at `#` or `##` (CARD-0097): markdownlint MD025
/// forces single-H1 documents to demote their sections, and the schema
/// follows the document. Deeper headings are section BODY, never sections.
fn heading_title(line: &str) -> Option<&str> {
    line.strip_prefix("## ").or_else(|| line.strip_prefix("# "))
}
/// One body line, classified for the section walker. A ``` fence toggles
/// fenced mode: headings and `---` INSIDE a fence are content — the
/// plan-review packs embed whole plan cards in ```text fences, and their
/// `# CHILDREN` must never leak out as wrapper sections.
enum Line<'a> {
    Fence,
    Frontmatter,
    Heading(&'a str),
    Body,
}

fn classify<'a>(line: &'a str, fenced: &mut bool) -> Line<'a> {
    if line.starts_with("```") {
        *fenced = !*fenced;
        return Line::Fence;
    }
    if *fenced {
        return Line::Body;
    }
    if line == "---" {
        return Line::Frontmatter;
    }
    match heading_title(line) {
        Some(title) => Line::Heading(title),
        None => Line::Body,
    }
}

impl Card {
    /// Parse'ina kortos tekstą: `---` frontmatter blokas, po to `#`/`##`
    /// `Sekcija` antraštės (H2 leidžiama, nes markdownlint MD025 reikalauja
    /// vienintelio H1 dokumente — CARD-0097; gilesnės antraštės lieka
    /// turiniu). Fenced blokų antraštės — turinys (plan-review paketai).
    pub fn parse(text: &str) -> Result<Self, CardErr> {
        let mut frontmatter = BTreeMap::new();
        let mut in_fm = false;
        let mut sections = Vec::new();
        let mut cur: Option<Section> = None;
        let mut fenced = false;
        for (i, raw) in text.lines().enumerate() {
            let line = raw.trim_end();
            let n = i + 1;
            let cls = classify(line, &mut fenced);
            if let Line::Frontmatter = cls {
                in_fm = !in_fm && frontmatter.is_empty() && sections.is_empty();
                continue;
            }
            if in_fm {
                if let Some((k, v)) = line.split_once(':') {
                    frontmatter.insert(k.trim().to_string(), v.trim().to_string());
                }
                continue;
            }
            if let Line::Heading(title) = cls {
                if let Some(s) = cur.take() {
                    sections.push(s);
                }
                cur = Some(Section {
                    title: title.trim().to_string(),
                    start_line: n,
                    body: String::new(),
                });
                continue;
            }
            if let Some(s) = cur.as_mut() {
                s.body.push_str(raw);
                s.body.push('\n');
            }
        }
        if let Some(s) = cur.take() {
            sections.push(s);
        }
        Ok(Self {
            frontmatter,
            sections,
        })
    }

    pub fn section(&self, title: &str) -> Option<&Section> {
        self.sections
            .iter()
            .find(|s| s.title.eq_ignore_ascii_case(title))
    }

    /// CARD-SCHEMA-v1 būtinosios sekcijos (26 red-first įstatymas): Done-When su
    /// falsifikuojama forma + RED-TEST eilutė. v0 reikalauja abiejų.
    pub fn validate(&self) -> Result<(), CardErr> {
        for key in ["id", "class", "owner"] {
            if !self.frontmatter.contains_key(key) {
                return Err(CardErr::MissingFrontmatter("id/class/owner"));
            }
        }
        if self.section("Done-When").is_none() {
            return Err(CardErr::MissingSection("Done-When"));
        }
        if self.section("RED-TEST").is_none() {
            return Err(CardErr::MissingSection("RED-TEST"));
        }
        Ok(())
    }

    /// CARD-SCHEMA-v2 strict: the EXECUTION contract for a card destined to
    /// a weak/local executor (quorum card-ladder, 2026-08-23). ADDITIVE —
    /// `validate()` keeps its v1 meaning; strict demands EXECUTION and every
    /// field the ladder reasons about. Anchors are EXACT-verbatim; blast is
    /// an integer 1..=3 as a HARD error (a legitimate 4-path card is a new
    /// class, never an override); level defaults LOW (L1) on absent/invalid
    /// — never an error; claims-forbidden must be explicitly true.
    pub fn validate_strict(&self) -> Result<Execution, CardErr> {
        // BC1 (quorum 2026-08-23): a PLAN never takes the strict oracle —
        // its truth is intent review, not execution shape. Structural, so
        // a plan that grew an EXECUTION section by mistake still never passes.
        if self.frontmatter.get("class").map(String::as_str) == Some("plan") {
            return Err(CardErr::MissingSection("EXECUTION/plan-class"));
        }
        let exec_section = self
            .section("EXECUTION")
            .ok_or(CardErr::MissingSection("EXECUTION"))?;
        let exec = Execution::parse(exec_section.body.as_str())?;
        if let Some(ann) = self.continuation() {
            if ann.blast_cap.unwrap_or(exec.blast) > exec.blast {
                return Err(CardErr::MissingSection("CONTINUATION-broadens"));
            }
        }
        if let Some(split) = self.split() {
            if split.order == 0 || split.of == 0 || split.order > split.of {
                return Err(CardErr::MissingSection("SPLIT-malformed"));
            }
        }
        Ok(exec)
    }

    /// The CONTINUATION annex: how a chained card carries context from its
    /// parent. It may never broaden what it continues — the cap is stated,
    /// and strict rejects a cap above the card's own blast.
    pub fn continuation(&self) -> Option<Continuation> {
        self.section("CONTINUATION")
            .map(|s| Continuation::parse(&s.body))
    }

    /// The SPLIT marker: this card is child `order` of `of` split from
    /// `parent` (operator directive: cards too thick for the executor are
    /// split automatically; each child is a full strict card of its own).
    pub fn split(&self) -> Option<Split> {
        self.section("SPLIT").map(|s| Split::parse(&s.body))
    }
}
