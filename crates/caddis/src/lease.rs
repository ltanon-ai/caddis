//! lease.rs — CARD-0301. The succession state organ: everything past
//! the drain verdict — handover receipts (CLASSIFY a promote, never
//! gate; the shared key is tamper-evidence, not identity — S2/G4/F2),
//! fenced claims, linger hygiene, atomic small-file writes (G9/S3).

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::hmac;
use crate::lineage;
use crate::receipt;

/// Atomic small-file write: temp file in the same dir, then rename.
/// `fs::write` in place is not atomic on Windows (quorum G9/S3).
pub fn write_atomic(dir: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    let tmp = dir.join(format!("{name}.tmp"));
    let dest = dir.join(name);
    let mut f = fs::File::create(&tmp).map_err(|e| format!("create {}: {e}", tmp.display()))?;
    f.write_all(bytes)
        .map_err(|e| format!("write {}: {e}", tmp.display()))?;
    drop(f);
    fs::rename(&tmp, &dest).map_err(|e| format!("rename -> {}: {e}", dest.display()))?;
    Ok(dest)
}

/// The line's current claim generation (0 = never claimed).
pub fn generation(dir: &Path) -> u64 {
    fs::read_to_string(dir.join("claimed.gen"))
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

/// `rotate handover` — the predecessor's last voluntary act.
pub fn handover_cmd(args: &[String]) -> Result<(), String> {
    let (id, rest) = lineage::take(args)?;
    if let Some(a) = rest.first() {
        return Err(format!("unknown argument {a}"));
    }
    let dir = lineage::dir(&id)?;
    let (kind, model, pane, key) = read_arm(&dir, &id)?;
    let receipt = compose(&key, &kind, &model, &pane, &id, "run");
    write_atomic(&dir, "handover.receipt", receipt.as_bytes())?;
    println!("LINEAGE {id}");
    println!("handover: recorded (classifies the promote, never gates it)");
    Ok(())
}

/// Classification: true = clean (valid handover receipt present).
pub fn classify(dir: &Path) -> bool {
    let Ok(bytes) = fs::read(dir.join("handover.receipt")) else {
        return false;
    };
    let Ok(key) = receipt::load_key(dir) else {
        return false;
    };
    let Some((body, mac)) = receipt::split_receipt(&bytes) else {
        return false;
    };
    hmac::hmac_sha256(&key, body) == mac
}

/// CARD-0303: the work root — the ready session's cwd, canonicalized
/// (junction resolution), \\?/UNC prefixes stripped (herdr `--cwd`
/// takes plain Win32 paths; a UNC leak is E3 with extra steps).
pub fn work_root() -> String {
    let cwd = std::env::current_dir().unwrap_or_default();
    let phys = cwd.canonicalize().unwrap_or(cwd);
    let s = phys.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC") {
        return format!(r"\\{rest}");
    }
    s.strip_prefix(r"\\?\").unwrap_or(&s).to_string()
}

/// Stamp the authoritative work root (atomic); `rotate ready` IS at the work.
pub fn stamp_root(dir: &Path) -> Result<String, String> {
    let root = work_root();
    write_atomic(dir, "ready.root", format!("{root}\n").as_bytes())?;
    Ok(root)
}

/// CARD-0304/0308 single-flight: a live armed pane blocks a new ready;
/// a LANDED claim spends the reservation (the promoted pane owns).
pub fn refuse_if_blocked(dir: &Path) -> Result<(), String> {
    if claim_landed(dir) {
        return Ok(()); // CARD-0308: the previous rotation concluded
    }
    let Ok(bytes) = fs::read(dir.join("arm.receipt")) else {
        return Ok(()); // swallow: fail-safe-by-law — no arm, no reservation to protect
    };
    let Ok(key) = receipt::load_key(dir) else {
        return Ok(()); // swallow: fail-safe-by-law — an unreadable key cannot validate a block
    };
    let Some((body, mac)) = receipt::split_receipt(&bytes) else {
        return Ok(()); // swallow: fail-safe-by-law — a malformed arm carries no reservation
    };
    if hmac::hmac_sha256(&key, body) != mac {
        return Ok(()); // corrupt or forged — not a reservation
    }
    let pane = receipt::extract_field(body, "pane").unwrap_or_default();
    if pane.is_empty() {
        return Ok(()); // paneless arm (legacy) blocks nothing
    }
    let kind = receipt::extract_field(body, "kind").unwrap_or_else(|| "omp".into());
    match crate::drain::drain(&kind, Some(&pane)) {
        crate::drain::DrainResult::LiveAgent(msg) => {
            Err(format!("ready: refused — rotation in flight: {msg}"))
        }
        _ => Ok(()),
    }
}

/// A valid claimed.receipt exists — succession concluded for this arm.
fn claim_landed(dir: &Path) -> bool {
    let Some(bytes) = fs::read(dir.join("claimed.receipt")).ok() else {
        return false;
    };
    let Ok(key) = receipt::load_key(dir) else {
        return false;
    };
    let Some((body, mac)) = receipt::split_receipt(&bytes) else {
        return false;
    };
    hmac::hmac_sha256(&key, body) == mac
}

/// Remove a stale linger.lease; true when one was cleared.
pub fn clear_linger(dir: &Path) -> Result<bool, String> {
    match fs::remove_file(dir.join("linger.lease")) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(format!("clear linger.lease: {e}")),
    }
}

/// Write a linger lease on successor-fail (moved from rotate.rs).
pub fn write_linger(dir: &Path, reason: &str) -> Result<(), String> {
    let ts = receipt::timestamp();
    let body = format!("reason={reason}\nts={ts}\n");
    write_atomic(dir, "linger.lease", body.as_bytes()).map(|_| ())
}

/// Claim the line: fenced claimed.receipt + bumped claimed.gen.
/// CARD-0319: a torn claimed.gen under an existing claim REFUSES —
/// never re-fence the line from zero (silent generation collision).
pub fn claim(
    dir: &Path,
    key: &[u8],
    kind: &str,
    model: &str,
    lineage_id: &str,
    claimer: &str,
) -> Result<u64, String> {
    let gen_path = dir.join("claimed.gen");
    if dir.join("claimed.receipt").is_file()
        && matches!(fs::read_to_string(&gen_path), Ok(s) if s.trim().parse::<u64>().is_err())
    {
        return Err(format!(
            "claimed.gen torn (receipt present) — resolve manually: {}",
            gen_path.display()
        ));
    }
    let gen = generation(dir) + 1;
    let receipt = compose(key, kind, model, claimer, lineage_id, "run");
    write_atomic(dir, "claimed.receipt", receipt.as_bytes())?;
    write_atomic(dir, "claimed.gen", format!("{gen}\n").as_bytes())?;
    Ok(gen)
}

/// Compose a receipt in the CARD-0119 wire shape (same bytes as
/// lineage::write_paced) so readers need no new parser.
fn compose(key: &[u8], kind: &str, model: &str, pane: &str, lineage: &str, pace: &str) -> String {
    let ts = receipt::timestamp();
    let pane_line = if pane.is_empty() {
        String::new()
    } else {
        format!("pane={pane}\n")
    };
    let body =
        format!("kind={kind}\nmodel={model}\nlineage={lineage}\n{pane_line}pace={pace}\nts={ts}\n");
    let mac = hmac::hmac_sha256(key, body.as_bytes());
    format!("{body}---\n{}\n", receipt::hex_string(&mac))
}

fn read_arm(dir: &Path, id: &str) -> Result<(String, String, String, Vec<u8>), String> {
    let bytes = fs::read(dir.join("arm.receipt")).map_err(|e| format!("no arm.receipt: {e}"))?;
    let key = receipt::load_key(dir)?;
    let (body, mac) = receipt::split_receipt(&bytes).ok_or("arm.receipt is malformed")?;
    if hmac::hmac_sha256(&key, body) != mac {
        return Err("arm.receipt HMAC mismatch".into());
    }
    let lineage_field =
        receipt::extract_field(body, "lineage").ok_or("arm receipt has no lineage")?;
    if lineage_field != id {
        return Err(format!("lineage {lineage_field} != --lineage {id}"));
    }
    let kind = receipt::extract_field(body, "kind").ok_or("arm receipt has no kind")?;
    let model = receipt::extract_field(body, "model").ok_or("arm receipt has no model")?;
    let pane = receipt::extract_field(body, "pane").unwrap_or_default();
    Ok((kind, model, pane, key))
}
