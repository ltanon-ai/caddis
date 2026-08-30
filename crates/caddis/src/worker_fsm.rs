//! worker_fsm.rs — CARD-0225. The phase state machine organ.
//! Council fold #4: phases are an enum, repairs are counted, the cap
//! is enforced HERE — never by a model's promise. Bees advance phase
//! only through `caddis worker phase`; phases.log stays append-only
//! truth, phase.state is the live cursor. `attempt` counts REPAIRS;
//! display floors at r1.

use std::fs;
use std::path::Path;

use crate::lineage;

pub enum Error {
    Usage(String),
    Fail(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Usage(s) | Error::Fail(s) => write!(f, "{s}"),
        }
    }
}

pub const PHASES: [&str; 7] = ["task", "scout", "build", "scan", "repair", "done", "fail"];
const REPAIR_CAP: u32 = 3;

pub struct State {
    pub card: String,
    pub repairs: u32,
    pub phase: String,
}

struct FsmArgs {
    card: Option<String>,
    advance: Option<String>,
}

pub fn run(args: &[String]) -> Result<(), Error> {
    let (id, rest) = lineage::take(args).map_err(Error::Usage)?;
    let dir = lineage::dir(&id).map_err(Error::Fail)?;
    let a = parse_args(&rest)?;
    match a.advance {
        None => report(&dir),
        Some(ph) => {
            let card = a
                .card
                .ok_or_else(|| Error::Usage("advance requires --card".into()))?;
            advance_phase(&dir, &card, &ph)
        }
    }
}

fn parse_args(rest: &[String]) -> Result<FsmArgs, Error> {
    let mut out = FsmArgs {
        card: None,
        advance: None,
    };
    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            "--card" => {
                i += 1;
                out.card = Some(
                    rest.get(i)
                        .ok_or_else(|| Error::Usage("missing --card value".into()))?
                        .clone(),
                );
            }
            "--advance" => {
                i += 1;
                out.advance = Some(
                    rest.get(i)
                        .ok_or_else(|| Error::Usage("missing --advance value".into()))?
                        .clone(),
                );
            }
            a => return Err(Error::Usage(format!("unknown argument {a}"))),
        }
        i += 1;
    }
    Ok(out)
}

fn report(dir: &Path) -> Result<(), Error> {
    match read_state(dir) {
        Some(s) => {
            println!("CARD {} r{} {}", s.card, s.repairs.max(1), s.phase);
            Ok(())
        }
        None => {
            println!("CARD none");
            Ok(())
        }
    }
}

fn advance_phase(dir: &Path, card: &str, phase: &str) -> Result<(), Error> {
    if !PHASES.contains(&phase) {
        return Err(Error::Usage(format!(
            "unknown phase {phase}; one of {}",
            PHASES.join(" ")
        )));
    }
    let cur = read_state(dir);
    let (mut repairs, prev_card) = match &cur {
        Some(s) => (s.repairs, s.card.clone()),
        None => (0, card.to_string()),
    };
    if prev_card != card {
        repairs = 0; // new card resets the repair counter
    }
    // Repair cap: a 4th repair fails the card (organ law, not a prompt).
    if phase == "repair" && prev_card == card {
        if repairs >= REPAIR_CAP {
            journal(dir, card, "fail", repairs);
            write_state(dir, card, repairs, "fail");
            println!("CARD {card} fail (repair cap {REPAIR_CAP})");
            return Err(Error::Fail("repair cap reached".into()));
        }
        repairs += 1;
    }
    journal(dir, card, phase, repairs);
    write_state(dir, card, repairs, phase);
    println!("CARD {card} r{} {phase}", repairs.max(1));
    Ok(())
}

fn journal(dir: &Path, card: &str, phase: &str, repairs: u32) {
    use std::io::Write;
    let ts = crate::receipt::timestamp();
    let line = format!(
        "{{\"card\":\"{card}\",\"phase\":\"{phase}\",\"repairs\":{repairs},\"ts\":\"{ts}\"}}\n"
    );
    // swallow: best-effort-telemetry
    if let Ok(mut f) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("phases.log"))
    {
        let _ = writeln!(f, "{line}"); // swallow: best-effort-telemetry
    }
}

fn write_state(dir: &Path, card: &str, repairs: u32, phase: &str) {
    let _ = fs::create_dir_all(dir); // swallow: checked-elsewhere
                                     // swallow: best-effort-telemetry
    let _ = fs::write(
        dir.join("phase.state"),
        format!("card={card}\nrepairs={repairs}\nphase={phase}\n"),
    );
}

fn read_state(dir: &Path) -> Option<State> {
    let text = fs::read_to_string(dir.join("phase.state")).ok()?;
    let card = kv(&text, "card")?;
    let repairs = kv(&text, "repairs")?.parse().ok()?;
    let phase = kv(&text, "phase")?;
    Some(State {
        card,
        repairs,
        phase,
    })
}

fn kv(text: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}=");
    text.lines()
        .find(|l| l.starts_with(&prefix))
        .map(|l| l.trim_start_matches(&prefix).to_string())
}
