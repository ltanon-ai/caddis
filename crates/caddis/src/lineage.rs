//! lineage.rs — CARD-0134. A rotation line is named, never global.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::hmac;
use crate::receipt;

/// Take `--lineage <id>` out of args. Missing/invalid → Err (usage).
pub fn take(args: &[String]) -> Result<(String, Vec<String>), String> {
    let mut id = None;
    let mut rest = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if let Some(v) = eat_lineage(args, &mut i)? {
            set_id(&mut id, v)?;
        } else {
            rest.push(args[i].clone());
            i += 1;
        }
    }
    let id = id.ok_or_else(|| "rotate requires --lineage".to_string())?;
    validate(&id)?;
    Ok((id, rest))
}

fn eat_lineage(args: &[String], i: &mut usize) -> Result<Option<String>, String> {
    let a = args[*i].as_str();
    if let Some(v) = a.strip_prefix("--lineage=") {
        *i += 1;
        return Ok(Some(v.to_string()));
    }
    if a != "--lineage" {
        return Ok(None);
    }
    *i += 1;
    let v = args
        .get(*i)
        .ok_or_else(|| "missing --lineage value".to_string())?;
    *i += 1;
    Ok(Some(v.clone()))
}

fn set_id(slot: &mut Option<String>, v: String) -> Result<(), String> {
    if slot.is_some() {
        return Err("duplicate --lineage".into());
    }
    *slot = Some(v);
    Ok(())
}

/// `^[a-z][a-z0-9-]{1,31}$` — 2..=32 chars, directory-safe.
pub fn validate(id: &str) -> Result<(), String> {
    let b = id.as_bytes();
    if !(2..=32).contains(&b.len()) {
        return Err("rotate --lineage must be 2..=32 chars".into());
    }
    let ok = b[0].is_ascii_lowercase()
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-');
    if !ok {
        return Err("rotate --lineage must match [a-z][a-z0-9-]{1,31}".into());
    }
    Ok(())
}

pub fn dir(id: &str) -> Result<PathBuf, String> {
    let home = env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or_else(|| "HOME is unset".to_string())?;
    Ok(home.join(".caddis").join("rotation").join("lines").join(id))
}

/// CARD-0302: the line's owner pane — claimed receipt first, arm
/// fallback (legacy lines). arm.receipt now freezes at arm time; the
/// CARD-0150 restamp is retired (it destroyed the armed identity).
pub fn owner_pane(dir: &Path) -> Option<String> {
    let key = receipt::load_key(dir).ok()?; // swallow: fail-safe-by-law — an unreadable key has no owner to name
    for name in ["claimed.receipt", "arm.receipt"] {
        // swallow: fail-safe-by-law — a missing receipt falls through to the next source
        let Ok(bytes) = fs::read(dir.join(name)) else {
            continue;
        };
        // swallow: fail-safe-by-law — a malformed receipt is not ownership evidence
        let Some((body, mac)) = receipt::split_receipt(&bytes) else {
            continue;
        };
        if hmac::hmac_sha256(&key, body) != mac {
            continue; // corrupt or forged -> not evidence of ownership
        }
        if let Some(p) = receipt::extract_field(body, "pane") {
            if !p.is_empty() {
                return Some(p);
            }
        }
    }
    None
}

pub fn write_receipt(
    dir: &Path,
    name: &str,
    key: &[u8],
    kind: &str,
    model: &str,
    pane: &str,
    lineage: &str,
) -> Result<PathBuf, String> {
    write_paced(dir, name, key, kind, model, pane, lineage, "run")
}
// Receipt fields are the contract (CARD-0119/0214); the count is the shape.
#[allow(clippy::too_many_arguments)] // receipt fields ARE the wire contract (CARD-0119/0214); a params struct would be a second unvalidated shape
pub fn write_paced(
    dir: &Path,
    name: &str,
    key: &[u8],
    kind: &str,
    model: &str,
    pane: &str,
    lineage: &str,
    pace: &str,
) -> Result<PathBuf, String> {
    if pace != "run" && pace != "stop" {
        return Err("pace must be run or stop".into());
    }
    let ts = receipt::timestamp();
    let pane_line = if pane.is_empty() {
        String::new()
    } else {
        format!("pane={pane}\n")
    };
    let body =
        format!("kind={kind}\nmodel={model}\nlineage={lineage}\n{pane_line}pace={pace}\nts={ts}\n");
    let mac = hmac::hmac_sha256(key, body.as_bytes());
    let text = format!("{body}---\n{}\n", receipt::hex_string(&mac));
    let path = dir.join(name);
    fs::write(&path, text.as_bytes()).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_short_and_path() {
        assert!(validate("t").is_err());
        assert!(validate("../x").is_err());
        assert!(validate("Line-A").is_err());
        assert!(validate("line-a").is_ok());
    }

    #[test]
    fn owner_pane_prefers_claim_falls_back_rejects_corrupt() {
        let dir = std::env::temp_dir().join("caddis-owner-pane-test");
        let _ = fs::remove_dir_all(&dir); // swallow: best-effort-cleanup — stale temp dir from a prior run
        fs::create_dir_all(&dir).unwrap();
        let key = receipt::load_or_create_key(&dir).unwrap();
        write_receipt(&dir, "arm.receipt", &key, "omp", "m", "w1:p1", "lin-x").unwrap();
        assert_eq!(
            owner_pane(&dir).as_deref(),
            Some("w1:p1"),
            "legacy: arm pane"
        );
        write_receipt(&dir, "claimed.receipt", &key, "omp", "m", "w1:p2", "lin-x").unwrap();
        assert_eq!(owner_pane(&dir).as_deref(), Some("w1:p2"), "claim wins");
        let claimed = dir.join("claimed.receipt");
        let mut bytes = fs::read(&claimed).unwrap();
        bytes[3] ^= 0xff; // inside the HMAC-covered body
        fs::write(&claimed, bytes).unwrap();
        assert_eq!(owner_pane(&dir).as_deref(), Some("w1:p1"), "corrupt -> arm");
    }
}
