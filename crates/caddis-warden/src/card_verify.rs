//! card_verify.rs — CARD-0128: `card close --verify -- <cmd>`.
//! CARD-0133: a successful verify+close writes `{card}.attest.json`.
//!
//! Executes the command, appends `card.verify` with the exit code, and
//! only then closes. A nonzero command leaves the card open.

use std::process::Command;

use crate::card::{append, changed_since_open, fail, read_state};
use crate::card_state::{self, CLOSE_TYPE, VERIFY_TYPE};

pub fn close(rest: &[String]) -> i32 {
    let attest = match rest.first().map(String::as_str) {
        Some("--verify") => match verify(&rest[1..]) {
            Ok(pair) => Some(pair),
            Err(code) => return code,
        },
        Some(_) => return fail("unknown close flag; use close or close --verify -- <cmd>"),
        None => None,
    };
    let code = close_plain();
    if code != 0 {
        return code;
    }
    match attest {
        Some((id, path)) => write_attest(&id, &path),
        None => 0,
    }
}

fn verify(rest: &[String]) -> Result<(String, String), i32> {
    let argv = strip_ddash(rest);
    if argv.is_empty() {
        return Err(fail("card close --verify needs `-- <cmd>`"));
    }
    let (_, caller, state) = read_state()?;
    let Some(active) = state.active else {
        return Err(fail(&format!("no card is open for {caller}")));
    };
    if let Some(why) = changed_since_open(&active) {
        return Err(fail(&why));
    }
    let status = Command::new(&argv[0]).args(&argv[1..]).status();
    let exit = match status {
        Ok(s) => s.code().unwrap_or(1),
        Err(e) => {
            eprintln!("card verify: cannot run {}: {e}", argv[0]);
            return Err(1);
        }
    };
    let body = card_state::body("verify", &active.id, &exit.to_string(), &active.hash);
    match append(VERIFY_TYPE, &body, &caller) {
        Ok(seq) => println!("card verify: {} exit={exit} seq={seq}", active.id),
        Err(why) => return Err(fail(&why)),
    }
    if exit != 0 {
        return Err(exit);
    }
    Ok((active.id, active.path))
}

fn strip_ddash(rest: &[String]) -> &[String] {
    match rest.first().map(String::as_str) {
        Some("--") => &rest[1..],
        _ => rest,
    }
}

fn write_attest(id: &str, card_path: &str) -> i32 {
    let Some(text) = crate::propose::read_ledger("attest") else {
        return 2;
    };
    match crate::attest::build(text.as_str(), id) {
        Ok(b) => {
            let dest = std::path::Path::new(card_path).with_extension("attest.json");
            match std::fs::write(&dest, crate::attest_verify::render_json(&b)) {
                Ok(()) => {
                    println!("card attest: {}", dest.display());
                    0
                }
                Err(e) => fail(&format!("cannot write {}: {e}", dest.display())),
            }
        }
        Err(why) => fail(&why),
    }
}

fn close_plain() -> i32 {
    let (_, caller, state) = match read_state() {
        Ok(v) => v,
        Err(code) => return code,
    };
    let Some(active) = state.active else {
        return fail(&format!("no card is open for {caller}"));
    };
    if let Some(why) = changed_since_open(&active) {
        return fail(&why);
    }
    match append(
        CLOSE_TYPE,
        &card_state::body("close", &active.id, &active.path, &active.hash),
        &caller,
    ) {
        Ok(seq) => {
            println!("card close: {} seq={seq}", active.id);
            println!(
                "  note: this ledger cannot prove the RED-TEST passed — it records \
                 intent, not results. `card close --verify -- <cmd>` is the proof."
            );
            0
        }
        Err(why) => fail(&why),
    }
}
