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
    Ok(home
        .join(".caddis")
        .join("rotation")
        .join("lines")
        .join(id))
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
    let ts = receipt::timestamp();
    let pane_line = if pane.is_empty() {
        String::new()
    } else {
        format!("pane={pane}\n")
    };
    let body = format!("kind={kind}\nmodel={model}\nlineage={lineage}\n{pane_line}ts={ts}\n");
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
}
