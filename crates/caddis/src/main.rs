//! caddis — join binary (CARD-0118 + CARD-0119). One-shot, no daemon.
//! ⛔ NOTHING HERE MAY CALL `std::process::exit` (CARD-0107 coverage contract).

mod akis;
mod akis_json;
mod attach;
mod bee;
mod beekeeper;
mod occupancy;
mod doctor;
mod drain;
mod eddy_nerve;
mod eddy_nerve_io;
mod fold;
mod harness;
mod hmac;
mod lease;
mod ledger;
mod lineage;
mod pace;
mod packet;
mod packet_tail;
mod page;
mod page_mark;
mod page_mode;
mod page_report;
mod page_report_tally;
mod page_report_usage;
mod panic;
mod project;
mod prokuratura;
mod prokuratura_fix;
mod prove;
mod receipt;
mod restart;
mod rotate;
mod sentinel;
mod sentinel_engine;
mod sentinel_post;
mod session;
mod soul_cli;
mod soul_writer;
mod usage;
mod voice;
mod which;
mod worker;
mod worker_board;
mod worker_board_frame;
mod worker_board_over;
mod worker_board_sections;
mod worker_board_state;
mod worker_board_tail;
mod worker_dash;
mod worker_done;
mod worker_fsm;
mod worker_lock;
mod worker_phase;
mod worker_reach;
mod worker_scan;
use std::process::ExitCode;
pub(crate) use usage::USAGE;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    dispatch(&args)
}

fn dispatch(args: &[String]) -> ExitCode {
    if let Some(code) = meta(args) {
        return code;
    }
    run_cmd(args)
}
fn meta(args: &[String]) -> Option<ExitCode> {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print!("{USAGE}");
        return Some(ExitCode::SUCCESS);
    }
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!(
            "caddis {}-{}",
            env!("CARGO_PKG_VERSION"),
            env!("CADDIS_GIT_HASH")
        );
        return Some(ExitCode::SUCCESS);
    }
    if args.is_empty() {
        eprint!("{USAGE}");
        return Some(ExitCode::from(2));
    }
    None
}

fn run_cmd(args: &[String]) -> ExitCode {
    match args[0].as_str() {
        "beekeeper" => beekeeper_cmd(&args[1..]),
        "eddy" => eddy_cmd(&args[1..]),
        "attach" => attach_cmd(&args[1..]),
        "rotate" => rotate_cmd(&args[1..]),
        "fold" => fold_cmd(&args[1..]),
        "lineage" => lineage_cmd(&args[1..]),
        "page" => page_cmd(&args[1..]),
        "bee" => bee_cmd(&args[1..]),
        "occupancy" => occupancy_cmd(&args[1..]),
        "ledger" => ledger_cmd(&args[1..]),
        "check" => check_cmd(&args[1..]),
        "panic" => match panic::run(&args[1..]) {
            Ok(()) => ExitCode::SUCCESS,
            Err(panic::Error::Usage(s)) => usage_fail(&s),
            Err(e) => fail(&e.to_string()),
        },
        "prove" => prove::cmd(&args[1..]),
        "sentinel" => sentinel::cmd(&args[1..]),
        "worker" => worker_cmd(&args[1..]),
        "brief" | "fix" | "build" => prokuratura_cmd(args),
        "soul" => soul_cmd(&args[1..]),
        "akis" => akis_cmd(&args[1..]),
        "restart" => restart_cmd(&args[1..]),
        "doctor" => match doctor::run(&args[1..]) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => fail(&e.to_string()),
        },
        _ => {
            eprint!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn eddy_cmd(args: &[String]) -> ExitCode {
    // Exit code IS the nerve contract: 0 continue/stagnant, 3 halt,
    // 2 fail-closed/usage (CARD-0233).
    match eddy_nerve::run(args) {
        Ok(code) => ExitCode::from(code.clamp(0, 255) as u8),
        Err(eddy_nerve::Error::Usage(s)) => usage_fail(&s),
        Err(eddy_nerve::Error::Closed(s)) => {
            eprintln!("{s}");
            ExitCode::from(2)
        }
    }
}

fn beekeeper_cmd(args: &[String]) -> ExitCode {
    match beekeeper::run(args) {
        Ok(code) => ExitCode::from(code.clamp(0, 255) as u8),
        Err(beekeeper::Error::Usage(s)) => usage_fail(&s),
    }
}
fn attach_cmd(args: &[String]) -> ExitCode {
    match attach::run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(attach::Error::Usage(s)) => usage_fail(&s),
        Err(e) => fail(&e.to_string()),
    }
}

fn rotate_cmd(args: &[String]) -> ExitCode {
    match rotate::run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(rotate::Error::Usage(s)) => usage_fail(&s),
        Err(e) => fail(&e.to_string()),
    }
}

fn fold_cmd(args: &[String]) -> ExitCode {
    match fold::run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(fold::Error::Usage(s)) => usage_fail(&s),
        Err(fold::Error::Deny) => ExitCode::from(1),
        Err(e) => fail(&e.to_string()),
    }
}

fn lineage_cmd(args: &[String]) -> ExitCode {
    match packet::run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(packet::Error::Usage(s)) => usage_fail(&s),
        Err(e) => fail(&e.to_string()),
    }
}

fn page_cmd(args: &[String]) -> ExitCode {
    match page::run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(page::Error::Usage(s)) => usage_fail(&s),
        Err(e) => fail(&e.to_string()),
    }
}

fn bee_cmd(args: &[String]) -> ExitCode {
    match bee::run(args) {
        Ok(0) => ExitCode::SUCCESS,
        Ok(_) => ExitCode::from(1),
        Err(bee::Error::Usage(s)) => usage_fail(&s),
        Err(e) => fail(&e.to_string()),
    }
}

fn occupancy_cmd(args: &[String]) -> ExitCode {
    match occupancy::run(args) {
        Ok(0) => ExitCode::SUCCESS,
        Ok(_) => ExitCode::from(1),
        Err(occupancy::Error::Usage(s)) => usage_fail(&s),
        Err(e) => fail(&e.to_string()),
    }
}

fn ledger_cmd(args: &[String]) -> ExitCode {
    match ledger::run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => fail(&e),
    }
}

fn check_cmd(args: &[String]) -> ExitCode {
    match pace::run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(pace::Error::Usage(s)) => usage_fail(&s),
        Err(e) => fail(&e.to_string()),
    }
}

fn worker_cmd(args: &[String]) -> ExitCode {
    match worker::run(args) {
        Ok(0) => ExitCode::SUCCESS,
        Ok(n) => ExitCode::from(n.clamp(1, 255) as u8),
        Err(worker::Error::Usage(s)) => usage_fail(&s),
        Err(e) => fail(&e.to_string()),
    }
}
fn prokuratura_cmd(args: &[String]) -> ExitCode {
    match prokuratura::run(args) {
        Ok(0) => ExitCode::SUCCESS,
        Ok(n) => ExitCode::from(n.clamp(1, 255) as u8),
        Err(prokuratura::Error::Usage(s)) => usage_fail(&s),
        Err(e) => fail(&e.to_string()),
    }
}
fn soul_cmd(args: &[String]) -> ExitCode {
    match soul_cli::run(args) {
        Ok(0) => ExitCode::SUCCESS,
        Ok(n) => ExitCode::from(n.clamp(1, 255) as u8),
        Err(soul_cli::Error::Usage(s)) => usage_fail(&s),
        Err(e) => fail(&e.to_string()),
    }
}
fn akis_cmd(args: &[String]) -> ExitCode {
    match akis::run(args) {
        Ok(0) => ExitCode::SUCCESS,
        Ok(n) => ExitCode::from(n.clamp(1, 255) as u8),
        Err(akis::Error::Usage(s)) => usage_fail(&s),
        Err(e) => fail(&e.to_string()),
    }
}
fn restart_cmd(args: &[String]) -> ExitCode {
    match restart::run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(restart::Error::Usage(s)) => usage_fail(&s),
        Err(e) => fail(&e.to_string()),
    }
}
fn usage_fail(s: &str) -> ExitCode {
    eprintln!("{s}");
    eprint!("{USAGE}");
    ExitCode::from(2)
}

fn fail(s: &str) -> ExitCode {
    eprintln!("{s}");
    ExitCode::from(1)
}
