//! identity.rs — WHO is calling and WHERE the ledger is, moved out of the
//! binary so the library can answer both (CARD-0110).
//!
//! `card open` and the frame path must agree about the caller and the ledger
//! file to the byte: a card recorded under one identity and looked up under
//! another is not a card, and a card written to one ledger and read from
//! another is invisible. Two copies of that logic would eventually disagree, so
//! there is one.

/// The calling harness's name, for the ledger's `from` field (CARD-FROM-1).
///
/// One conscience serves several harnesses — omp, little-coder, prime-agent,
/// Claude Code — and the adapter stamps `CADDIS_WARDEN_FROM` so the shared
/// ledger can answer "which of my agents did this", the only question a shared
/// ledger exists to answer. Sanitized like a type name: a hostile value must not
/// corrupt the JSONL row, and an unusable one falls back to "omp" rather than
/// losing the record (the envelope rejects an empty `from`).
///
/// The `.` is what makes CARD-0109 work with no schema change: an adapter that
/// knows its session stamps `<label>.<8-hex>` and it passes through untouched.
pub fn caller_id() -> String {
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

/// Whether this caller names a SESSION rather than just a harness.
///
/// A card held under a bare harness label would bound a different session's
/// writes, because every session in that harness stamps the same string. An
/// adapter that cannot name its session therefore cannot hold a card, and
/// `card open` refuses rather than silently crossing two agents.
pub fn is_session_scoped(caller: &str) -> bool {
    match caller.split_once('.') {
        Some((label, session)) => !label.is_empty() && !session.is_empty(),
        None => false,
    }
}

pub fn ledger_path() -> std::path::PathBuf {
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

/// FNV-1a — stable ids and change detection, NEVER security.
///
/// ⚠ SAY WHAT THIS DOES NOT DO. The card hash it produces detects an ACCIDENTAL
/// edit — a card revised mid-work, a file rewritten by a formatter. It is not
/// collision-resistant and does not detect a deliberate forgery. That is a
/// proportionate choice rather than a gap being papered over: an agent that can
/// rewrite its own card to match a hash can also append whatever it likes to the
/// ledger, so a cryptographic digest here would buy nothing without signing the
/// ledger itself, which is a different and much larger card.
pub fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x1000_0000_01b3);
    }
    h
}

pub fn unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
#[path = "identity_tests.rs"]
mod tests;
