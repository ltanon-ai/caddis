//! execution.rs — the CARD-SCHEMA-v2 EXECUTION contract, split from lib.rs
//! under the 280-line law (quorum card-ladder correction #5 anticipated the
//! split). Fields pinned by the quorum before any code: anchors EXACT-
//! verbatim; blast 1..=3 hard error; level defaults LOW; claims-forbidden
//! explicit; CONTINUATION may never broaden; SPLIT states parentage+order.

use crate::CardErr;

/// The EXECUTION contract fields (strict mode).
#[derive(Debug, Clone, PartialEq)]
pub struct Execution {
    pub level: String,
    pub blast: u32,
    pub claims_forbidden: bool,
    pub anchors: Vec<Anchor>,
    pub allowlist: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Anchor {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Continuation {
    pub parent: String,
    pub carries: String,
    pub blast_cap: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Split {
    pub parent: String,
    pub order: u32,
    pub of: u32,
}

impl Execution {
    pub(crate) fn parse(body: &str) -> Result<Self, CardErr> {
        let (level, blast, claims) = parse_fields(body);
        let allowlist = parse_allowlist(body);
        let blast = blast.ok_or(CardErr::MissingSection("EXECUTION/blast"))?;
        if blast > 3 {
            return Err(CardErr::MissingSection("EXECUTION/blast>3"));
        }
        let anchors = parse_anchors(body);
        if anchors.is_empty() {
            return Err(CardErr::MissingSection("EXECUTION/anchors"));
        }
        if allowlist.is_empty() {
            return Err(CardErr::MissingSection("EXECUTION/allowlist"));
        }
        if !claims {
            return Err(CardErr::MissingSection("EXECUTION/claims-forbidden"));
        }
        Ok(Execution {
            level,
            blast,
            claims_forbidden: claims,
            anchors,
            allowlist,
        })
    }
}

/// Anchor block parser, split from Execution::parse under the CCN law: each
/// `- path:` opens an anchor; indented `content: |` lines (6 spaces) are the
/// verbatim body until the next anchor or section.
fn parse_anchors(body: &str) -> Vec<Anchor> {
    let mut anchors = Vec::new();
    let mut cur_path = String::new();
    let mut cur_content = String::new();
    for raw in body.lines() {
        let line = raw.trim_end();
        if let Some(rest) = line.strip_prefix("  - path: ") {
            if !cur_path.is_empty() {
                anchors.push(Anchor {
                    path: std::mem::take(&mut cur_path),
                    content: std::mem::take(&mut cur_content),
                });
            }
            cur_path = rest.trim().to_string();
        } else if let Some(stripped) = line.strip_prefix("      ") {
            cur_content.push_str(stripped);
            cur_content.push('\n');
        }
    }
    if !cur_path.is_empty() {
        anchors.push(Anchor {
            path: cur_path,
            content: cur_content.trim_end_matches('\n').to_string(),
        });
    }
    anchors
}

/// Scalar `key: value` fields, one pass (level normalizes LOW on garbage;
/// blast parses or stays None so the caller can hard-error once).
fn parse_fields(body: &str) -> (String, Option<u32>, bool) {
    let mut level = String::new();
    let mut blast = None;
    let mut claims = false;
    for line in body.lines() {
        if let Some((k, v)) = line.split_once(':') {
            match k.trim() {
                "level" => level = normalize_level(v.trim()),
                "blast" => blast = v.trim().parse::<u32>().ok(),
                "claims-forbidden" => claims = v.trim().eq_ignore_ascii_case("true"),
                _ => {}
            }
        }
    }
    (level, blast, claims)
}

/// `allowlist:` bullet items (`  - ...`).
fn parse_allowlist(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_list = false;
    for line in body.lines() {
        let t = line.trim_end();
        if t == "allowlist:" {
            in_list = true;
        } else if t == "anchors:" {
            in_list = false;
        } else if in_list {
            if let Some(item) = t.strip_prefix("  - ") {
                out.push(item.trim().to_string());
            }
        }
    }
    out
}

fn normalize_level(v: &str) -> String {
    match v {
        "L1" | "L2" | "L3" => v.to_string(),
        _ => "L1".to_string(), // the ladder defaults LOW, never errors
    }
}

impl Continuation {
    pub(crate) fn parse(body: &str) -> Self {
        let mut parent = String::new();
        let mut carries = String::new();
        let mut cap = None;
        for line in body.lines() {
            if let Some((k, v)) = line.split_once(':') {
                match k.trim() {
                    "parent" => parent = v.trim().to_string(),
                    "carries" => carries = v.trim().to_string(),
                    "blast-cap" => cap = v.trim().parse::<u32>().ok(),
                    _ => {}
                }
            }
        }
        Self {
            parent,
            carries,
            blast_cap: cap,
        }
    }
}

impl Split {
    pub(crate) fn parse(body: &str) -> Self {
        let mut parent = String::new();
        let mut order = 0;
        let mut of = 0;
        for line in body.lines() {
            if let Some((k, v)) = line.split_once(':') {
                match k.trim() {
                    "parent" => parent = v.trim().to_string(),
                    "order" => order = v.trim().parse().unwrap_or(0),
                    "of" => of = v.trim().parse().unwrap_or(0),
                    _ => {}
                }
            }
        }
        Self { parent, order, of }
    }
}
