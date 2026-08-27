//! caddis — join binary (CARD-0118 + CARD-0119). One-shot, no daemon.
//!
//! CARD-0 implements `attach`. CARD-1 implements `rotate`.
//! ⛔ NOTHING HERE MAY CALL `std::process::exit` (CARD-0107 coverage contract).

mod attach;
mod bee;
mod drain;
mod fold;
mod harness;
mod hmac;
mod lineage;
mod packet;
mod project;
mod receipt;
mod rotate;
mod session;
mod voice;
mod which;

use std::process::ExitCode;

const USAGE: &str = "\
usage: caddis <attach|rotate|fold|lineage|bee|--help|--version>
       caddis attach --harness omp-peleda|claude|qpi [--skill-src DIR]
       caddis rotate ready --lineage <id> --kind omp|claude|qpi --model <id> [--pane <id>]
       caddis rotate arm --lineage <id>
       caddis rotate verify --lineage <id> [--kind omp|claude|qpi] [--force]
       caddis fold threshold --at <1-99>
       caddis fold tick --lineage <id> --used-pct <0-100>
       caddis lineage packet --lineage <id>
       caddis bee spawn --harness omp|claude|qpi -- <cmd>
";

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
        println!("caddis {}", env!("CARGO_PKG_VERSION"));
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
        "attach" => attach_cmd(&args[1..]),
        "rotate" => rotate_cmd(&args[1..]),
        "fold" => fold_cmd(&args[1..]),
        "lineage" => lineage_cmd(&args[1..]),
        "bee" => bee_cmd(&args[1..]),
        _ => {
            eprint!("{USAGE}");
            ExitCode::from(2)
        }
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

fn bee_cmd(args: &[String]) -> ExitCode {
    match bee::run(args) {
        Ok(0) => ExitCode::SUCCESS,
        Ok(_) => ExitCode::from(1),
        Err(bee::Error::Usage(s)) => usage_fail(&s),
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
