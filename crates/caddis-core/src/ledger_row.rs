//! ledger_row.rs — how one ledger row is ENCODED and BOUNDED, split from
//! ledger.rs under the 280-line law.
//!
//! The seam matches the one that already separated the lock: `ledger.rs` is
//! about the RECORD, `ledger_lock.rs` is about the EXCLUSION, and this file is
//! about the BYTES — escaping, the size the atomicity guarantee rests on, and
//! the elision that makes an oversized field fit while saying that it did.
//!
//! It lives here rather than in `ledger.rs` because the guarantee and the
//! encoding are one subject: the cap is only meaningful in terms of the escaped
//! form, and the escaped form is only bounded because of the cap.

use crate::envelope::Envelope;

/// CARD-WARDEN-1: escape a string for a JSON string literal.
///
/// The v0 line escaped `\` and `"` only, which held for as long as every body
/// was a short ASCII test string. A warden body carries real tool input — a
/// multi-line bash command, file content — and a RAW newline inside a JSONL
/// record ENDS the record: one append reads back as two lines, the second
/// unparsable, and `open` then recovers `seq` from a fragment.
///
/// Escapes the two structural characters, the five short forms JSON defines,
/// and every remaining C0 control as `\u00XX`. Nothing above U+001F needs
/// escaping in JSON, so text and emoji pass through as themselves (the file is
/// UTF-8 by contract — see rules/common/encoding.md).
pub(crate) fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// The row size the atomicity claim actually rests on.
///
/// A single `write_all` of this much is one syscall on every platform we ship.
/// Above it `write_all` LOOPS, and a loop is where tearing returns -- so the
/// guarantee `append` makes is only true for rows that fit, which makes
/// `append` the place that has to make them fit.
const ROW_CAP: usize = 4096;

/// Per-field cap for the ENVELOPE METADATA (id, idem_key, type, from, to, ts).
///
/// Capping only the body bounded the BODY, not the ROW. `envelope::validate`
/// puts no upper bound on any of these fields either, so a caller with a
/// one-megabyte `from` wrote past ROW_CAP however small its body was — the same
/// unenforced-guarantee shape one level out. Six fields at 256 escaped bytes
/// plus the JSON skeleton still leaves the body over 2 KB, so the row is now
/// bounded BY CONSTRUCTION rather than by a check a later edit could drift past.
///
/// Truncating an id is lossy in a way truncating a body is not — a cut id is a
/// WRONG id — so it carries the same visible marker. A field long enough to
/// reach this is pathological, and a pathological row that SAYS it was cut
/// beats a torn one that does not.
const FIELD_CAP: usize = 256;

/// Cut `body` so its ESCAPED form fits `budget`, saying so when it does.
///
/// Budgeted on the ESCAPED length, never the raw one: `esc` can turn one byte
/// into six (`\u001f`), so a raw-byte budget is not a row-byte budget. That is
/// the same class of mistake as the comment this function exists to retire.
pub(crate) fn elide(body: &str, budget: usize) -> String {
    if esc(body).len() <= budget {
        return body.to_string();
    }
    // Room for the marker itself, escaped. Generous on purpose: overshooting
    // costs a few characters of a body that is already being truncated, while
    // undershooting would push the row back over the cap.
    let keep_to = budget.saturating_sub(48);
    let mut kept = String::new();
    let mut used = 0usize;
    for c in body.chars() {
        let w = esc(&c.to_string()).len();
        if used + w > keep_to {
            break;
        }
        kept.push(c);
        used += w;
    }
    let dropped = body.len() - kept.len();
    format!("{kept}…[+{dropped} bytes truncated]")
}

/// The finished JSONL row for `seq` and `env`, bounded by construction.
///
/// Every field is elided before it is escaped, so the row cannot exceed
/// `ROW_CAP` for ANY envelope: six metadata fields at `FIELD_CAP` plus the JSON
/// skeleton leave the body over 2 KB of budget. The body is measured LAST,
/// against whatever the head and tail actually consumed, rather than against an
/// assumed constant.
pub(crate) fn row_for(seq: u64, env: &Envelope) -> String {
    let head = format!(
        "{{\"seq\":{},\"v\":{},\"id\":\"{}\",\"idem_key\":\"{}\",\"type\":\"{}\",\"from\":\"{}\",\"to\":\"{}\",\"body\":\"",
        seq,
        env.v,
        esc(&elide(&env.id, FIELD_CAP)),
        esc(&elide(&env.idem_key, FIELD_CAP)),
        esc(&elide(&env.r#type, FIELD_CAP)),
        esc(&elide(&env.from, FIELD_CAP)),
        esc(&elide(&env.to, FIELD_CAP))
    );
    let tail = format!(
        "\",\"ts\":\"{}\"}}
",
        esc(&elide(&env.ts, FIELD_CAP))
    );
    let budget = ROW_CAP.saturating_sub(head.len() + tail.len());
    format!("{head}{}{tail}", esc(&elide(&env.body, budget)))
}
