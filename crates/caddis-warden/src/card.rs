//! card.rs — `caddis-warden card open|status|close` (CARD-0110, unit B core).
//!
//! The card schema has always declared `allowlist` and `blast`, and the warden
//! has always seen every write path. Nothing has ever connected them. This
//! module makes "a card is open" a FACT IN THE LEDGER, which is the prerequisite
//! for the gate (CARD-0111), for the receipt, and for attest.
//!
//! ⚠ THIS MODULE DENIES NOTHING. It records intent; enforcement is a separate
//! card, so the state can be proven correct before anything starts refusing
//! work on the strength of it.

use crate::card_state::{self, ActiveCard, CardState, OPEN_TYPE};
use crate::identity::{caller_id, fnv1a, is_session_scoped, ledger_path, unix_seconds};

const USAGE: &str = "\
usage: caddis-warden card open <card.md> | card status | card close [--verify -- <cmd>]";

pub fn run(args: &[String]) -> i32 {
    match args.get(2).map(String::as_str) {
        Some("open") => match args.get(3) {
            Some(path) => open(path),
            None => fail("card open needs a path to a card"),
        },
        Some("status") => status(),
        Some("close") => crate::card_verify::close(&args[3..]),
        _ => fail("unknown card subcommand"),
    }
}

pub(crate) fn fail(why: &str) -> i32 {
    eprintln!("card: {why}\n{USAGE}");
    2
}

/// The ledger text and the caller, or an error already reported.
pub(crate) fn read_state() -> Result<(String, String, CardState), i32> {
    let caller = caller_id();
    let path = ledger_path();
    // An unreadable ledger is an ERROR, never an empty one: "no card is open"
    // and "the ledger could not be read" must never look alike.
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(fail(&format!("cannot read {}: {e}", path.display()))),
    };
    let state = card_state::active_for(&text, &caller);
    Ok((text, caller, state))
}

fn open(card_path: &str) -> i32 {
    let bytes = match std::fs::read_to_string(card_path) {
        Ok(b) => b,
        Err(e) => return fail(&format!("cannot read {card_path}: {e}")),
    };
    let card = match caddis_card::Card::parse(&bytes) {
        Ok(c) => c,
        Err(e) => return fail(&format!("{card_path} is not a card: {e:?}")),
    };
    if let Err(e) = card.validate() {
        return fail(&format!("{card_path} fails the card schema: {e:?}"));
    }
    if let Some(why) = allowlist_exceeds_blast(&card) {
        return fail(&why);
    }
    let (_, caller, state) = match read_state() {
        Ok(v) => v,
        Err(code) => return code,
    };
    // ⛔ AN ADAPTER THAT CANNOT NAME ITS SESSION CANNOT HOLD A CARD. Every
    // session behind a bare harness label stamps the same string, so the card
    // would bound a DIFFERENT session's writes.
    if !is_session_scoped(&caller) {
        return fail(&format!(
            "caller `{caller}` names a harness, not a session, so a card opened \
             here would bound another session's writes. The adapter must stamp \
             CADDIS_WARDEN_FROM=<label>.<session> (CARD-0109)"
        ));
    }
    if let Some(why) = bee_unbounded(&caller, &card) {
        return fail(&why);
    }
    // Refuse, never nest: a nested card has no defensible answer to "which
    // allowlist applies".
    if let Some(active) = state.active {
        return fail(&format!(
            "{} is already open for {caller}; close it first",
            active.id
        ));
    }
    let id = card_id(&card, card_path);
    let hash = format!("{:016x}", fnv1a(&bytes));
    match append(
        OPEN_TYPE,
        &card_state::body("open", &id, card_path, &hash),
        &caller,
    ) {
        Ok(seq) => {
            println!("card open: {id} ({card_path}) seq={seq} hash={hash}");
            println!("{}", bound_note(&card));
            0
        }
        Err(why) => fail(&why),
    }
}

/// What this card actually bounds, said plainly at the moment it is opened.
///
/// A v1 card (Done-When + RED-TEST) is a legitimate card and most of this
/// repository's own cards are v1 — but it declares no allowlist, so the gate
/// has nothing to bound writes WITH. Printing that at `open` is the difference
/// between a mechanism and a reassuring noise.
fn bound_note(card: &caddis_card::Card) -> String {
    match card.execution() {
        Some(exec) => format!(
            "  bounded: {} allowlist path(s), blast {}",
            exec.allowlist.len(),
            exec.blast
        ),
        None => "  NOT BOUNDED: no EXECUTION contract, so no allowlist to enforce \
                 (this is a v1 card; writes are recorded, not restricted)"
            .to_string(),
    }
}

/// Bee lanes (little-coder / droid / bee) may not open a v1 card: no
/// EXECUTION means the gate cannot bound their writes (CARD-0131).
fn bee_unbounded(caller: &str, card: &caddis_card::Card) -> Option<String> {
    if card.execution().is_some() {
        return None;
    }
    let label = caller.split('.').next().unwrap_or(caller);
    if matches!(label, "little-coder" | "droid" | "bee") {
        Some(format!(
            "v1 card has no EXECUTION; bee lane `{label}` cannot open it unbounded"
        ))
    } else {
        None
    }
}

/// `CARDS.md` defines `blast` as the paths the card may touch and `allowlist`
/// as the exact editable paths, and nothing in code has ever related the two —
/// so a `blast: 1` card naming three allowlist entries was self-contradictory
/// before any work began. Caught statically, where it costs nothing.
fn allowlist_exceeds_blast(card: &caddis_card::Card) -> Option<String> {
    let exec = card.execution()?;
    let mut paths: Vec<&str> = exec.allowlist.iter().map(String::as_str).collect();
    paths.sort_unstable();
    paths.dedup();
    if paths.len() as u32 > exec.blast {
        return Some(format!(
            "the card declares blast {} but names {} distinct allowlist paths; \
             one of the two numbers is wrong",
            exec.blast,
            paths.len()
        ));
    }
    None
}

fn card_id(card: &caddis_card::Card, path: &str) -> String {
    card.frontmatter
        .get("id")
        .cloned()
        .unwrap_or_else(|| path.to_string())
}

fn status() -> i32 {
    let (_, caller, state) = match read_state() {
        Ok(v) => v,
        Err(code) => return code,
    };
    match &state.active {
        Some(c) => println!(
            "card open for {caller}: {} ({}) hash={}",
            c.id, c.path, c.hash
        ),
        None => println!("no card open for {caller}"),
    }
    // ALWAYS printed, including when it is zero: a reader must be able to tell
    // "nothing is open" from "the ledger is damaged and I may have missed it".
    println!("ledger lines unreadable: {}", state.unreadable);
    0
}


/// The card file must be the card that was opened.
///
/// Nothing else stops an executor from editing its own allowlist mid-card,
/// which would make the whole declaration meaningless. Detects an edit, not a
/// forgery — see `identity::fnv1a` for why a stronger hash buys nothing here.
pub(crate) fn changed_since_open(active: &ActiveCard) -> Option<String> {
    let now = match std::fs::read_to_string(&active.path) {
        Ok(b) => format!("{:016x}", fnv1a(&b)),
        Err(e) => {
            return Some(format!(
                "{} was opened from {} and it can no longer be read ({e}); \
                 a card that vanished mid-work cannot be closed honestly",
                active.id, active.path
            ))
        }
    };
    if now != active.hash {
        return Some(format!(
            "{} changed since it was opened ({} -> {}); an executor that can \
             rewrite its own card mid-work has no card at all",
            active.id, active.hash, now
        ));
    }
    None
}

/// Append one card row and READ IT BACK.
///
/// ⛔ `Ledger::append` FAILS OPEN by design — a full disk must not halt every
/// tool call behind an audit trail — so "no error" is not "it landed". The one
/// place that distinction matters most is the row that declares a card open,
/// because everything downstream derives from it.
///
/// The body is NOT routed through `mask_at_rest`: a card path is easily long
/// enough to be redacted whole, and a redacted card row cannot be reconstructed
/// into state (quorum, program REVISION 1).
pub(crate) fn append(row_type: &str, body: &str, caller: &str) -> Result<u64, String> {
    let path = ledger_path();
    let mut led = caddis_core::ledger::Ledger::open(&path)
        .map_err(|e| format!("ledger unavailable at {}: {e}", path.display()))?;
    let id = format!("card{:016x}", fnv1a(body));
    let env = caddis_core::envelope::validate(
        1,
        &id,
        &format!("{:016x}", fnv1a(&format!("{row_type}{body}"))),
        row_type,
        caller,
        "warden",
        body,
        &unix_seconds().to_string(),
    )
    .map_err(|e| format!("envelope refused: {} {}", e.code, e.why))?;
    let seq = led
        .append(&env)
        .map_err(|e| format!("ledger append failed: {e}"))?;
    let written = std::fs::read_to_string(&path).unwrap_or_default();
    if !written.contains(&id) {
        return Err(format!(
            "the {row_type} row reported seq {seq} but is not in {}; \
             the append failed open and the card is NOT recorded",
            path.display()
        ));
    }
    Ok(seq)
}

#[cfg(test)]
#[path = "card_tests.rs"]
mod tests;
