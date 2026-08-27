//! fold.rs — CARD-0135. UI threshold organ: warn once, then deny.

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
        _ => Err(Error::Usage(format!("unknown fold subcommand {sub}"))),
    }
}

fn threshold(args: &[String]) -> Result<(), Error> {
    let at = parse_u32(args, "--at")?;
    if !(1..=99).contains(&at) {
        return Err(Error::Usage(
            "fold threshold --at must be 1..=99".into(),
        ));
    }
    let path = fold_at_path()?;
    mkdir_parent(&path)?;
    fs::write(&path, format!("{at}\n")).map_err(|e| Error::Fail(format!("write fold.at: {e}")))?;
    println!("fold.at {at}");
    Ok(())
}

fn tick(args: &[String]) -> Result<(), Error> {
    let (id, rest) = lineage::take(args).map_err(Error::Usage)?;
    let used = parse_u32(&rest, "--used-pct")?;
    if used > 100 {
        return Err(Error::Usage("fold tick --used-pct must be 0..=100".into()));
    }
    decide(&id, used, read_threshold()?)
}

fn decide(id: &str, used: u32, t: u32) -> Result<(), Error> {
    let warned = already_warned(id)?;
    if used < t && !warned {
        println!("FOLD quiet");
        return Ok(());
    }
    if warned {
        println!("FOLD deny");
        return Err(Error::Deny);
    }
    write_warned(id)?;
    println!("FOLD warn");
    Ok(())
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
