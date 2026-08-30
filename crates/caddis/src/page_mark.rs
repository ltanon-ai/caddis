//! page_mark.rs — CARD-0202. Write/read ~/.caddis/pager/<session>/mark.
//! One-shot. Invalid --set fails. Missing --set prints the session file.

use std::env;
use std::fs;
use std::path::Path;

use crate::page::{flag, session_dir, Error};

pub fn run(args: &[String]) -> Result<(), Error> {
    let session = flag(args, "--session")?;
    let dir = session_dir(session)?;
    fs::create_dir_all(&dir).map_err(|e| Error::Fail(format!("mkdir: {e}")))?;
    let path = dir.join("mark");
    if has_flag(args, "--set") {
        let v = flag(args, "--set")?;
        let n: u64 = v.parse().ok().filter(|&n| n > 0).ok_or_else(|| {
            Error::Usage(format!("page mark --set wants a positive integer, got {v}"))
        })?;
        fs::write(&path, format!("{n}\n")).map_err(|e| Error::Fail(format!("write mark: {e}")))?;
    }
    let n = read_u64(&path).unwrap_or(0);
    println!("mark={n}");
    Ok(())
}

fn has_flag(args: &[String], name: &str) -> bool {
    let prefix = format!("{name}=");
    args.iter().any(|a| a == name || a.starts_with(&prefix))
}

pub(crate) fn resolve(pager: &Path, session: &str) -> Option<u64> {
    env::var("CADDIS_PAGE_MARK_TOKENS")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .or_else(|| read_u64(&pager.join(session).join("mark")))
        .or_else(|| read_u64(&pager.join("mark")))
        .filter(|&n| n > 0)
}

fn read_u64(path: &Path) -> Option<u64> {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
}
