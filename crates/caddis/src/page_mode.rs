//! page_mode.rs — CARD-0188. Write/read ~/.caddis/pager/<session>/mode.
//! One-shot. Invalid --set fails. Missing --set prints the resolved file.

use std::fs;

use crate::page::Error;

pub fn run(args: &[String]) -> Result<(), Error> {
    let session = crate::page::flag(args, "--session")?;
    let set = opt_set(args)?;
    let dir = crate::page::session_dir(session)?;
    if let Some(v) = set {
        fs::create_dir_all(&dir)
            .map_err(|e| Error::Fail(format!("mkdir {}: {e}", dir.display())))?;
        fs::write(dir.join("mode"), format!("{v}\n"))
            .map_err(|e| Error::Fail(format!("write mode: {e}")))?;
        println!("mode={v}");
        return Ok(());
    }
    let text = fs::read_to_string(dir.join("mode")).unwrap_or_default();
    let mode = if text.trim() == "page" {
        "page"
    } else {
        "observe"
    };
    println!("mode={mode}");
    Ok(())
}

fn opt_set(args: &[String]) -> Result<Option<&str>, Error> {
    let has = args.iter().any(|a| a == "--set" || a.starts_with("--set="));
    if !has {
        return Ok(None);
    }
    let v = crate::page::flag(args, "--set")?;
    if v == "page" || v == "observe" {
        Ok(Some(v))
    } else {
        Err(Error::Usage(format!(
            "--set must be page or observe, got {v}"
        )))
    }
}
