//! incident_log.rs — reading the outcome-watch log, with no JSON library.
//!
//! Split out of `incidents.rs` under the repo's 280-line file law, at the seam
//! the module already had: this file answers "what incidents are open", and
//! `incidents.rs` answers "does this command act on one of them". Both audits
//! that hit this area landed on THIS half — a value scanner that grabbed the
//! next key's name for a `null`, and a resolved-flag test that a nested key
//! could win — so the parsing deserves its own door.
//!
//! ⚠ NO JSON LIBRARY. This crate carries zero third-party dependencies, so the
//! few fields needed are lifted out by hand. A line that does not yield them is
//! SKIPPED — that under-blocks a real incident rather than inventing one, and
//! the incident stays loud in the session banner regardless.

use std::path::PathBuf;

/// One unresolved history-rewrite incident, reduced to what the finding needs.
pub struct Incident {
    pub repo: String,
    pub reference: String,
    pub old: String,
    pub new: String,
}

fn log_path() -> PathBuf {
    match std::env::var("OUTCOME_WATCH_LOG") {
        Ok(p) => PathBuf::from(p),
        Err(_) => {
            let home = std::env::var("USERPROFILE")
                .or_else(|_| std::env::var("HOME"))
                .unwrap_or_else(|_| ".".to_string());
            PathBuf::from(home)
                .join(".claude")
                .join("hooks")
                .join(".outcome-incidents.jsonl")
        }
    }
}

/// The string value of `"<key>":` on this line, JSON escapes undone.
///
/// ⛔ AUDIT FINDING 4, AND IT WAS LIVE AGAINST REAL DATA. The old version did
/// `after.find('"')` to locate the value — but for `"new": null` there is no
/// quote after the value, so it skipped ahead and grabbed THE NEXT KEY'S OWN
/// NAME. Both currently-unresolved incidents on disk carry exactly
/// `"new": null, "verdict": "vanished"`, so the very next real trigger of this
/// HARD check would have reported *"655f64d2 is not an ancestor of verdict"* —
/// a fabricated SHA, in a denial, presented as measurement.
///
/// That is the "correct verdict, wrong reason" failure this crate's own
/// `bypasses_signing` doc holds up as the thing to avoid, committed by the file
/// that quotes it. A non-string value must read as ABSENT: the value begins at
/// the first non-space character after the colon, and if that character is not
/// a quote there is no string here.
fn field(line: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let at = line.find(&needle)?;
    let rest = &line[at + needle.len()..];
    let colon = rest.find(':')?;
    let after = rest[colon + 1..].trim_start();
    let body = after.strip_prefix('"')?;
    read_json_string(body)
}

/// Read one JSON string body up to its unescaped closing quote.
fn read_json_string(body: &str) -> Option<String> {
    let mut out = String::new();
    let mut chars = body.chars();
    while let Some(c) = chars.next() {
        if c == '"' {
            return Some(out);
        }
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some(other) => out.push(other),
            None => return None,
        }
    }
    None
}

/// True when the line carries a TOP-LEVEL `"resolved": true`.
///
/// ⛔ AUDIT 2, FINDING 4. The old version took the first `"resolved"` anywhere on
/// the line and asked whether the next 30 raw characters contained `true`. Two
/// ways that drops a genuinely OPEN incident — and dropping one means a push
/// into a rewritten repo is silently ALLOWED, the failure direction that matters:
///
///   {"resolved": false, "x": "true", ...}          -> read as resolved
///   {"note":{"resolved":true},"resolved":false,..} -> the NESTED key wins
///
/// Now it walks the line tracking brace depth and quotes, considers only a key
/// at depth 1, and compares the literal VALUE token rather than sniffing for a
/// substring nearby. Not currently triggered by the real log — found by
/// construction, which is the only way this class gets found before it bites.
fn is_resolved(line: &str) -> bool {
    match top_level_value(line, "resolved") {
        Some(v) => v == "true",
        None => false,
    }
}

/// The literal value of a TOP-LEVEL key, as written (`true`, `false`, `null`, a
/// number). Returns `None` for a string value or a key that is not at depth 1.
fn top_level_value(line: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let chars: Vec<char> = line.chars().collect();
    let mut depth = 0usize;
    let mut i = 0usize;
    while i < chars.len() {
        match chars[i] {
            '{' | '[' => depth += 1,
            '}' | ']' => depth = depth.saturating_sub(1),
            '"' => {
                if depth == 1 {
                    if let Some(value) = value_if_key(line, &chars, i, &needle) {
                        return Some(value);
                    }
                }
                // A quoted run is skipped WHOLE, so a key name sitting inside a
                // VALUE can never be mistaken for a key. Jumping to the closing
                // quote replaces the in_string flag this loop used to carry:
                // the state machine was the whole of its complexity.
                i = run_close(&chars, i)?;
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// The literal value when the quoted run opening at `open` IS the wanted key.
///
/// A KEY is a quoted run followed by a colon. Without the colon test, a depth-1
/// string VALUE whose text equals the key (`"state": "resolved"`) matches the
/// needle first and shadows the real key — LOW 7: a resolved incident kept
/// denying. The colon is what tells a key from a value.
fn value_if_key(line: &str, chars: &[char], open: usize, needle: &str) -> Option<String> {
    if !line[byte_at(chars, open)..].starts_with(needle) {
        return None;
    }
    let close = run_close(chars, open)?;
    if !followed_by_colon(chars, close + 1) {
        return None;
    }
    read_literal(chars, close + 1)
}

/// Index of the closing quote of the run opening at `open`, escape-aware, or
/// `None` when it never closes.
fn run_close(chars: &[char], open: usize) -> Option<usize> {
    let mut i = open + 1;
    while i < chars.len() {
        if chars[i] == '\\' {
            i += 2;
            continue;
        }
        if chars[i] == '"' {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Is the first non-whitespace character at or after `from` a colon?
fn followed_by_colon(chars: &[char], from: usize) -> bool {
    let mut i = from;
    while i < chars.len() && chars[i].is_whitespace() {
        i += 1;
    }
    chars.get(i) == Some(&':')
}

/// Byte offset of char index `i` — the slice comparison above needs bytes.
fn byte_at(chars: &[char], i: usize) -> usize {
    chars[..i].iter().map(|c| c.len_utf8()).sum()
}

/// Skip `: ` then read the bare literal that follows.
fn read_literal(chars: &[char], from: usize) -> Option<String> {
    let mut i = from;
    while i < chars.len() && (chars[i].is_whitespace() || chars[i] == ':') {
        i += 1;
    }
    if i >= chars.len() || chars[i] == '"' {
        return None;
    }
    let mut out = String::new();
    while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '.' || chars[i] == '-') {
        out.push(chars[i]);
        i += 1;
    }
    Some(out)
}

/// Unresolved rewrite incidents. Unreadable state yields NONE, never an error.
///
/// Fail-open is a deliberate trade: this runs ahead of the operator's tool call,
/// and a corrupt log must not make their session unusable. The same state is
/// reported independently by the session banner, so a read failure here is loud
/// somewhere else rather than silent everywhere.
pub fn open_incidents() -> Vec<Incident> {
    match std::fs::read_to_string(log_path()) {
        Ok(text) => open_incidents_from(&text),
        Err(_) => Vec::new(),
    }
}

/// The PURE half: log text in, unresolved incidents out.
///
/// Split from the read so the parsing can be tested without touching the
/// filesystem or the process environment. A test that has to set
/// `OUTCOME_WATCH_LOG` mutates global state shared with every other test in the
/// binary, and tests that fight each other over an env var fail in a pattern
/// nobody can reproduce on demand.
pub fn open_incidents_from(text: &str) -> Vec<Incident> {
    text.lines()
        .filter(|line| !is_resolved(line))
        .filter_map(|line| {
            field(line, "repo").map(|repo| Incident {
                repo,
                reference: field(line, "ref").unwrap_or_default(),
                old: field(line, "old").unwrap_or_default(),
                new: field(line, "new").unwrap_or_default(),
            })
        })
        .collect()
}
