//! caddis-card — kortos schema + parser v0 (CARD-0003). Korta = darbo vienetas su
//! nuoankstyvu įrodymu (26 red-first) ir mirties planu (28 §1). v0: frontmatter
//! (raktas: reikšmė eilutėmis) + sekcijos (# Antraštė) + eilučių inkarai.
use std::collections::BTreeMap;

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

impl Card {
    /// Parse'ina kortos tekstą: `---` frontmatter blokas, po to `# Sekcija` antraštės.
    pub fn parse(text: &str) -> Result<Self, CardErr> {
        let mut frontmatter = BTreeMap::new();
        let mut in_fm = false;
        let mut sections = Vec::new();
        let mut cur: Option<Section> = None;
        for (i, raw) in text.lines().enumerate() {
            let line = raw.trim_end();
            let n = i + 1;
            if line == "---" {
                in_fm = !in_fm && frontmatter.is_empty() && sections.is_empty();
                if in_fm {
                    continue;
                }
                continue;
            }
            if in_fm {
                if let Some((k, v)) = line.split_once(':') {
                    frontmatter.insert(k.trim().to_string(), v.trim().to_string());
                }
                continue;
            }
            if let Some(title) = line.strip_prefix("# ") {
                if let Some(s) = cur.take() {
                    sections.push(s);
                }
                cur = Some(Section {
                    title: title.trim().to_string(),
                    start_line: n,
                    body: String::new(),
                });
            } else if let Some(s) = cur.as_mut() {
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
}
