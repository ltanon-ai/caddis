//! warden.rs — R5 (P4 slice 5): WARDEN-SIGNED ROW IDENTITY.
//!
//! The gap this closes (noted at the outcome row since P2): `model` — and
//! every other identity field on a row — was transport-record truth as PLAIN
//! DATA. Any process able to write the ledger file could append a row
//! claiming any lane/model, and the capability fold would believe it. R5
//! makes the organ's appends ATTESTED: every row the organ appends while a
//! warden key exists carries `sig = HMAC-SHA256(key, canonical-row)`, and
//! [`crate::verify`] separates organ-written rows (signature checks) from
//! everything else (mismatch = hand-edited or injected, unsigned after
//! activation = injected, loud finding).
//!
//! What the signature IS and IS NOT (honest boundary):
//! - It attests the ROW'S VALUES were appended by the organ under the key —
//!   nothing more. It does not prove the transport record itself was honest;
//!   a lying provider is an F2 measurement problem, not a signing one.
//! - It is NOT tamper-proof against the file's owner: whoever can write the
//!   ledger can also DELETE warden.key (appends then go unsigned — visible,
//!   activation era ends) or edit rows (signatures break — findings). The
//!   signature makes tampering VISIBLE, not impossible. The warden-crate
//!   identity.rs precedent said a digest "buys nothing without signing the
//!   ledger itself" — this signs the ledger's rows.
//!
//! Activation law: minting writes `activated_seq = <current max seq>`. Rows
//! at or below it are the honest unsigned history (reported, never a
//! finding); rows ABOVE it without a signature are injected — a finding.
//!
//! File law (`warden.key`, BESIDE the ledger — the same home as policy.json
//! and lanes.jsonl): line 1 = 64 lowercase hex (the key), line 2 =
//! `activated_seq=<u64>`. Minted by the organ (`caddis-router warden mint`),
//! never hand-typed, never overwritten, never printed — only its
//! fingerprint (first 16 hex of SHA-256(key)) is shown. A malformed file is
//! [`WardenSlot::Broken`]: appends REFUSE (fail closed — a corrupted key
//! must not silently strip attestation) and verify flags every signed row
//! unverifiable.
//!
//! Alerts (alerts.jsonl) are deliberately NOT signed in this slice: they are
//! a separate stream with its own append law; signing it is its own card if
//! the operator ever wants it.

use std::collections::hash_map::RandomState;
use std::fs;
use std::hash::{BuildHasher, Hasher};
use std::path::Path;

use crate::sha256::sha256;

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
    let mut inner = crate::sha256::Sha256::new();
    inner.update(&ipad);
    inner.update(msg);
    let ih = inner.finalize();
    let mut outer = crate::sha256::Sha256::new();
    outer.update(&opad);
    outer.update(&ih);
    outer.finalize()
}

fn hex64(d: &[u8]) -> String {
    let mut s = String::with_capacity(d.len() * 2);
    for b in d {
        s.push_str(&format!("{b:02x}"));
    }
    s
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

/// A parsed, ready-to-sign warden key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WardenKey {
    key: [u8; 32],
    activated_seq: u64,
}

impl WardenKey {
    /// 16-hex display identity — the only form of the key that is ever
    /// shown. The key material itself never prints.
    pub fn fingerprint(&self) -> String {
        hex64(&sha256(&self.key))[..16].to_string()
    }

    /// Rows at or below this seq are the pre-activation unsigned era.
    pub fn activated_seq(&self) -> u64 {
        self.activated_seq
    }

    /// Sign the canonical encoding of a row (the exact bytes the ledger
    /// writes, minus the sig member itself).
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

/// The ledger's view of the warden: absent, loaded, or present-but-broken.
#[derive(Debug, Clone, PartialEq)]
pub enum WardenSlot {
    /// No `warden.key` beside the ledger: appends are unsigned (the honest
    /// pre-activation shape — activation is the operator's call).
    Absent,
    /// Key loaded: every append signs.
    Key(WardenKey),
    /// The key file exists but will not parse: appends REFUSE (fail closed),
    /// verify flags signed rows unverifiable. Never silently unsigned.
    Broken(String),
}

impl WardenSlot {
    /// Load the slot for a LEDGER path (the key lives beside the ledger).
    pub fn load(ledger_path: &Path) -> WardenSlot {
        let Some(dir) = ledger_path.parent() else {
            return WardenSlot::Absent;
        };
        if dir.as_os_str().is_empty() {
            return WardenSlot::Absent;
        }
        let key_path = dir.join("warden.key");
        let text = match fs::read_to_string(&key_path) {
            Ok(t) => t,
            Err(_) => return WardenSlot::Absent, // missing = not activated
        };
        parse_key_file(&text)
    }

    /// Sign a canonical row, or fail closed when the key is broken.
    pub fn sign(&self, canonical: &str) -> Result<Option<String>, String> {
        match self {
            WardenSlot::Absent => Ok(None),
            WardenSlot::Key(k) => Ok(Some(k.sign(canonical))),
            WardenSlot::Broken(why) => Err(why.clone()),
        }
    }
}

/// Strict two-line parse: exactly the shape `mint` writes. Anything else is
/// Broken — a key file is machinery, not a document to interpret leniently.
fn parse_key_file(text: &str) -> WardenSlot {
    let mut lines = text.lines();
    let key_line = lines.next().unwrap_or_default().trim();
    let Some(key) = unhex64(key_line) else {
        return WardenSlot::Broken("line 1 is not 64 lowercase/uppercase hex chars".into());
    };
    let Some(seq_line) = lines.next() else {
        return WardenSlot::Broken("missing 'activated_seq=<n>' line 2".into());
    };
    let seq_line = seq_line.trim();
    let Some(rest) = seq_line.strip_prefix("activated_seq=") else {
        return WardenSlot::Broken(format!("line 2 is not 'activated_seq=<n>': {seq_line:?}"));
    };
    let Ok(activated_seq) = rest.trim().parse::<u64>() else {
        return WardenSlot::Broken(format!("activated_seq is not a u64: {rest:?}"));
    };
    if lines.next().is_some() {
        return WardenSlot::Broken("extra lines after activated_seq".into());
    }
    WardenSlot::Key(WardenKey { key, activated_seq })
}

/// Mint a fresh key into `dir` (beside the ledger), activated at
/// `activated_seq` rows. REFUSES to overwrite an existing file — a key file
/// is born once; rotation is a deliberate operator card, never an accident.
pub fn mint(dir: &Path, activated_seq: u64) -> Result<WardenKey, String> {
    let key_path = dir.join("warden.key");
    if key_path.exists() {
        return Err(format!(
            "refusing to overwrite {} — mint once; rotation is an operator card",
            key_path.display()
        ));
    }
    let key = fresh_key();
    let text = format!("{}\nactivated_seq={}\n", hex64(&key), activated_seq);
    fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    // Create-new: another process minting concurrently must fail, not clobber.
    fs::write(&key_path, text).map_err(|e| format!("cannot write {}: {e}", key_path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&key_path, fs::Permissions::from_mode(0o600));
    }
    Ok(WardenKey { key, activated_seq })
}

/// 32 bytes from std-only entropy: OS-seeded `RandomState` hashers (each
/// instance carries fresh per-process OS entropy), the high-resolution
/// clock, the pid, and an ASLR'd stack address, folded through SHA-256.
/// Honest scope: this is strong for a LOCAL attestation key an attacker
/// would need to READ the box to abuse; it is not a CSPRNG API — std has
/// none, and pulling one in would break the zero-dep law.
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

#[cfg(test)]
#[path = "warden_tests.rs"]
mod tests;
