//! packet.rs — CARD-0136. Successor reads a query, not a letter.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::hmac;
use crate::lineage;
use crate::receipt;

pub enum Error {
    Usage(String),
    Fail(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Usage(s) | Error::Fail(s) => write!(f, "{s}"),
        }
    }
}

pub fn run(args: &[String]) -> Result<(), Error> {
    let sub = args
        .first()
        .ok_or_else(|| Error::Usage("lineage requires a subcommand".into()))?;
    match sub.as_str() {
        "packet" => packet(&args[1..]),
        _ => Err(Error::Usage(format!("unknown lineage subcommand {sub}"))),
    }
}

/// CARD-0257: the orientation packet is three zones — the soul HEAD
/// (session-stable identity), the arm-receipt body (unchanged), and the
/// valence TAIL (volatile body state). HEAD and TAIL are pure additions;
/// the arm lines and their order stay byte-identical.
fn packet(args: &[String]) -> Result<(), Error> {
    let (id, rest) = lineage::take(args).map_err(Error::Usage)?;
    if let Some(a) = rest.first() {
        return Err(Error::Usage(format!("unknown argument {a}")));
    }
    let dir = lineage::dir(&id).map_err(Error::Fail)?;
    let home = home()?;
    print!("{}", crate::soul_cli::identity_for(&id).unwrap_or_default());
    arm_body(&dir, &id)?;
    println!("{}", crate::packet_tail::tail(&dir, &home, &id));
    Ok(())
}

/// Print the arm-receipt body (the unchanged middle zone). Validates the
/// arm fields and the lineage match, then prints the arm lines in their
/// fixed byte-identical order.
fn arm_body(dir: &Path, id: &str) -> Result<(), Error> {
    let body = read_arm(dir)?;
    let kind = field(&body, "kind")?;
    let model = field(&body, "model")?;
    let got = field(&body, "lineage")?;
    if got != id {
        return Err(Error::Fail(format!("arm lineage {got} != --lineage {id}")));
    }
    println!("LINEAGE {id}");
    println!("kind={kind}");
    println!("model={model}");
    println!("lineage={id}");
    if let Some(pane) = receipt::extract_field(&body, "pane") {
        println!("pane={pane}");
    }
    println!("fold_at={}", fold_at());
    println!("fold={}", fold_status(dir));
    Ok(())
}

fn read_arm(dir: &Path) -> Result<Vec<u8>, Error> {
    let bytes =
        fs::read(dir.join("arm.receipt")).map_err(|e| Error::Fail(format!("no arm: {e}")))?;
    let key = receipt::load_key(dir).map_err(Error::Fail)?;
    let (body, mac) = receipt::split_receipt(&bytes)
        .ok_or_else(|| Error::Fail("arm receipt is malformed".into()))?;
    if hmac::hmac_sha256(&key, body) != mac {
        return Err(Error::Fail("arm receipt HMAC mismatch".into()));
    }
    Ok(body.to_vec())
}

fn field(body: &[u8], name: &str) -> Result<String, Error> {
    receipt::extract_field(body, name).ok_or_else(|| Error::Fail(format!("arm has no {name}")))
}

fn fold_at() -> u32 {
    let Ok(home) = home() else {
        return 50;
    };
    let Ok(text) = fs::read_to_string(home.join(".caddis").join("fold.at")) else {
        return 50;
    };
    text.trim()
        .parse()
        .ok()
        .filter(|n| (1..=99).contains(n))
        .unwrap_or(50)
}

fn fold_status(dir: &Path) -> &'static str {
    if dir.join("fold.state").is_file() {
        "warned"
    } else {
        "quiet"
    }
}

fn home() -> Result<PathBuf, Error> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or_else(|| Error::Fail("HOME is unset".into()))
}
