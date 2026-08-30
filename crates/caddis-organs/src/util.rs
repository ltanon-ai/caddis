//! util.rs — zero-dep helpers shared by the organs (wave 1).
//! JSON string escaping, wall-clock, ISO-8601 from civil days.
//! No allocation beyond the returned buffer; no panics on unknown input.

/// Escape a string for embedding in a JSON document (RFC 8259 minimal set).
/// Control chars become \\u00XX; quotes and backslashes are escaped.
pub fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

/// Wall clock in milliseconds since the Unix epoch (0 on clock failure).
pub fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub use crate::util_time::{
    iso8601_from_unix, iso8601_from_unix_vilnius, iso8601_now, unix_from_iso8601,
    vilnius_offset_secs,
};

/// Extract `"key":"value"` from a flat one-line JSON object (unescapes
/// \\n, \\r, \\t and \\\\ pairs). None when the key is absent.
pub(crate) fn json_str_field(line: &str, key: &str) -> Option<String> {
    let pat = format!("\"{key}\":\"");
    let start = line.find(&pat)? + pat.len();
    scan_json_string(&line[start..])
}

/// Extract `"key":["a","b",...]` as the raw unescaped strings.
/// None when the key is absent; empty vec for an empty array.
pub(crate) fn json_str_array_field(line: &str, key: &str) -> Option<Vec<String>> {
    let pat = format!("\"{key}\":[");
    let start = line.find(&pat)? + pat.len();
    let rest = &line[start..];
    let mut out = Vec::new();
    let mut i = 0;
    let bytes = rest.as_bytes();
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                let s = scan_json_string(&rest[i + 1..])?;
                out.push(s);
                // advance past the closing quote: rescan char by char
                let mut j = i + 1;
                let mut esc = false;
                while j < bytes.len() {
                    if esc {
                        esc = false;
                    } else if bytes[j] == b'\\' {
                        esc = true;
                    } else if bytes[j] == b'"' {
                        break;
                    }
                    j += 1;
                }
                i = j + 1;
            }
            b']' => return Some(out),
            _ => i += 1,
        }
    }
    None
}

/// Scan one JSON string body (starting AFTER the opening quote) up to the
/// closing unescaped quote.
fn scan_json_string(s: &str) -> Option<String> {
    let mut out = String::new();
    let mut esc = false;
    for c in s.chars() {
        if esc {
            out.push(match c {
                'n' => '\n',
                'r' => '\r',
                't' => '\t',
                other => other,
            });
            esc = false;
            continue;
        }
        match c {
            '\\' => esc = true,
            '"' => return Some(out),
            c => out.push(c),
        }
    }
    None
}

/// The estate's build-stable hash. BYTE-IDENTICAL to
/// `caddis-warden/src/identity.rs::fnv1a` — including its prime literal
/// `0x1000_0000_01b3`, which carries one more zero than published
/// FNV-1a (0x100000001b3). Kept that way ON PURPOSE: the warden has
/// persisted id/idem/fp values under this literal, so the estate's one
/// hash law is THIS function, and "stable across builds" (never
/// `DefaultHasher`) is the requirement — published-vector parity is not
/// (CARD-0228).
pub fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x1000_0000_01b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_covers_the_json_minimal_set() {
        assert_eq!(json_escape("plain"), "plain");
        assert_eq!(json_escape("a\"b\\c"), "a\\\"b\\\\c");
        assert_eq!(json_escape("l1\nl2\tl3\rl4"), "l1\\nl2\\tl3\\rl4");
        assert_eq!(json_escape("\u{1}"), "\\u0001");
    }

    #[test]
    fn fnv1a_estate_vectors() {
        // Pinned to the ESTATE algorithm (see fnv1a docs): any change to
        // basis, prime, or order breaks these — including a "fix" to the
        // published FNV prime, which would fork the estate hash law.
        assert_eq!(fnv1a(""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a("a"), 0xaf74_d84c_8601_ec8c);
        assert_eq!(fnv1a("foobar"), 0xf8ac_2471_f739_67e8);
    }

    #[test]
    fn field_roundtrip_including_windows_paths() {
        let path = "E:\\work\\caddis";
        let line = format!("{{\"p\":\"{}\"}}", json_escape(path));
        assert_eq!(json_str_field(&line, "p").as_deref(), Some(path));
    }

    #[test]
    fn array_roundtrip_and_empty() {
        let line = "{\"files\":[\"a\",\"b\"]}";
        assert_eq!(
            json_str_array_field(line, "files"),
            Some(vec!["a".into(), "b".into()])
        );
        assert_eq!(
            json_str_array_field("{\"files\":[]}", "files"),
            Some(vec![])
        );
        assert_eq!(json_str_array_field(line, "nope"), None);
    }

    #[test]
    fn epoch_and_known_dates() {
        assert_eq!(iso8601_from_unix(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso8601_from_unix(951_782_400), "2000-02-29T00:00:00Z"); // leap day
        assert_eq!(iso8601_from_unix(1_756_028_519), "2025-08-24T09:41:59Z");
        assert_eq!(iso8601_from_unix(86_399), "1970-01-01T23:59:59Z");
    }

    #[test]
    fn negative_seconds_stay_total() {
        // Pre-epoch: correct calendar math, never a panic.
        assert_eq!(iso8601_from_unix(-1), "1969-12-31T23:59:59Z");
        assert_eq!(iso8601_from_unix(-86_400), "1969-12-31T00:00:00Z");
    }
}
