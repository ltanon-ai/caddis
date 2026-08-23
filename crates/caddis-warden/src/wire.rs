//! wire.rs — the protocol between omp's adapter and the warden binary.
//!
//! ASYMMETRIC ON PURPOSE, and the asymmetry is the safety argument:
//!
//!   IN  (JS -> Rust): LENGTH-PREFIXED frames. A tool payload is arbitrary
//!        bytes — a heredoc, a regex, JSON, a NUL. Any separator-based framing
//!        has a payload that breaks it, and a framing bug in the SECURITY path
//!        fails open (the warden mis-reads a command and waves it through).
//!        A byte count cannot be spoofed by content.
//!   OUT (Rust -> JS): JSON, because we are the producer and `JSON.parse` on
//!        the far side is a standard-library call.
//!
//! So neither side hand-writes a PARSER for a format it does not control.
//! caddis-warden carries no dependencies (canon D-023), and a hand-rolled JSON
//! parser in the deny path is exactly the kind of code that should not exist.
//!
//! FRAME (each field, in fixed order: tool, command, path, content):
//!     <name> <byte-len>\n
//!     <exactly byte-len bytes>\n

use crate::ToolCall;

const FIELDS: [&str; 4] = ["tool", "command", "path", "content"];

/// Parse one request frame. `Err` carries a human reason; the caller must FAIL
/// CLOSED on it — an unparseable request is not an allowed one.
pub fn parse(buf: &[u8]) -> Result<ToolCall, String> {
    let mut pos = 0usize;
    let mut vals: Vec<String> = Vec::with_capacity(4);
    for want in FIELDS {
        let (val, next) = read_field(buf, pos, want)?;
        vals.push(val);
        pos = next;
    }
    Ok(ToolCall::new(&vals[0])
        .command(&vals[1])
        .path(&vals[2])
        .content(&vals[3]))
}

/// One `<name> <len>\n<bytes>\n` record. Split from its header parsing because
/// the combined form measured CCN 11 against the repo's cap of 10 — the gate
/// caught it, and the fix is a split, never a trim.
fn read_field(buf: &[u8], start: usize, want: &str) -> Result<(String, usize), String> {
    let (len, body_start) = parse_header(buf, start, want)?;
    read_body(buf, body_start, len, want)
}

/// `<name> <len>\n` -> (len, index just past the newline).
fn parse_header(buf: &[u8], start: usize, want: &str) -> Result<(usize, usize), String> {
    let nl = find_nl(buf, start).ok_or_else(|| format!("frame: no header for `{want}`"))?;
    let header = std::str::from_utf8(&buf[start..nl]).map_err(|_| "frame: header not utf-8")?;
    let (name, len_s) = header
        .split_once(' ')
        .ok_or_else(|| format!("frame: malformed header `{header}`"))?;
    if name != want {
        return Err(format!("frame: expected `{want}`, got `{name}`"));
    }
    let len = len_s
        .trim()
        .parse::<usize>()
        .map_err(|_| format!("frame: bad length for `{want}`"))?;
    Ok((len, nl + 1))
}

/// Exactly `len` bytes from `body_start`, refusing anything that would read
/// past the buffer — a length is attacker-adjacent input, not a promise.
fn read_body(
    buf: &[u8],
    body_start: usize,
    len: usize,
    want: &str,
) -> Result<(String, usize), String> {
    let body_end = body_start
        .checked_add(len)
        .filter(|e| *e <= buf.len())
        .ok_or_else(|| format!("frame: `{want}` length {len} runs past the buffer"))?;
    let val = String::from_utf8_lossy(&buf[body_start..body_end]).into_owned();
    // The trailing newline is optional so a final field needs no padding.
    let next = if buf.get(body_end) == Some(&b'\n') {
        body_end + 1
    } else {
        body_end
    };
    Ok((val, next))
}

fn find_nl(buf: &[u8], from: usize) -> Option<usize> {
    (from..buf.len()).find(|i| buf[*i] == b'\n')
}

/// Escape for a JSON string literal. Same law as the ledger's: the two
/// structural characters, JSON's five short forms, and every remaining C0
/// control as `\u00XX`. A raw control byte here would produce a reply that
/// `JSON.parse` rejects — and the adapter FAILS CLOSED on that, so a sloppy
/// escape would turn a plain `allow` into a blocked tool.
pub fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
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

/// Build the reply the adapter reads.
pub fn reply(verdict: &str, reason: &str, law: &str, seq: u64) -> String {
    format!(
        "{{\"verdict\":\"{}\",\"reason\":\"{}\",\"law\":\"{}\",\"seq\":{}}}",
        json_escape(verdict),
        json_escape(reason),
        json_escape(law),
        seq
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(tool: &str, command: &str, path: &str, content: &str) -> Vec<u8> {
        let mut v = Vec::new();
        for (n, val) in [
            ("tool", tool),
            ("command", command),
            ("path", path),
            ("content", content),
        ] {
            v.extend_from_slice(format!("{n} {}\n", val.len()).as_bytes());
            v.extend_from_slice(val.as_bytes());
            v.push(b'\n');
        }
        v
    }

    #[test]
    fn a_payload_containing_the_framing_characters_survives() {
        // The whole reason for length prefixes: this content contains newlines
        // AND text that looks exactly like a field header.
        let nasty = "echo one\ncontent 5\nnot-a-field\n\"quotes\" \\slashes";
        let f = frame("bash", nasty, "", "");
        let call = parse(&f).expect("parse");
        assert_eq!(call.tool, "bash");
        assert_eq!(call.command, nasty, "the payload must survive verbatim");
    }

    #[test]
    fn a_truncated_frame_is_an_error_not_a_silent_allow() {
        let mut f = frame("bash", "rm -rf /", "", "");
        f.truncate(f.len() / 2);
        assert!(
            parse(&f).is_err(),
            "a short frame must FAIL, never parse-partial"
        );
    }

    #[test]
    fn a_length_running_past_the_buffer_is_refused() {
        let f = b"tool 9999\nbash\n".to_vec();
        assert!(
            parse(&f).is_err(),
            "an overlong length must not panic or read past"
        );
    }

    #[test]
    fn the_reply_escapes_control_characters() {
        let r = reply("deny", "line one\nline \"two\"", "", 7);
        assert!(!r.contains('\n'), "a raw newline breaks JSON.parse: {r}");
        assert!(r.contains("\\n") && r.contains("\\\""), "{r}");
        assert!(r.contains("\"seq\":7"), "{r}");
    }
}
