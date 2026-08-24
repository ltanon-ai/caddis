//! body.rs — how a warden decision becomes the ledger row's body, split
//! from main.rs under the 280-line law (the seam: pure text shaping here,
//! envelope/ledger IO there). The body is `tag|command|path|why` and every
//! field's shape is load-bearing, pinned by card.

use caddis_warden::Verdict;

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
pub(crate) fn body_command(command: &str) -> String {
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

/// The fourth field of the body: WHY (CARD-LEDGER-2, the reporter's section 9).
///
/// The reporter re-verified CARD-LEDGER-1 and found the elision could still
/// swallow the judged line above the cap. The durable guarantee is not the
/// head — it is the engine's own explanation: deny reasons name the law id
/// and, for the shell-grammar laws, quote the spelling they fired on; steer
/// carries the law ids. One line, capped, is enough for the row to explain
/// its own refusal even when the head is padding.
pub(crate) fn why_field(verdict: &Verdict) -> String {
    let raw = match verdict {
        Verdict::Deny { reason } => reason.as_str(),
        Verdict::Steer { why, .. } => why.as_str(),
        Verdict::Allow => "",
    };
    raw.lines().next().unwrap_or("").chars().take(160).collect()
}

/// Credential-shaped runs are masked before the command head is persisted
/// (CARD-LEDGER-2). The command is stored so the row explains itself — which
/// means a command carrying a secret would persist that secret at rest. The
/// estate's mask doctrine applies to the audit trail too: vault values print
/// as masks, never as themselves. A run qualifies when it starts with a known
/// credential prefix (at 20+ chars) or is a 32+ char token-charset run; it is
/// replaced by `***redacted(len=N)`. The JUDGEMENT sees the raw command —
/// only the RECORD is masked.
pub(crate) fn mask_at_rest(s: &str) -> String {
    const TOKEN: &[char] = &[
        'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o', 'p', 'q', 'r',
        's', 't', 'u', 'v', 'w', 'x', 'y', 'z', 'A', 'B', 'C', 'D', 'E', 'F', 'G', 'H', 'I', 'J',
        'K', 'L', 'M', 'N', 'O', 'P', 'Q', 'R', 'S', 'T', 'U', 'V', 'W', 'X', 'Y', 'Z', '0', '1',
        '2', '3', '4', '5', '6', '7', '8', '9', '_', '+', '=', '/', '@', ':', '.', '-',
    ];
    const PREFIXES: &[&str] = &[
        "sk-", "ghp_", "gho_", "ghu_", "glpat-", "AKIA", "xoxb-", "xoxp-", "eyJ",
    ];
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0usize;
    while i < chars.len() {
        if TOKEN.contains(&chars[i]) {
            let start = i;
            while i < chars.len() && TOKEN.contains(&chars[i]) {
                i += 1;
            }
            let run: String = chars[start..i].iter().collect();
            let known = PREFIXES.iter().any(|p| run.starts_with(p)) && run.len() >= 20;
            let long = run.len() >= 32;
            if known || long {
                out.push_str(&format!("***redacted(len={})", run.len()));
            } else {
                out.push_str(&run);
            }
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}
