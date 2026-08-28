//! seed.rs — P4 slice 3 (F13): the SIGNED SEED artifact + verify-gate.
//!
//! F13 (quorum verdict, groq biggest-risk): the WORLD SEED is a SIGNED
//! artifact — build verifies the signature before constructing anything
//! (supply-chain integrity). The seed rebuilds the organ home on any
//! machine (sovereignty as product; feeds r2-public-clean). Reframe-2:
//! the world hosts VIEWS/CONTROLS + the seed only — organ code never
//! leaves the caddis repo; the seed carries the STREAM, not the code.
//!
//! What the seed IS:
//! - One flat JSON object, fixed member order, byte-deterministic for a
//!   given stream + key: `kind`, `v`, `rows`, `stream_sha256`,
//!   `stream_b64` (the EXACT `seats.jsonl` bytes, base64), then the
//!   attestation tail `fingerprint` + `sig`.
//! - `sig = HMAC-SHA256(seed.key, canonical_bytes_without_sig)` — the
//!   canonical form is what the writer writes and the ONLY shape the
//!   verifier re-encodes (registry audit==obey law).
//! - `stream_sha256` = [`registry::stream_digest`] — plain sha256 over
//!   the stream bytes, the SAME value the view and every external
//!   verifier compute (F2 law; see the double-hash defect note there).
//!
//! Key law (`seed.key` BESIDE the stream, in the organ home): line 1 =
//! 64 lowercase hex (the key), line 2 = `born_rows=<u64>` (stream size
//! at mint — the honest history marker; no clocks, MV11 family). Minted
//! by the organ on first export, never hand-typed, never overwritten,
//! never printed — only its fingerprint (first 16 hex of SHA-256(key))
//! is shown. A malformed file is fail-closed: verify reports KEY_BROKEN
//! and restore REFUSES. Vendored by organ law from
//! caddis-router/src/warden.rs (R5) — the estate's ONE HMAC/key law;
//! ⚠ any fix here lands in every vendored copy until the primitive
//! graduates to caddis-core.
//!
//! Honest boundary (same law as the router warden):
//! - The signature attests the artifact's bytes were produced by the
//!   organ under the key. It is NOT a public-key signature: a verifying
//!   machine needs the KEY (a personal product — the owner carries
//!   `seed.key` beside the seed, `--key <file>`). Whoever holds the key
//!   can mint new seeds; whoever can rewrite the home can re-export.
//!   The gate makes tampering VISIBLE and construction REFUSE, never
//!   impossible — supply-chain integrity for the sovereign-artifact
//!   path, not DRM.
//! - Verify findings are cumulative and name themselves (BAD_SHAPE,
//!   STREAM_DIGEST_MISMATCH, ROWS_MISMATCH, FINGERPRINT_MISMATCH,
//!   SIG_MISMATCH, KEY_ABSENT, KEY_BROKEN): an operator reading the
//!   refusal must see WHICH law broke, never just "no".

use std::collections::hash_map::RandomState;
use std::fmt;
use std::fs;
use std::hash::{BuildHasher, Hasher};
use std::path::Path;

use crate::json::{self, Value};
use crate::registry;
use crate::sha256::{sha256, Sha256};

pub const SEED_KIND: &str = "caddis-deliberate-seed";
pub const SEED_V: u64 = 1;
const KEY_FILE: &str = "seed.key";

// ---------------------------------------------------------------------------
// HMAC-SHA256 (RFC 2104) — vendored from caddis-router/src/warden.rs
// ---------------------------------------------------------------------------

/// True bytes→hex encoder. NOT [`crate::sha256::hex`] — that helper is
/// the COMPLETE one-shot digest (the double-hash defect family fixed in
/// `stream_digest`, P4 slice 1); here we encode RAW bytes.
fn hex64(d: &[u8]) -> String {
    let mut s = String::with_capacity(d.len() * 2);
    for b in d {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// HMAC-SHA256 (RFC 2104). Keys longer than the 64-byte block are hashed
/// first; shorter keys are zero-padded — both per the RFC.
pub fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    let k: [u8; 64] = {
        let mut k = [0u8; 64];
        if key.len() > 64 {
            k[..32].copy_from_slice(&sha256(key));
        } else {
            k[..key.len()].copy_from_slice(key);
        }
        k
    };
    let mut ipad = [0x36u8; 64];
    let mut opad = [0x5cu8; 64];
    for i in 0..64 {
        ipad[i] ^= k[i];
        opad[i] ^= k[i];
    }
    let mut inner = Sha256::new();
    inner.update(&ipad);
    inner.update(msg);
    let ih = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(&opad);
    outer.update(&ih);
    outer.finalize()
}

/// Constant-time equality over equal-length digests (no early exit — the
/// comparison time must not leak how many bytes matched).
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for i in 0..a.len() {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

/// 64 lowercase hex chars -> 32 bytes.
fn unhex64(s: &str) -> Option<[u8; 32]> {
    let b = s.as_bytes();
    if b.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for i in 0..32 {
        let hi = (b[i * 2] as char).to_digit(16)?;
        let lo = (b[i * 2 + 1] as char).to_digit(16)?;
        out[i] = (hi * 16 + lo) as u8;
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// Base64 (std alphabet, padded) — the stream rides as text inside the artifact
// ---------------------------------------------------------------------------

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn b64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            B64[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            B64[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

fn b64_decode(s: &str) -> Option<Vec<u8>> {
    let bytes = s.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    let mut seen_pad = false;
    for (ci, quad) in bytes.chunks(4).enumerate() {
        let mut n: u32 = 0;
        for (i, &c) in quad.iter().enumerate() {
            let v = if c == b'=' {
                // Padding: only the last two positions of the LAST quad.
                if ci + 1 != bytes.len() / 4 || i < 2 {
                    return None;
                }
                seen_pad = true;
                0
            } else {
                if seen_pad {
                    return None; // data after padding
                }
                B64.iter().position(|&a| a == c)? as u32
            };
            n = (n << 6) | v;
        }
        let pad = quad.iter().filter(|&&c| c == b'=').count();
        let take = 3 - pad;
        if take == 0 {
            continue;
        }
        out.push((n >> 16) as u8);
        if take > 1 {
            out.push((n >> 8) as u8);
        }
        if take > 2 {
            out.push(n as u8);
        }
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// Key slot (vendored shape from the router warden; born_rows, not activated_seq)
// ---------------------------------------------------------------------------

/// A parsed, ready-to-sign seed key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeedKey {
    key: [u8; 32],
    born_rows: u64,
}

impl SeedKey {
    /// 16-hex display identity — the only form of the key that is ever
    /// shown. The key material itself never prints.
    pub fn fingerprint(&self) -> String {
        hex64(&sha256(&self.key))[..16].to_string()
    }

    /// Stream size at mint (the honest history marker; no clocks).
    pub fn born_rows(&self) -> u64 {
        self.born_rows
    }

    /// Sign the canonical encoding of the artifact (the exact bytes the
    /// writer writes, minus the sig member itself).
    pub fn sign(&self, canonical: &str) -> String {
        hex64(&hmac_sha256(&self.key, canonical.as_bytes()))
    }

    /// Check a signature over the canonical encoding. Malformed hex is a
    /// failed check, never a panic.
    pub fn check(&self, canonical: &str, sig: &str) -> bool {
        let expected = hmac_sha256(&self.key, canonical.as_bytes());
        match unhex64(sig) {
            Some(got) => ct_eq(&expected, &got),
            None => false,
        }
    }
}

/// The home's view of the seed key: absent, loaded, or present-but-broken.
#[derive(Debug, Clone, PartialEq)]
pub enum SeedKeySlot {
    /// No `seed.key` beside the stream: first export mints it.
    Absent,
    /// Key loaded.
    Key(SeedKey),
    /// The key file exists but will not parse: verify reports it, export
    /// and restore REFUSE (fail closed — a corrupted key must not
    /// silently strip attestation).
    Broken(String),
}

impl SeedKeySlot {
    /// Load the slot from a DIRECTORY (the organ home) or an explicit
    /// key FILE path (`--key`, the carry-the-key sovereignty path).
    pub fn load(from: &Path) -> SeedKeySlot {
        let text = match fs::read_to_string(from) {
            Ok(t) => t,
            Err(_) => return SeedKeySlot::Absent,
        };
        parse_key_file(&text)
    }
}

/// Strict two-line parse: exactly the shape [`mint_seed_key`] writes.
/// Anything else is Broken — a key file is machinery, not a document to
/// interpret leniently.
fn parse_key_file(text: &str) -> SeedKeySlot {
    let mut lines = text.lines();
    let key_line = lines.next().unwrap_or_default().trim();
    let Some(key) = unhex64(key_line) else {
        return SeedKeySlot::Broken("line 1 is not 64 lowercase/uppercase hex chars".into());
    };
    let Some(rows_line) = lines.next() else {
        return SeedKeySlot::Broken("missing 'born_rows=<n>' line 2".into());
    };
    let rows_line = rows_line.trim();
    let Some(rest) = rows_line.strip_prefix("born_rows=") else {
        return SeedKeySlot::Broken(format!("line 2 is not 'born_rows=<n>': {rows_line:?}"));
    };
    let Ok(born_rows) = rest.trim().parse::<u64>() else {
        return SeedKeySlot::Broken(format!("born_rows is not a u64: {rest:?}"));
    };
    if lines.next().is_some() {
        return SeedKeySlot::Broken("extra lines after born_rows".into());
    }
    SeedKeySlot::Key(SeedKey { key, born_rows })
}

/// Mint a fresh seed key into `dir`. REFUSES to overwrite an existing
/// file — a key file is born once; rotation is a deliberate operator
/// card, never an accident.
pub fn mint_seed_key(dir: &Path, born_rows: u64) -> Result<SeedKey, String> {
    let key_path = dir.join(KEY_FILE);
    if key_path.exists() {
        return Err(format!(
            "refusing to overwrite {} — mint once; rotation is an operator card",
            key_path.display()
        ));
    }
    let key = fresh_key();
    let text = format!("{}\nborn_rows={}\n", hex64(&key), born_rows);
    fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    // Create-new: another process minting concurrently must fail, not clobber.
    fs::write(&key_path, text).map_err(|e| format!("cannot write {}: {e}", key_path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&key_path, fs::Permissions::from_mode(0o600));
    }
    Ok(SeedKey { key, born_rows })
}

/// 32 bytes from std-only entropy — vendored from the router warden.
/// Honest scope: strong for a LOCAL attestation key an attacker would
/// need to READ the box to abuse; not a CSPRNG API (zero-dep law).
fn fresh_key() -> [u8; 32] {
    let mut seed = String::new();
    for salt in 0u8..8 {
        let rs = RandomState::new();
        let mut h = rs.build_hasher();
        h.write(&[salt]);
        h.write(&std::process::id().to_be_bytes());
        seed.push_str(&format!("{:016x}", h.finish()));
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let stack_marker = 0u8;
    let aslr = &stack_marker as *const u8 as usize;
    seed.push_str(&format!(
        "{:016x}{:016x}{:016x}",
        now.as_secs(),
        now.subsec_nanos(),
        aslr
    ));
    sha256(seed.as_bytes())
}

// ---------------------------------------------------------------------------
// The artifact
// ---------------------------------------------------------------------------

/// A strictly-parsed seed artifact (post-shape-check; crypto verdict is
/// [`verify_seed_text`]'s, never the parser's).
#[derive(Debug, Clone, PartialEq)]
pub struct SeedArtifact {
    pub rows: u64,
    pub stream_sha256: String,
    pub stream_text: String,
    pub fingerprint: String,
    pub sig: String,
}

impl SeedArtifact {
    /// Canonical bytes WITHOUT the sig member — the exact message the
    /// HMAC covers. Fixed member order; byte-deterministic.
    fn canonical(&self) -> String {
        format!(
            "{{\"kind\":\"{}\",\"v\":{},\"rows\":{},\"stream_sha256\":\"{}\",\"stream_b64\":\"{}\",\"fingerprint\":\"{}\"",
            SEED_KIND,
            SEED_V,
            self.rows,
            self.stream_sha256,
            b64_encode(self.stream_text.as_bytes()),
            self.fingerprint
        )
    }
}

const ARTIFACT_FIELDS: &[&str] = &[
    "kind",
    "v",
    "rows",
    "stream_sha256",
    "stream_b64",
    "fingerprint",
    "sig",
];

/// Strict parse: one JSON object, EXACT field set, exact types. Unknown
/// or missing fields are refused (flat exact-field law — a typo must
/// never silently drop the member it was trying to forge).
fn parse_artifact(text: &str) -> Result<SeedArtifact, String> {
    let v = json::parse(text).map_err(|e| format!("BAD_SHAPE not JSON: {e:?}"))?;
    let obj = v
        .as_obj()
        .ok_or("BAD_SHAPE artifact is not a JSON object")?;
    let have: Vec<&str> = obj.iter().map(|(k, _)| k.as_str()).collect();
    if have != ARTIFACT_FIELDS {
        return Err(format!(
            "BAD_SHAPE field set is not exactly {ARTIFACT_FIELDS:?}: {have:?}"
        ));
    }
    let get = |k: &str| obj.iter().find(|(key, _)| key == k).map(|(_, v)| v);
    if get("kind").and_then(Value::as_str) != Some(SEED_KIND) {
        return Err(format!("BAD_SHAPE kind is not {SEED_KIND:?}"));
    }
    let ver = get("v")
        .and_then(Value::as_f64)
        .ok_or("BAD_SHAPE v is not a number")?;
    if ver != SEED_V as f64 {
        return Err(format!(
            "VERSION artifact v={ver} but this organ knows v={SEED_V}"
        ));
    }
    let rows = get("rows")
        .and_then(Value::as_f64)
        .filter(|n| n.fract() == 0.0 && *n >= 0.0)
        .ok_or("BAD_SHAPE rows is not a whole number")? as u64;
    let stream_sha256 = get("stream_sha256")
        .and_then(Value::as_str)
        .ok_or("BAD_SHAPE stream_sha256 is not a string")?
        .to_string();
    let stream_b64 = get("stream_b64")
        .and_then(Value::as_str)
        .ok_or("BAD_SHAPE stream_b64 is not a string")?;
    let fingerprint = get("fingerprint")
        .and_then(Value::as_str)
        .ok_or("BAD_SHAPE fingerprint is not a string")?
        .to_string();
    let sig = get("sig")
        .and_then(Value::as_str)
        .ok_or("BAD_SHAPE sig is not a string")?
        .to_string();
    let stream_bytes = b64_decode(stream_b64).ok_or("BAD_SHAPE stream_b64 is not valid base64")?;
    let stream_text =
        String::from_utf8(stream_bytes).map_err(|_| "BAD_SHAPE stream is not UTF-8".to_string())?;
    Ok(SeedArtifact {
        rows,
        stream_sha256,
        stream_text,
        fingerprint,
        sig,
    })
}

/// The verify verdict. `findings` is cumulative — every broken law names
/// itself; `clean` requires zero findings.
#[derive(Debug, Clone, PartialEq)]
pub struct SeedVerify {
    pub clean: bool,
    pub findings: Vec<String>,
    pub fingerprint: Option<String>,
    pub rows: Option<u64>,
    pub stream_sha256: Option<String>,
}

impl SeedVerify {
    fn refused(findings: Vec<String>) -> Self {
        SeedVerify {
            clean: false,
            findings,
            fingerprint: None,
            rows: None,
            stream_sha256: None,
        }
    }
}

impl fmt::Display for SeedVerify {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.clean {
            write!(
                f,
                "clean (fingerprint {}, {} rows, stream {})",
                self.fingerprint.as_deref().unwrap_or("?"),
                self.rows.unwrap_or(0),
                self.stream_sha256.as_deref().unwrap_or("?")
            )
        } else {
            write!(f, "REFUSED: {}", self.findings.join("; "))
        }
    }
}

/// The F13 GATE: verify a seed artifact against a key slot. Cumulative
/// findings; `clean` only when every law holds:
/// - shape/version (strict parse),
/// - `stream_sha256` == sha256 of the embedded stream bytes (payload
///   integrity, key-independent),
/// - `rows` == the stream's actual parsed card count,
/// - `fingerprint` == the key's (the artifact names its signer),
/// - `sig` == HMAC over the canonical bytes (the attestation itself).
pub fn verify_seed_text(text: &str, slot: &SeedKeySlot) -> SeedVerify {
    let key = match slot {
        SeedKeySlot::Key(k) => k,
        SeedKeySlot::Absent => {
            return SeedVerify::refused(vec!["KEY_ABSENT no seed.key — cannot verify".into()])
        }
        SeedKeySlot::Broken(why) => return SeedVerify::refused(vec![format!("KEY_BROKEN {why}")]),
    };
    let art = match parse_artifact(text) {
        Ok(a) => a,
        Err(e) => return SeedVerify::refused(vec![e]),
    };
    let mut findings = Vec::new();
    let actual_digest = registry::stream_digest(&art.stream_text);
    if actual_digest != art.stream_sha256 {
        findings.push(format!(
            "STREAM_DIGEST_MISMATCH artifact says {} but the embedded stream hashes to {actual_digest}",
            art.stream_sha256
        ));
    }
    match registry::parse_stream(&art.stream_text) {
        Ok(cards) => {
            if cards.len() as u64 != art.rows {
                findings.push(format!(
                    "ROWS_MISMATCH artifact says {} rows but the stream holds {}",
                    art.rows,
                    cards.len()
                ));
            }
        }
        Err(e) => findings.push(format!("ROWS_MISMATCH stream does not parse: {e}")),
    }
    if art.fingerprint != key.fingerprint() {
        findings.push(format!(
            "FINGERPRINT_MISMATCH artifact names {} but the key is {}",
            art.fingerprint,
            key.fingerprint()
        ));
    }
    let canonical = art.canonical();
    if !key.check(&canonical, &art.sig) {
        findings.push("SIG_MISMATCH the signature does not cover these bytes".into());
    }
    SeedVerify {
        clean: findings.is_empty(),
        findings,
        fingerprint: Some(key.fingerprint()),
        rows: Some(art.rows),
        stream_sha256: Some(art.stream_sha256.clone()),
    }
}

// ---------------------------------------------------------------------------
// Export + restore (the constructor, gated)
// ---------------------------------------------------------------------------

/// What an [`export_seed`] call did.
#[derive(Debug, Clone, PartialEq)]
pub struct SeedExport {
    pub artifact: String,
    pub fingerprint: String,
    pub rows: u64,
    pub stream_sha256: String,
    pub key_minted: bool,
}

/// Export the home's stream as a SIGNED seed artifact.
///
/// Law: the stream must exist and parse (fail-closed — a malformed home
/// never ships); the key is minted ONCE at first export (born_rows = the
/// stream size at that moment) and reused ever after; a broken key
/// refuses (fail-closed). Deterministic: same stream + same key ⇒ same
/// artifact bytes.
pub fn export_seed(home_dir: &Path) -> Result<SeedExport, String> {
    let stream_path = home_dir.join("seats.jsonl");
    let stream_text = fs::read_to_string(&stream_path)
        .map_err(|e| format!("cannot read {}: {e}", stream_path.display()))?;
    let cards = registry::parse_stream(&stream_text)
        .map_err(|e| format!("home stream refuses export: {e}"))?;
    let rows = cards.len() as u64;
    let (key, key_minted) = match SeedKeySlot::load(&home_dir.join(KEY_FILE)) {
        SeedKeySlot::Absent => (mint_seed_key(home_dir, rows)?, true),
        SeedKeySlot::Key(k) => (k, false),
        SeedKeySlot::Broken(why) => {
            return Err(format!(
                "seed.key broken — export refuses (fail closed): {why}"
            ))
        }
    };
    let stream_sha256 = registry::stream_digest(&stream_text);
    let art = SeedArtifact {
        rows,
        stream_sha256: stream_sha256.clone(),
        stream_text,
        fingerprint: key.fingerprint(),
        sig: String::new(),
    };
    let canonical = art.canonical();
    let sig = key.sign(&canonical);
    let artifact = format!("{},\"sig\":\"{sig}\"}}\n", canonical);
    Ok(SeedExport {
        artifact,
        fingerprint: key.fingerprint(),
        rows,
        stream_sha256,
        key_minted,
    })
}

/// What a [`restore_seed`] call did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreOutcome {
    /// Fresh construction: stream written, view re-derived + proven.
    Constructed { rows: u64 },
    /// The target already held BYTE-IDENTICAL bytes (idempotent; the view
    /// was still proven against the stream truth).
    AlreadyIdentical { rows: u64 },
}

/// THE F13 CONSTRUCTOR: rebuild the organ home from a seed artifact.
///
/// Verify FIRST — ANY finding refuses with NOTHING written. Then:
/// an existing target stream is never clobbered (identical bytes are an
/// idempotent no-op; diverged bytes refuse — the existing home is truth
/// until an operator rules); a fresh target is written atomically and
/// the view is re-derived through the REAL loader (never trusted from
/// the artifact) and proven against `stream_sha256`.
pub fn restore_seed(
    text: &str,
    slot: &SeedKeySlot,
    to_dir: &Path,
) -> Result<RestoreOutcome, String> {
    let verdict = verify_seed_text(text, slot);
    if !verdict.clean {
        return Err(format!("seed REFUSED — nothing constructed: {verdict}"));
    }
    let art = parse_artifact(text).expect("verify clean implies parse ok");
    let stream_path = to_dir.join("seats.jsonl");
    let view_path = to_dir.join("seats-view.json");
    let cards = registry::parse_stream(&art.stream_text)
        .map_err(|e| format!("artifact stream does not parse: {e}"))?;
    let rows = cards.len() as u64;
    match fs::read_to_string(&stream_path) {
        Ok(existing) => {
            if existing != art.stream_text {
                return Err(format!(
                    "refusing to clobber {} — the target stream differs from the seed \
                     (identical bytes are idempotent; a diverged home is truth until an \
                     operator rules)",
                    stream_path.display()
                ));
            }
            registry::load_and_sync(&stream_path, &view_path)
                .map_err(|e| format!("view re-proof failed: {e}"))?;
            Ok(RestoreOutcome::AlreadyIdentical { rows })
        }
        Err(_) => {
            fs::create_dir_all(to_dir)
                .map_err(|e| format!("cannot create {}: {e}", to_dir.display()))?;
            let tmp = to_dir.join("seats.jsonl.tmp");
            fs::write(&tmp, art.stream_text.as_bytes())
                .map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
            fs::rename(&tmp, &stream_path).map_err(|e| {
                format!(
                    "cannot move {} -> {}: {e}",
                    tmp.display(),
                    stream_path.display()
                )
            })?;
            // Prove what landed on disk: re-read the bytes, then run the
            // REAL loader (view re-derivation through registry law).
            let written = fs::read_to_string(&stream_path)
                .map_err(|e| format!("cannot re-read {}: {e}", stream_path.display()))?;
            if written != art.stream_text {
                return Err("constructed stream does not match the artifact (I/O honesty)".into());
            }
            if registry::stream_digest(&written) != art.stream_sha256 {
                return Err("constructed stream digest does not match the signed digest".into());
            }
            registry::load_and_sync(&stream_path, &view_path)
                .map_err(|e| format!("view derivation failed: {e}"))?;
            Ok(RestoreOutcome::Constructed { rows })
        }
    }
}

#[cfg(test)]
#[path = "seed_tests.rs"]
mod tests;
