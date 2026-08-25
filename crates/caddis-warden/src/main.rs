//! caddis-warden — the decision binary omp's adapter calls once per tool call.
//!
//! stdin: one request frame (wire.rs). stdout: one JSON verdict. Side effect:
//! one append to the append-only ledger, ALWAYS — allow, steer and deny alike.
//! A warden that records only its refusals cannot answer "what did the agent
//! do last night", which is the question the ledger exists for.
//!
//! STATELESS BY DESIGN. Spawned per call, holding nothing between calls: no
//! process lifecycle to supervise, nothing to leak, and a crash costs exactly
//! one decision instead of the session. The ledger on disk is the only state.
//!
//! ⚠ WHAT THIS DOES NOT DO, stated plainly rather than implied: it does NOT
//! enforce idempotency. `caddis_core::Idempotency` is an in-memory set and a
//! stateless process cannot use it. Replay protection would mean scanning the
//! ledger per call, and for TOOL calls it is not even obviously right — running
//! the same command twice is normal and legitimate. Claiming it here because
//! the kernel has the module would be exactly the kind of unearned "verified"
//! this estate treats as the most expensive failure. Owed as its own card if
//! effect-level idempotency is ever wanted.

mod body;

use body::{body_command, mask_at_rest, why_field};
use caddis_warden::identity::{caller_id, fnv1a, ledger_path, unix_seconds};
use caddis_warden::{
    attest, card, cli, decide, laws, propose, receipt, replay, report, wire, Verdict,
};
use std::io::{Read, Write};
use std::process::ExitCode;

/// ⛔ NOTHING HERE MAY CALL `std::process::exit`, AND THAT IS A MEASUREMENT
/// CONTRACT, not a style preference (CARD-0107). A process that ends that way
/// discards its LLVM coverage counters, so every subcommand dispatched through
/// it contributed NOTHING to coverage — `cargo llvm-cov --test report` ran five
/// passing spawn tests and reported TOTAL 0.00%, `main.rs` included. Returning
/// an `ExitCode` lets the process wind down normally and the counters survive.
/// The observable contract is unchanged: same codes, same streams.
fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if let Some(code) = dispatch(&args) {
        return code;
    }
    let mut buf = Vec::new();
    if std::io::stdin().read_to_end(&mut buf).is_err() {
        fail_closed("warden: could not read the request frame");
        return ExitCode::SUCCESS;
    }
    let call = match wire::parse(&buf) {
        Ok(c) => c,
        // FAIL CLOSED. An unparsable request is not an allowed one: if the
        // warden cannot see what it is judging, the safe answer is no.
        Err(e) => {
            fail_closed(&format!("warden: unreadable request ({e})"));
            return ExitCode::SUCCESS;
        }
    };

    let verdict = decide(&call);
    let seq = record(&call, &verdict);

    let (reason, law) = match &verdict {
        Verdict::Allow => (String::new(), String::new()),
        Verdict::Steer { law, why } => (why.clone(), law.clone()),
        Verdict::Deny { reason } => (reason.clone(), String::new()),
    };
    emit(&wire::reply(verdict.tag(), &reason, &law, seq));
    ExitCode::SUCCESS
}

/// The human-facing arguments, answered without ever touching stdin. `None`
/// means "no argument claimed this invocation" — the frame path, which is the
/// only one an adapter ever takes.
///
/// Anything that looks like an argument is ANSWERED, never fed to the frame
/// parser: falling through is what made `--version` reply with a denial.
fn dispatch(args: &[String]) -> Option<ExitCode> {
    match args.get(1).map(String::as_str) {
        Some("--replay") => Some(exit_code(replay::run(args))),
        Some("report") => Some(exit_code(report::run(args))),
        Some("card") => Some(exit_code(card::run(args))),
        Some("receipt") => Some(exit_code(receipt::run(args))),
        Some("laws") => Some(exit_code(laws::run(args))),
        Some("propose-laws") => Some(exit_code(propose::run(args))),
        Some("attest") => Some(exit_code(attest::run(args))),
        Some(other) => Some(match cli::for_flag(other) {
            cli::Reply::Out(text) => {
                println!("{text}");
                ExitCode::SUCCESS
            }
            cli::Reply::Err(text, code) => {
                eprintln!("{text}");
                exit_code(code)
            }
        }),
        None => None,
    }
}

/// A subcommand's `i32` as a process exit code. Values outside a byte cannot be
/// reported faithfully by any platform here, so they clamp to 1 (failure)
/// rather than wrapping into a code that would read as success.
fn exit_code(code: i32) -> ExitCode {
    match u8::try_from(code) {
        Ok(byte) => ExitCode::from(byte),
        Err(_) => ExitCode::FAILURE,
    }
}

fn emit(s: &str) {
    let out = std::io::stdout();
    let mut h = out.lock();
    // If the reply cannot be written the adapter reads NOTHING, and an
    // unreadable verdict BLOCKS the tool. A lost write therefore degrades to a
    // refusal, never to a silent allow — which is why these two are safe.
    let _ = writeln!(h, "{s}"); // swallow: fail-safe-by-law
    let _ = h.flush(); // swallow: fail-safe-by-law
}

fn fail_closed(reason: &str) {
    emit(&wire::reply("deny", reason, "", 0));
}

/// Append the decision to the ledger. Returns the sequence number, or 0 when
/// the ledger could not be written.
///
/// A LEDGER FAILURE DOES NOT BLOCK THE TOOL, and that is a deliberate ruling
/// rather than an oversight: a full disk or a bad path would otherwise halt all
/// work behind an audit trail. The failure is loud on stderr and the reply
/// carries `seq: 0`, so an unrecorded decision is DETECTABLE rather than
/// disguised. Fail-closed guards the JUDGEMENT; the RECORD fails open and says
/// so.
fn record(call: &caddis_warden::ToolCall, verdict: &Verdict) -> u64 {
    let path = ledger_path();
    let mut led = match caddis_core::ledger::Ledger::open(&path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!(
                "caddis-warden: ledger unavailable at {}: {e}",
                path.display()
            );
            return 0;
        }
    };
    let body = format!(
        "{}|{}|{}|{}",
        verdict.tag(),
        mask_at_rest(&body_command(&call.command)),
        call.path,
        why_field(verdict)
    );
    let id = format!("wardn{:016x}", fnv1a(&call.payload()));
    let idem = format!(
        "{:016x}",
        fnv1a(&format!("{}{}", call.tool, call.payload()))
    );
    let ts = unix_seconds().to_string();
    let env = match caddis_core::envelope::validate(
        1,
        &id,
        &idem,
        &format!("tool.{}", sanitize_type(&call.tool)),
        &caller_id(),
        "warden",
        &body,
        &ts,
    ) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("caddis-warden: envelope refused: {} {}", e.code, e.why);
            return 0;
        }
    };
    match led.append(&env) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("caddis-warden: ledger append failed: {e}");
            0
        }
    }
}

/// The envelope `type` must start with an ASCII letter (envelope.rs); this
/// keeps a future exotic tool name from losing its ledger row.
fn sanitize_type(tool: &str) -> String {
    let cleaned: String = tool
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    match cleaned.chars().next() {
        Some(c) if c.is_ascii_alphabetic() => cleaned,
        _ => format!("x{cleaned}"),
    }
}
