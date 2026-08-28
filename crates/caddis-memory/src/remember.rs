//! remember.rs — the WRITE HALF of the memory organ (P3 `caddis-remember`,
//! quorum-ratified 2026-08-26, verdict in OMP state/briefs/caddis-remember-quorum/).
//!
//! Slice (a) scope (this file's first landing): the byte-level contract the
//! whole gate family stands on —
//! - **I1 hash-bound full payload:** the warden frame's `content` is the
//!   byte-exact doc that lands on disk, minus the ONLY two fields that may
//!   derive post-verdict (`warden_seq`, `warden_tx_hash`). The audit leg
//!   strips those stamps, re-renders, and must get the draft bytes back.
//! - **I1+ deterministic serialization:** frontmatter is a `BTreeMap`
//!   rendered in key order — a TESTED property (randomized insertion
//!   orders must produce identical bytes), not a convention, because the
//!   re-serialization audit false-fails on any ordering drift.
//! - **I3 filename law:** `<UTC YYYYMMDDTHHMMSS>Z-<slug>.md`, civil date
//!   computed std-only (Howard Hinnant's days-from-civil inverse).
//! - **Wire mirror:** `encode_frame` produces exactly what
//!   crates/caddis-warden `wire.rs::parse` consumes (`name len\n bytes\n`
//!   in fixed field order) — the two ends must never drift apart.
//! - **Fail-closed verdict parse:** missing/ill-typed fields are errors,
//!   never a lenient default.
//!
//! Lock (I2+), sandbox (I5+), head-linearity (I3+) and the runner-wired
//! remember() flow land in the next slice-(a) increment; this module is
//! the tested substrate they compose.

use std::collections::BTreeMap;

use crate::json;
use crate::sha256;

/// The only frontmatter keys allowed to derive AFTER the warden verdict.
pub const STAMP_WARDEN_SEQ: &str = "warden_seq";
pub const STAMP_WARDEN_TX: &str = "warden_tx_hash";

// ---------------------------------------------------------------------------
// MemoryDoc — deterministic frontmatter + body
// ---------------------------------------------------------------------------

/// One memory file's full content model. Frontmatter values are single-line
/// by construction: `render` refuses a value containing a newline (the
/// `key: value` surface has no escaping, and inventing one would fork the
/// format qmd already parses).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryDoc {
    pub front: BTreeMap<String, String>,
    pub body: String,
}

impl MemoryDoc {
    /// Deterministic bytes: `---` fence, one `key: value` per line in BTreeMap
    /// (sorted) key order, blank line, then the body. Insertion order of the
    /// map cannot influence the output — that is the I1+ property.
    pub fn render(&self) -> String {
        let mut s = String::from("---\n");
        for (k, v) in &self.front {
            assert!(
                !v.contains('\n') && !k.contains('\n'),
                "frontmatter key/value must be single-line: {k:?}"
            );
            s.push_str(k);
            s.push_str(": ");
            s.push_str(v);
            s.push('\n');
        }
        s.push_str("---\n\n");
        s.push_str(&self.body);
        s
    }

    /// The byte-exact payload the warden must allow (I1). A draft that
    /// already carries warden stamps is a caller bug — loud, not quiet.
    pub fn draft_bytes(&self) -> Vec<u8> {
        assert!(
            !self.front.contains_key(STAMP_WARDEN_SEQ) && !self.front.contains_key(STAMP_WARDEN_TX),
            "draft must not carry warden stamps"
        );
        self.render().into_bytes()
    }

    /// Hash of the draft bytes — the anchor the warden ledger row and the
    /// doc audit share.
    pub fn draft_sha256(&self) -> String {
        sha256::hex(&self.draft_bytes())
    }

    /// Post-verdict stamps (I4): the ONLY mutations allowed after the gate.
    pub fn apply_stamps(&mut self, seq: u64, tx_hash: &str) {
        self.front
            .insert(STAMP_WARDEN_SEQ.to_string(), seq.to_string());
        self.front
            .insert(STAMP_WARDEN_TX.to_string(), tx_hash.to_string());
    }

    /// Audit leg (I1): drop the two stamps, re-render. Result must equal the
    /// `draft_bytes()` the warden allowed — byte-exact, or the doc was
    /// mutated after the verdict.
    pub fn strip_stamps_render(&self) -> String {
        let mut front = self.front.clone();
        front.remove(STAMP_WARDEN_SEQ);
        front.remove(STAMP_WARDEN_TX);
        MemoryDoc {
            front,
            body: self.body.clone(),
        }
        .render()
    }
}

// ---------------------------------------------------------------------------
// Warden wire — encoder mirroring crates/caddis-warden/src/wire.rs
// ---------------------------------------------------------------------------

/// Frame field order is FIXED (`tool`, `command`, `path`, `content`) — the
/// warden's parser reads exactly this sequence; ordering is protocol, not
/// taste. `content` takes bytes because the payload is the draft doc, and
/// the length prefix is a BYTE count: content size must never round-trip
/// through a character-count assumption.
pub fn encode_frame(tool: &str, command: &str, path: &str, content: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, val) in [
        ("tool", tool.as_bytes()),
        ("command", command.as_bytes()),
        ("path", path.as_bytes()),
        ("content", content),
    ] {
        out.extend_from_slice(format!("{} {}\n", name, val.len()).as_bytes());
        out.extend_from_slice(val);
        out.push(b'\n');
    }
    out
}

/// A parsed warden reply. `allow` is `verdict == "allow"` exactly — any
/// other verdict string is a deny with the verdict preserved for telemetry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WardenVerdict {
    pub verdict: String,
    pub allow: bool,
    pub reason: String,
    pub law: String,
    pub seq: u64,
}

/// Fail-closed parse of the warden's JSON reply (`{verdict, reason, law,
/// seq}`). Adapter doctrine: RAN but unreadable → BLOCK — so every
/// malformation here is an `Err`, never a default.
pub fn parse_verdict(reply: &str) -> Result<WardenVerdict, String> {
    let v = json::parse(reply).map_err(|e| format!("unparseable warden reply: {e:?}"))?;
    let verdict = field_str(&v, "verdict")?;
    let reason = field_str(&v, "reason")?;
    let law = field_str(&v, "law")?;
    let seq_f = v
        .get("seq")
        .and_then(|x| x.as_f64())
        .ok_or_else(|| "seq missing or not a number".to_string())?;
    if !(seq_f.is_finite() && seq_f >= 0.0 && seq_f.fract() == 0.0) {
        return Err(format!("seq not a whole non-negative number: {seq_f}"));
    }
    let seq = seq_f as u64;
    let allow = verdict == "allow";
    Ok(WardenVerdict {
        verdict: verdict.to_string(),
        allow,
        reason: reason.to_string(),
        law: law.to_string(),
        seq,
    })
}

fn field_str<'a>(v: &'a json::Value, key: &str) -> Result<&'a str, String> {
    v.get(key)
        .and_then(|x| x.as_str())
        .ok_or_else(|| format!("{key} missing or not a string"))
}

// ---------------------------------------------------------------------------
// I3 filename law
// ---------------------------------------------------------------------------

/// Std-only civil date from a Unix day count (Howard Hinnant,
/// chrono-free). Valid for the whole epoch the organ can ever see.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// `YYYYMMDDTHHMMSS` in UTC — the timestamp half of the I3 filename.
pub fn utc_compact(unix: u64) -> String {
    let days = (unix / 86400) as i64;
    let secs = unix % 86400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{:04}{:02}{:02}T{:02}{:02}{:02}",
        y,
        m,
        d,
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

/// `[a-z0-9]` runs → `-`, collapsed, trimmed, capped. Empty result (no
/// usable characters) is an error: a memory with no slug has no filename.
pub fn slugify(title: &str) -> Result<String, String> {
    let mut slug = String::new();
    let mut dash = false;
    for ch in title.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            dash = false;
        } else if !slug.is_empty() && !dash {
            slug.push('-');
            dash = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    slug.truncate(64);
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        return Err(format!("title yields no slug: {title:?}"));
    }
    Ok(slug)
}

/// Full I3 filename: `<UTC YYYYMMDDTHHMMSS>Z-<slug>.md`.
pub fn filename(unix: u64, slug: &str) -> String {
    format!("{}Z-{}.md", utc_compact(unix), slug)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// xorshift64* — deterministic "randomness", std-only.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            self.0 = x;
            x.wrapping_mul(0x2545F4914F6CDD1D)
        }
    }

    fn sample_doc(seed: u64) -> MemoryDoc {
        let mut rng = Rng(seed | 1);
        let mut front = BTreeMap::new();
        for i in 0..24 {
            let k = format!("key_{:02}", rng.next() % 24);
            let v = format!("val-{}-{}", i, rng.next() % 1000);
            front.insert(k, v);
        }
        MemoryDoc {
            front,
            body: "body line\nsecond line\n".to_string(),
        }
    }

    /// I1+ gate property (quorum (a)->(b)): >=1000 randomized key
    /// insertions must yield identical bytes. 40 seeds x 30 shuffled
    /// insertion orders = 1200 randomized builds of the same map.
    #[test]
    fn serialization_stability_randomized_insertions() {
        for seed in 1..=40u64 {
            let doc = sample_doc(seed);
            let canonical = doc.render();
            let pairs: Vec<(String, String)> = doc.front.clone().into_iter().collect();
            for perm in 0..30u64 {
                let mut rng = Rng(seed * 1000 + perm);
                let mut shuffled = pairs.clone();
                // Fisher-Yates driven by the rng
                for i in (1..shuffled.len()).rev() {
                    let j = (rng.next() as usize) % (i + 1);
                    shuffled.swap(i, j);
                }
                let mut front = BTreeMap::new();
                for (k, v) in shuffled {
                    front.insert(k, v);
                }
                let rebuilt = MemoryDoc {
                    front,
                    body: doc.body.clone(),
                };
                assert_eq!(
                    rebuilt.render(),
                    canonical,
                    "seed {seed} perm {perm} drifted"
                );
            }
        }
    }

    #[test]
    fn stamps_round_trip_is_byte_exact() {
        let mut doc = sample_doc(7);
        let draft = doc.draft_bytes();
        let draft_hash = doc.draft_sha256();
        doc.apply_stamps(42, "deadbeef");
        assert_eq!(doc.strip_stamps_render().into_bytes(), draft);
        // BTreeMap ordering: the two late stamps land in sorted position,
        // never re-flowing the rest of the frontmatter.
        let stamped = doc.render();
        assert!(stamped.contains("\nwarden_seq: 42\n"));
        assert!(stamped.contains("\nwarden_tx_hash: deadbeef\n"));
        // And the audit anchor is stable across the round trip.
        let mut again = sample_doc(7);
        again.apply_stamps(43, "other");
        let _ = again.strip_stamps_render();
        again.front.remove(STAMP_WARDEN_TX);
        again.front.remove(STAMP_WARDEN_SEQ);
        assert_eq!(again.draft_sha256(), draft_hash);
    }

    #[test]
    #[should_panic(expected = "draft must not carry warden stamps")]
    fn draft_with_stamps_is_loud() {
        let mut doc = sample_doc(3);
        doc.apply_stamps(1, "h");
        let _ = doc.draft_bytes();
    }

    #[test]
    #[should_panic(expected = "single-line")]
    fn multiline_value_is_refused() {
        let mut front = BTreeMap::new();
        front.insert("bad".to_string(), "two\nlines".to_string());
        let doc = MemoryDoc {
            front,
            body: String::new(),
        };
        let _ = doc.render();
    }

    #[test]
    fn frame_encoding_is_exact_bytes() {
        let frame = encode_frame("memory.write", "put", "a.md", b"hi\nthere");
        let expected =
            b"tool 12\nmemory.write\ncommand 3\nput\npath 4\na.md\ncontent 8\nhi\nthere\n";
        assert_eq!(&frame, &expected[..]);
        // Byte-count law: multi-byte UTF-8 counts bytes, not chars.
        let utf8 = encode_frame("t", "c", "p", "äöü".as_bytes());
        assert!(utf8.windows(10).any(|w| w == b"content 6\n"));
    }

    #[test]
    fn verdict_allow_and_deny_parse() {
        let ok =
            parse_verdict(r#"{"verdict":"allow","reason":"clean","law":"L1","seq":12}"#).unwrap();
        assert!(ok.allow);
        assert_eq!(ok.seq, 12);
        let deny =
            parse_verdict(r#"{"verdict":"block","reason":"secret","law":"S2","seq":13}"#).unwrap();
        assert!(!deny.allow);
        assert_eq!(deny.verdict, "block");
    }

    #[test]
    fn verdict_malformed_fails_closed() {
        assert!(parse_verdict("not json").is_err());
        assert!(parse_verdict(r#"{"verdict":"allow"}"#).is_err());
        assert!(parse_verdict(r#"{"verdict":1,"reason":"r","law":"l","seq":1}"#).is_err());
        assert!(parse_verdict(r#"{"verdict":"allow","reason":"r","law":"l"}"#).is_err());
        assert!(parse_verdict(r#"{"verdict":"allow","reason":"r","law":"l","seq":-3}"#).is_err());
        assert!(parse_verdict(r#"{"verdict":"allow","reason":"r","law":"l","seq":1.5}"#).is_err());
    }

    #[test]
    fn utc_compact_known_moments() {
        assert_eq!(utc_compact(0), "19700101T000000");
        // Cross-checked against this build session's own file mtimes.
        assert_eq!(utc_compact(1787757392), "20260826T151632");
        assert_eq!(utc_compact(951_782_400), "20000229T000000"); // leap day
    }

    #[test]
    fn slug_and_filename_law() {
        assert_eq!(
            slugify("Council Verdict: I2+ — steal rule!").unwrap(),
            "council-verdict-i2-steal-rule"
        );
        assert_eq!(slugify("  --multiple   gaps--  ").unwrap(), "multiple-gaps");
        assert!(slugify("!!!").is_err());
        let long = "x".repeat(80);
        assert_eq!(slugify(&long).unwrap().len(), 64);
        assert_eq!(filename(0, "genesis"), "19700101T000000Z-genesis.md");
        assert_eq!(filename(1787757392, "tick"), "20260826T151632Z-tick.md");
    }
}
