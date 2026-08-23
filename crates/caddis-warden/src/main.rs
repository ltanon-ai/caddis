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

use caddis_warden::{decide, wire, Verdict};
use std::io::{Read, Write};

fn main() {
    let mut buf = Vec::new();
    if std::io::stdin().read_to_end(&mut buf).is_err() {
        return fail_closed("warden: could not read the request frame");
    }
    let call = match wire::parse(&buf) {
        Ok(c) => c,
        // FAIL CLOSED. An unparseable request is not an allowed one: if the
        // warden cannot see what it is judging, the safe answer is no.
        Err(e) => return fail_closed(&format!("warden: unreadable request ({e})")),
    };

    let verdict = decide(&call);
    let seq = record(&call, &verdict);

    let (reason, law) = match &verdict {
        Verdict::Allow => (String::new(), String::new()),
        Verdict::Steer { law, why } => (why.clone(), law.clone()),
        Verdict::Deny { reason } => (reason.clone(), String::new()),
    };
    emit(&wire::reply(verdict.tag(), &reason, &law, seq));
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
        "{}|{}|{}",
        verdict.tag(),
        body_command(&call.command),
        call.path
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

fn ledger_path() -> std::path::PathBuf {
    // An unset override is the NORMAL case here, and the default path below is
    // the lawful fallback rather than an error path.
    // swallow: fail-safe-by-law
    if let Ok(p) = std::env::var("CADDIS_WARDEN_LEDGER") {
        if !p.is_empty() {
            return std::path::PathBuf::from(p);
        }
    }
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());
    std::path::Path::new(&home)
        .join(".caddis")
        .join("warden-ledger.jsonl")
}

/// The calling harness's name, for the ledger's `from` field (CARD-FROM-1).
/// One conscience serves several harnesses — omp, little-coder, prime-agent,
/// Claude Code — and the adapter stamps CADDIS_WARDEN_FROM so the shared
/// ledger can answer "which of my agents did this", the only question a
/// shared ledger exists to answer. Sanitized like a type name: a hostile value
/// must not corrupt the JSONL row, and an unusable one falls back to "omp"
/// rather than losing the record (the envelope rejects an empty `from`).
fn caller_id() -> String {
    let cleaned: String = std::env::var("CADDIS_WARDEN_FROM")
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-' || *c == '.')
        .take(32)
        .collect();
    if cleaned.is_empty() {
        "omp".to_string()
    } else {
        cleaned
    }
}

/// The envelope's `type` must start with an ASCII letter (envelope.rs). omp
/// tool names are well-behaved today; this keeps a future one from being
/// rejected at the ledger and losing the record.
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

/// The command as the ledger row must record it (CARD-LEDGER-1).
///
/// The old first-line-only cut recorded `deny|echo harmless|` for a command
/// whose offence was on line two — the refused text appeared nowhere in its
/// own row, and the row read as a false positive (measured by the fourth
/// harness, DEFECT-ledger-truncates-at-first-newline). Newlines are kept —
/// the ledger JSON-escapes them, and the escaping is already pinned by
/// warden1_ledger_escaping — and a hard byte cap keeps rows bounded. The cap
/// is EXPLICIT: an elided row says so, never masquerading as the whole
/// command.
fn body_command(command: &str) -> String {
    const CAP: usize = 500;
    let mut kept = String::new();
    let mut consumed = 0usize;
    for ch in command.chars() {
        let width = ch.len_utf8();
        if consumed + width > CAP {
            break;
        }
        kept.push(ch);
        consumed += width;
    }
    if consumed < command.len() {
        kept.push_str(&format!("…[+{} bytes truncated]", command.len() - consumed));
    }
    kept
}

fn unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// FNV-1a. Used only to give the envelope a stable id and idem_key — never for
/// security. A cryptographic hash would mean a dependency, and nothing here
/// rests on collision resistance.
fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x1000_0000_01b3);
    }
    h
}
