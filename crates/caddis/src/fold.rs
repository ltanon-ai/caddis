//! fold.rs — CARD-0135 + CARD-0210. Warn at cap, hold until era, then deny.

use std::env;
use std::fs;
use std::path::PathBuf;

use crate::lineage;

pub enum Error {
    Usage(String),
    Fail(String),
    Deny,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Usage(s) | Error::Fail(s) => write!(f, "{s}"),
            Error::Deny => write!(f, "FOLD deny"),
        }
    }
}

pub fn run(args: &[String]) -> Result<(), Error> {
    let sub = args
        .first()
        .ok_or_else(|| Error::Usage("fold requires a subcommand".into()))?;
    match sub.as_str() {
        "threshold" => threshold(&args[1..]),
        "tick" => tick(&args[1..]),
        "cap" => cap(&args[1..]),
        _ => Err(Error::Usage(format!("unknown fold subcommand {sub}"))),
    }
}

fn threshold(args: &[String]) -> Result<(), Error> {
    let at = parse_u32(args, "--at")?;
    if !(1..=99).contains(&at) {
        return Err(Error::Usage("fold threshold --at must be 1..=99".into()));
    }
    let path = fold_at_path()?;
    mkdir_parent(&path)?;
    fs::write(&path, format!("{at}\n")).map_err(|e| Error::Fail(format!("write fold.at: {e}")))?;
    println!("fold.at {at}");
    Ok(())
}

fn tick(args: &[String]) -> Result<(), Error> {
    let (id, rest) = lineage::take(args).map_err(Error::Usage)?;
    let used = flag_u32(&rest, "--used-pct")?
        .ok_or_else(|| Error::Usage("fold requires --used-pct".into()))?;
    if used > 100 {
        return Err(Error::Usage("fold tick --used-pct must be 0..=100".into()));
    }
    let tokens = flag_u32(&rest, "--used-tokens")?;
    let t = pct_limit(&id)?;
    let cap = read_cap(&id)?;
    decide(&id, used >= t || over_tokens(tokens, cap))
}

fn pct_limit(id: &str) -> Result<u32, Error> {
    if arm_kind(id).as_deref() == Some("claude") {
        return Ok(30);
    }
    read_threshold()
}

fn over_tokens(tokens: Option<u32>, cap: Option<u32>) -> bool {
    match (tokens, cap) {
        (Some(tok), Some(c)) => tok >= c,
        _ => false,
    }
}

fn cap(args: &[String]) -> Result<(), Error> {
    let (id, rest) = lineage::take(args).map_err(Error::Usage)?;
    let n = parse_u32(&rest, "--tokens")?;
    let path = lineage::dir(&id).map_err(Error::Fail)?.join("cap.tokens");
    mkdir_parent(&path)?;
    fs::write(&path, format!("{n}\n"))
        .map_err(|e| Error::Fail(format!("write cap.tokens: {e}")))?;
    println!("cap.tokens {n}");
    Ok(())
}

fn decide(id: &str, over: bool) -> Result<(), Error> {
    let warned = already_warned(id)?;
    if !over && !warned {
        println!("FOLD quiet");
        crate::pace::feed(id);
        return Ok(());
    }
    if !warned {
        write_warned(id)?;
        println!("FOLD warn");
        crate::pace::feed(id);
        return Ok(());
    }
    if !era_open(id)? {
        println!("FOLD hold");
        crate::pace::feed(id);
        return Ok(());
    }
    println!("FOLD deny");
    crate::pace::feed(id);
    Err(Error::Deny)
}

fn parse_u32(args: &[String], flag: &str) -> Result<u32, Error> {
    let mut val = None;
    let mut i = 0;
    while i < args.len() {
        if let Some(v) = flag_val(args, &mut i, flag)? {
            val = Some(v);
        } else {
            return Err(Error::Usage(format!("unknown argument {}", args[i])));
        }
        i += 1;
    }
    let raw = val.ok_or_else(|| Error::Usage(format!("fold requires {flag}")))?;
    raw.parse::<u32>()
        .map_err(|_| Error::Usage(format!("invalid {flag} {raw}")))
}

fn flag_u32(args: &[String], flag: &str) -> Result<Option<u32>, Error> {
    let mut val = None;
    let mut i = 0;
    while i < args.len() {
        if let Some(v) = flag_val(args, &mut i, flag)? {
            val = Some(v);
        }
        i += 1;
    }
    match val {
        None => Ok(None),
        Some(raw) => raw
            .parse()
            .map(Some)
            .map_err(|_| Error::Usage(format!("invalid {flag} {raw}"))),
    }
}

fn era_open(id: &str) -> Result<bool, Error> {
    let path = caddis_home()?.join("pager").join(id).join("era");
    let Ok(text) = fs::read_to_string(path) else {
        return Ok(false);
    };
    Ok(text.lines().any(|l| l.trim() == "open=1"))
}

fn arm_kind(id: &str) -> Option<String> {
    let path = lineage::dir(id).ok()?.join("arm.receipt");
    let text = fs::read_to_string(path).ok()?;
    text.lines()
        .find_map(|l| l.strip_prefix("kind=").map(str::trim).map(str::to_string))
}

fn read_cap(id: &str) -> Result<Option<u32>, Error> {
    let path = lineage::dir(id).map_err(Error::Fail)?.join("cap.tokens");
    let Ok(text) = fs::read_to_string(path) else {
        if arm_kind(id).as_deref() == Some("claude") {
            return Ok(None);
        }
        return Ok(Some(170_000));
    };
    let n: u32 = text
        .trim()
        .parse()
        .map_err(|_| Error::Fail("cap.tokens is not a number".into()))?;
    if n == 0 {
        return Ok(None);
    }
    Ok(Some(n))
}

fn flag_val(args: &[String], i: &mut usize, flag: &str) -> Result<Option<String>, Error> {
    let a = args[*i].as_str();
    let prefix = format!("{flag}=");
    if let Some(v) = a.strip_prefix(&prefix) {
        return Ok(Some(v.to_string()));
    }
    if a != flag {
        return Ok(None);
    }
    *i += 1;
    let v = args
        .get(*i)
        .ok_or_else(|| Error::Usage(format!("missing {flag} value")))?;
    Ok(Some(v.clone()))
}

fn read_threshold() -> Result<u32, Error> {
    let path = fold_at_path()?;
    let Ok(text) = fs::read_to_string(&path) else {
        return Ok(50);
    };
    let n: u32 = text
        .trim()
        .parse()
        .map_err(|_| Error::Fail("fold.at is not a number".into()))?;
    if (1..=99).contains(&n) {
        Ok(n)
    } else {
        Err(Error::Fail("fold.at out of range".into()))
    }
}

/// CARD-0151: succession spends the warn. A clean rotate verify clears
/// fold.state so the successor's first tick is quiet, not deny. Without
/// this the loop bricks: warned once, every successor denied forever.
pub fn clear_warned(id: &str) -> Result<bool, Error> {
    let path = state_path(id)?;
    if !path.is_file() {
        return Ok(false);
    }
    fs::remove_file(&path).map_err(|e| Error::Fail(format!("clear fold.state: {e}")))?;
    Ok(true)
}

fn already_warned(id: &str) -> Result<bool, Error> {
    Ok(state_path(id)?.is_file())
}

fn write_warned(id: &str) -> Result<(), Error> {
    let path = state_path(id)?;
    mkdir_parent(&path)?;
    fs::write(&path, "warned=1\n").map_err(|e| Error::Fail(format!("write fold.state: {e}")))
}

fn state_path(id: &str) -> Result<PathBuf, Error> {
    Ok(lineage::dir(id).map_err(Error::Fail)?.join("fold.state"))
}

fn fold_at_path() -> Result<PathBuf, Error> {
    Ok(caddis_home()?.join("fold.at"))
}

fn caddis_home() -> Result<PathBuf, Error> {
    let home = env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or_else(|| Error::Fail("HOME is unset".into()))?;
    Ok(home.join(".caddis"))
}

fn mkdir_parent(path: &std::path::Path) -> Result<(), Error> {
    if let Some(p) = path.parent() {
        fs::create_dir_all(p).map_err(|e| Error::Fail(format!("mkdir: {e}")))?;
    }
    Ok(())
}
