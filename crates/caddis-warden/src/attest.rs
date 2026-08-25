//! attest.rs — proof that travels with the work (CARD-0114, the cherry).
//!
//! For a card that was opened and closed, assemble a PROOF BUNDLE from data
//! already recorded, and let anyone re-check it against the ledger: the
//! operator, a reviewer, another agent, or a future session with no memory of
//! any of it.
//!
//! WHAT IT CHANGES: the estate's hardest law is that the builder never grades
//! its own work. Today that is procedural discipline. A bundle makes a bee's
//! work checkable WITHOUT re-running it, which is the difference between a rule
//! people remember and a mechanism.
//!
//! ⛔ WHAT IT CANNOT DO, AND THE SPEC PROMISED OTHERWISE. The warden fires
//! BEFORE a tool runs and the row is `tag|command|path|why` — there is no exit
//! code anywhere in it. A bundle CANNOT say the RED-TEST failed before and
//! passed after. It says a command matching the RED-TEST was ATTEMPTED inside
//! the window, and the bundle carries that limit in a field of its own so a
//! reader who only looks at the JSON still sees it. An honest bundle that
//! admits the gap is worth more than a confident one nobody can check.

use crate::attest_window::{declared, locate, looks_like_red_test, red_test_lines};
use crate::rows::{parse_row, split_body, Row};
use std::collections::BTreeMap;

pub struct Bundle {
    pub card_id: String,
    pub card_path: String,
    pub card_hash: String,
    pub allowlist: Vec<String>,
    pub blast: u32,
    /// False when the card FILE could not be read or parsed at attest time.
    /// A bundle whose card is gone can check nothing against it, and must say
    /// so rather than printing `OUTSIDE: none` (pre-push review, finding #6).
    pub card_readable: bool,
    pub opened_at_row: usize,
    pub closed_at_row: usize,
    pub from: String,
    pub allow: u64,
    pub steer: u64,
    pub deny: u64,
    pub files: BTreeMap<String, u64>,
    pub outside: Vec<String>,
    pub red_test_seen: bool,
    pub laws: BTreeMap<String, u64>,
    pub unreadable: u64,
}

/// The limits every bundle carries, in the bundle, so a reader who never opens
/// the source still sees them.
pub const LIMITS: [&str; 4] = [
    "the ledger records what a tool was ASKED to do, before it ran: no exit code exists in any row",
    "so RED-TEST evidence means ATTEMPTED inside the window, never passed, and never failed-before",
    "shell write targets are not recoverable in general, so files-written under-reports bash",
    "rows are cited by physical position; seq is not unique for history before CARD-0108",
];

pub fn build(text: &str, card_id: &str) -> Result<Bundle, String> {
    let mut rows: Vec<Row> = Vec::new();
    let mut unreadable = 0u64;
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        match parse_row(line) {
            Some(r) => rows.push(r),
            None => unreadable += 1,
        }
    }
    let found = locate(&rows, card_id).ok_or_else(|| {
        format!("{card_id}: no matching card.open followed by a card.close in this ledger")
    })?;
    let declared_now = declared(&found.path);
    let card_readable = declared_now.is_some();
    let (allowlist, blast) = declared_now.unwrap_or_default();
    let red_lines = red_test_lines(&found.path);
    let mut b = Bundle {
        card_id: card_id.to_string(),
        card_path: found.path.clone(),
        card_hash: found.hash,
        allowlist,
        blast,
        card_readable,
        opened_at_row: found.open_idx,
        closed_at_row: found.close_idx,
        from: found.from.clone(),
        allow: 0,
        steer: 0,
        deny: 0,
        files: BTreeMap::new(),
        outside: Vec::new(),
        red_test_seen: false,
        laws: BTreeMap::new(),
        unreadable,
    };
    for row in &rows[found.open_idx + 1..found.close_idx] {
        if row.from != found.from {
            continue;
        }
        fold(&mut b, row, &red_lines);
    }
    b.outside.sort_unstable();
    b.outside.dedup();
    Ok(b)
}

fn fold(b: &mut Bundle, row: &Row, red_lines: &[String]) {
    let Some((tag, cmd)) = split_body(&row.body) else {
        return;
    };
    let why = row.body.rsplit('|').next().unwrap_or("");
    match tag.as_str() {
        "allow" => b.allow += 1,
        "steer" => {
            b.steer += 1;
            for id in why.split(", ").filter(|s| !s.is_empty()) {
                *b.laws.entry(id.to_string()).or_default() += 1;
            }
        }
        "deny" => {
            b.deny += 1;
            if let Some(id) = crate::rows::law_id_bracketed(why) {
                *b.laws.entry(id).or_default() += 1;
            }
        }
        _ => {}
    }
    if looks_like_red_test(&cmd, red_lines) {
        b.red_test_seen = true;
    }
    let path = crate::attest_verify::row_path(&row.body);
    if path.is_empty() {
        return;
    }
    *b.files.entry(path.clone()).or_default() += 1;
    // ⛔ THE MOST IMPORTANT LIST IN THE BUNDLE. A bundle that omitted a write
    // outside the declared allowlist would be exactly the reassuring artifact
    // this whole program exists against.
    if !b.allowlist.is_empty()
        && !crate::allowlist::declared_covers(&b.allowlist, &crate::allowlist::normalize(&path))
    {
        b.outside.push(path);
    }
}

pub fn run(args: &[String]) -> i32 {
    match args.get(2).map(String::as_str) {
        Some("--card") => match args.get(3) {
            Some(id) => emit(id, args.iter().any(|a| a == "--json")),
            None => usage("attest --card needs a card id"),
        },
        Some("--verify") => match args.get(3) {
            Some(path) => crate::attest_verify::verify(path),
            None => usage("attest --verify needs a bundle path"),
        },
        _ => usage("unknown attest argument"),
    }
}

fn usage(why: &str) -> i32 {
    eprintln!(
        "attest: {why}\nusage: caddis-warden attest --card <CARD-ID> [--json]\n       \
         caddis-warden attest --verify <bundle.json>"
    );
    2
}

fn emit(card_id: &str, json: bool) -> i32 {
    let Some(text) = crate::propose::read_ledger("attest") else {
        return 2;
    };
    match build(&text, card_id) {
        Ok(b) => {
            println!(
                "{}",
                if json {
                    crate::attest_verify::render_json(&b)
                } else {
                    crate::attest_verify::render_text(&b)
                }
            );
            0
        }
        Err(why) => {
            // NOT FOUND is an error, never an empty bundle attested over a
            // guessed window.
            eprintln!("attest: {why}");
            2
        }
    }
}

#[cfg(test)]
#[path = "attest_tests.rs"]
mod tests;
