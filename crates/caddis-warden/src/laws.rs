//! laws.rs — the law market (CARD-0113, unit C).
//!
//! Replay already prints `law fires` and a `never fired` list; this promotes
//! that data into a LIFECYCLE. The estate carries 130+ jit laws with zero usage
//! feedback, which is how a rule corpus rots: nobody can tell which rules are
//! working, which are being routed around, and which have never fired at all.
//!
//! ⚠ CIRCUMVENTION IS A HEURISTIC AND IS LABELLED ONE EVERYWHERE IT APPEARS.
//! The ledger records what a tool was ASKED to do, never what happened, so
//! "denied and then got through anyway" is inferred from a shape: a deny, then
//! the SAME caller shortly after running a command with the same head verb and
//! being allowed. That catches the common retry-with-a-variation, and it will
//! also count an honest fix-then-retry. It is a lead worth reading, never a
//! verdict about an agent.

use crate::rows::{body_why, from_matches, law_id_bracketed, parse_row, split_body, Row};
use std::collections::BTreeMap;

/// How many rows after a denial count as "shortly after". Wide enough to cover
/// a fix-and-retry, narrow enough that an unrelated later command does not get
/// attributed to the denial.
const LOOKAHEAD: usize = 12;

#[derive(Default, Clone)]
pub struct LawUse {
    pub deny: u64,
    pub steer: u64,
    pub circumvented: u64,
}

impl LawUse {
    pub fn fires(&self) -> u64 {
        self.deny + self.steer
    }

    /// Share of DENIALS that were followed by the same caller getting the same
    /// head verb through. Steers are excluded: a steer does not block anything,
    /// so proceeding after one is obedience, not circumvention.
    pub fn circumvention_rate(&self) -> f64 {
        if self.deny == 0 {
            return 0.0;
        }
        (self.circumvented as f64) * 100.0 / (self.deny as f64)
    }

    pub fn verdict(&self) -> &'static str {
        if self.fires() == 0 {
            // Registered and never fired. Not automatically useless — it may
            // guard something nobody has attempted yet — so the word is DEAD,
            // and the decision stays with a reader.
            return "DEAD";
        }
        if self.deny > 0 && self.circumvention_rate() >= 50.0 {
            return "WALLPAPER";
        }
        "EARNING"
    }
}

pub struct Market {
    pub laws: BTreeMap<String, LawUse>,
    pub rows: u64,
    pub unreadable: u64,
}

/// One row reduced to what the market needs.
struct Judged {
    from: String,
    tag: String,
    head: String,
    ids: Vec<String>,
}

fn head_verb(cmd: &str) -> String {
    cmd.split_whitespace().next().unwrap_or("").to_string()
}

fn judged(row: &Row) -> Option<Judged> {
    let (tag, cmd) = split_body(&row.body)?;
    let why = body_why(&row.body);
    let ids = match tag.as_str() {
        "deny" => law_id_bracketed(&why).into_iter().collect(),
        "steer" => why
            .split(", ")
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    };
    Some(Judged {
        from: row.from.clone(),
        tag,
        head: head_verb(&cmd),
        ids,
    })
}

/// Was this denial followed by the same caller getting the same head verb
/// through? The caller match is EXACT: attributing one agent's work-around to
/// another would make the record worse than no record.
fn worked_around(all: &[Judged], at: usize) -> bool {
    let d = &all[at];
    if d.head.is_empty() {
        return false;
    }
    all.iter()
        .skip(at + 1)
        .take(LOOKAHEAD)
        .any(|later| later.tag == "allow" && later.from == d.from && later.head == d.head)
}

/// Does this row belong in the window the reader asked for?
fn admits(row: &Row, from: Option<&str>, since_hours: Option<u64>, now: u64) -> bool {
    if let Some(f) = from {
        if !from_matches(&row.from, f) {
            return false;
        }
    }
    match since_hours {
        // ts == 0 means UNKNOWN, and treating unknown as recent would widen
        // every window silently.
        Some(h) => row.ts != 0 && now.saturating_sub(row.ts) <= h * 3600,
        None => true,
    }
}

/// The judged rows in the window, and how many lines could not be read.
/// Split from `build` when the gate measured CCN 14 against the cap of 10 —
/// the fix is a split, never a trim.
fn collect(
    text: &str,
    from: Option<&str>,
    since_hours: Option<u64>,
    now: u64,
) -> (Vec<Judged>, u64) {
    let mut unreadable = 0u64;
    let mut out: Vec<Judged> = Vec::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let Some(row) = parse_row(line) else {
            unreadable += 1;
            continue;
        };
        if !admits(&row, from, since_hours, now) {
            continue;
        }
        if let Some(j) = judged(&row) {
            out.push(j);
        }
    }
    (out, unreadable)
}

pub fn build(text: &str, from: Option<&str>, since_hours: Option<u64>, now: u64) -> Market {
    let (judged_rows, unreadable) = collect(text, from, since_hours, now);
    let mut laws: BTreeMap<String, LawUse> = BTreeMap::new();
    // ⛔ EVERY REGISTERED LAW IS SEEDED, INCLUDING THE ONES THAT NEVER FIRED. A
    // market that lists only what fired cannot report a DEAD rule, which is the
    // one finding it exists to produce.
    for id in crate::checks::registered_ids() {
        laws.entry(id.to_string()).or_default();
    }
    for (i, j) in judged_rows.iter().enumerate() {
        let around = j.tag == "deny" && worked_around(&judged_rows, i);
        for id in &j.ids {
            let e = laws.entry(id.clone()).or_default();
            match j.tag.as_str() {
                "deny" => {
                    e.deny += 1;
                    if around {
                        e.circumvented += 1;
                    }
                }
                "steer" => e.steer += 1,
                _ => {}
            }
        }
    }
    Market {
        laws,
        rows: judged_rows.len() as u64,
        unreadable,
    }
}

pub fn render_text(m: &Market) -> String {
    let mut s = format!("laws: {} judged row(s)", m.rows);
    s.push_str("\nverdict  fires  deny  steer  worked-around  law");
    for (id, u) in &m.laws {
        s.push_str(&format!(
            "\n{:<9}{:>5}{:>6}{:>7}   {:>3} ({:>5.1}%)  {id}",
            u.verdict(),
            u.fires(),
            u.deny,
            u.steer,
            u.circumvented,
            u.circumvention_rate()
        ));
    }
    let dead = m.laws.values().filter(|u| u.fires() == 0).count();
    let wall = m
        .laws
        .values()
        .filter(|u| u.verdict() == "WALLPAPER")
        .count();
    s.push_str(&format!(
        "\n\n{} law(s): {dead} DEAD, {wall} WALLPAPER, {} EARNING",
        m.laws.len(),
        m.laws.len() - dead - wall
    ));
    s.push_str(&format!(
        "\nworked-around is a HEURISTIC: a denial followed within {LOOKAHEAD} rows by the \
         SAME caller running the same head verb and being allowed. It counts an honest \
         fix-and-retry too. Read it as a lead, never as a verdict about an agent."
    ));
    s.push_str(&format!(
        "\n{} unreadable line(s) FILE-WIDE (a torn row has no window to belong to)",
        m.unreadable
    ));
    s
}

pub fn render_json(m: &Market) -> String {
    let body: Vec<String> = m
        .laws
        .iter()
        .map(|(id, u)| {
            format!(
                "\"{}\":{{\"verdict\":\"{}\",\"fires\":{},\"deny\":{},\"steer\":{},\
                 \"circumvented\":{},\"circumvention_pct\":{:.1}}}",
                crate::wire::json_escape(id),
                u.verdict(),
                u.fires(),
                u.deny,
                u.steer,
                u.circumvented,
                u.circumvention_rate()
            )
        })
        .collect();
    format!(
        "{{\"rows\":{},\"unreadable\":{},\"heuristic\":\"worked-around is inferred, \
         not observed\",\"laws\":{{{}}}}}",
        m.rows,
        m.unreadable,
        body.join(",")
    )
}

pub fn run(args: &[String]) -> i32 {
    let f = crate::receipt::parse_filters(&args[2.min(args.len())..]);
    let Some(text) = crate::propose::read_ledger("laws") else {
        return 2;
    };
    let m = build(
        &text,
        f.from.as_deref(),
        f.since_hours,
        crate::identity::unix_seconds(),
    );
    if args.iter().any(|a| a == "--json") {
        println!("{}", render_json(&m));
    } else {
        println!("{}", render_text(&m));
    }
    0
}

#[cfg(test)]
#[path = "laws_tests.rs"]
mod tests;
