//! plan.rs — the PLAN card class (BC1, card-tree council+quorum 2026-08-23).
//! A plan decomposes a goal into ordered children and carries the receipt
//! that an intent review HAPPENED (presence+shape, never rightness).
//! Different oracle from strict: a plan NEVER passes validate_strict —
//! enforced structurally over there. The crate validates structure only
//! (unique ids, orders 1..N, pairwise-disjoint child paths); repo-reality
//! checks (paths exist, symbols greppable) are walker-side. Parentage
//! reuses the SPLIT parent/of/order encoding via Card::split.

use crate::{Card, CardErr};

/// CHILDREN + REVIEW of a plan card — what a walker consumes.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub children: Vec<PlanChild>,
    pub review: PlanReview,
}

/// One ordered child: id, position, touched paths, named symbols.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanChild {
    pub id: String,
    pub order: u32,
    pub paths: Vec<String>,
    pub symbols: Vec<String>,
}

/// The REVIEW receipt: reviewer, verdict, checks. Presence proves a
/// review happened — the crate never judges whether it was right.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanReview {
    pub reviewer: String,
    pub verdict: String,
    pub checks: String,
}

impl Card {
    /// The PLAN oracle: v1 card basics, then CHILDREN (at least one child;
    /// unique ids; orders exactly 1..N in list order; paths pairwise
    /// disjoint — one path, one child, overlap needs CONTINUATION, never
    /// duplication) and the REVIEW receipt (reviewer named, verdict in
    /// {accepted,rejected}, checks non-empty).
    pub fn validate_plan(&self) -> Result<Plan, CardErr> {
        self.validate()?;
        let children = match self.section("CHILDREN") {
            Some(s) => parse_children(&s.body),
            None => return Err(CardErr::MissingSection("CHILDREN")),
        };
        if children.is_empty() {
            return Err(CardErr::MissingSection("CHILDREN"));
        }
        check_children(&children)?;
        let review = match self.section("REVIEW") {
            Some(s) => PlanReview::parse(&s.body),
            None => return Err(CardErr::MissingSection("REVIEW")),
        };
        check_review(&review)?;
        Ok(Plan { children, review })
    }
}

fn check_children(children: &[PlanChild]) -> Result<(), CardErr> {
    for (i, a) in children.iter().enumerate() {
        if a.id.is_empty() {
            return Err(CardErr::MissingSection("CHILDREN/id"));
        }
        if a.order != i as u32 + 1 {
            return Err(CardErr::MissingSection("CHILDREN/order"));
        }
        for b in &children[i + 1..] {
            if a.id == b.id {
                return Err(CardErr::MissingSection("CHILDREN/id-dup"));
            }
            if a.paths.iter().any(|p| b.paths.contains(p)) {
                return Err(CardErr::MissingSection("CHILDREN/paths-overlap"));
            }
        }
    }
    Ok(())
}

fn check_review(review: &PlanReview) -> Result<(), CardErr> {
    if review.reviewer.is_empty() {
        return Err(CardErr::MissingSection("REVIEW/reviewer"));
    }
    if review.verdict != "accepted" && review.verdict != "rejected" {
        return Err(CardErr::MissingSection("REVIEW/verdict"));
    }
    if review.checks.trim().is_empty() {
        return Err(CardErr::MissingSection("REVIEW/checks"));
    }
    Ok(())
}

/// `- id:` bullets open a child; two-space `key: value` lines feed it.
fn parse_children(body: &str) -> Vec<PlanChild> {
    let mut children: Vec<PlanChild> = Vec::new();
    for raw in body.lines() {
        let line = raw.trim_end();
        if let Some(rest) = line.strip_prefix("- id:") {
            children.push(PlanChild::new(rest.trim()));
        } else if let Some(child) = children.last_mut() {
            child.field(line);
        }
    }
    children
}

impl PlanChild {
    fn new(id: &str) -> Self {
        Self {
            id: id.to_string(),
            order: 0,
            paths: Vec::new(),
            symbols: Vec::new(),
        }
    }

    /// One indented `key: value` line under the id bullet.
    fn field(&mut self, line: &str) {
        if let Some((k, v)) = line.split_once(':') {
            match k.trim() {
                "order" => self.order = v.trim().parse().unwrap_or(0),
                "paths" => self.paths = split_list(v),
                "symbols" => self.symbols = split_list(v),
                _ => {}
            }
        }
    }
}

impl PlanReview {
    fn parse(body: &str) -> Self {
        let mut review = PlanReview {
            reviewer: String::new(),
            verdict: String::new(),
            checks: String::new(),
        };
        for line in body.lines() {
            if let Some((k, v)) = line.split_once(':') {
                match k.trim() {
                    "reviewer" => review.reviewer = v.trim().to_string(),
                    "verdict" => review.verdict = v.trim().to_string(),
                    "checks" => review.checks = v.trim().to_string(),
                    _ => {}
                }
            }
        }
        review
    }
}

/// Comma-separated list (`paths: a.py, b.rs`): trimmed, empties dropped.
fn split_list(v: &str) -> Vec<String> {
    v.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}
