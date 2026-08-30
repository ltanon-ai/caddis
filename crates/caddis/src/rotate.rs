//! `caddis rotate ready|arm|verify` — HMAC receipt contract (CARD-0119).
//! CARD-0134: every subcommand names a lineage; receipts never share a folder.

use std::fs;
use std::path::Path;

use crate::fold;
use crate::hmac;
use crate::lease;
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
        .ok_or_else(|| Error::Usage("rotate requires a subcommand".into()))?;
    match sub.as_str() {
        "ready" => {
            let (id, rest) = lineage::take(&args[1..]).map_err(Error::Usage)?;
            ready(&rest, &id)
        }
        "arm" => {
            let (id, rest) = lineage::take(&args[1..]).map_err(Error::Usage)?;
            if let Some(a) = rest.first() {
                return Err(Error::Usage(format!("unknown argument {a}")));
            }
            arm(&id)
        }
        "verify" => {
            let (id, rest) = lineage::take(&args[1..]).map_err(Error::Usage)?;
            verify(&rest, &id)
        }
        "handover" => lease::handover_cmd(&args[1..]).map_err(Error::Usage),
        _ => Err(Error::Usage(format!("unknown rotate subcommand {sub}"))),
    }
}

fn ready(args: &[String], id: &str) -> Result<(), Error> {
    let (kind, model, pane) = parse_ready(args)?;
    let dir = lineage::dir(id).map_err(Error::Fail)?;
    lease::refuse_if_blocked(&dir).map_err(Error::Fail)?;
    fs::create_dir_all(&dir).map_err(|e| Error::Fail(format!("mkdir {}: {e}", dir.display())))?;
    let key = receipt::load_or_create_key(&dir).map_err(Error::Fail)?;
    let path = lineage::write_receipt(&dir, "ready.receipt", &key, &kind, &model, &pane, id)
        .map_err(Error::Fail)?;
    println!("LINEAGE {id}");
    let root = lease::stamp_root(&dir).map_err(Error::Fail)?;
    println!("ready: {} model={model} root={root}", path.display());
    Ok(())
}

fn arm(id: &str) -> Result<(), Error> {
    let dir = lineage::dir(id).map_err(Error::Fail)?;
    let (kind, model, pane, key) = read_ready(&dir, id)?;
    let path = lineage::write_receipt(&dir, "arm.receipt", &key, &kind, &model, &pane, id)
        .map_err(Error::Fail)?;
    println!("LINEAGE {id}");
    if let Ok(root) = fs::read_to_string(dir.join("ready.root")) {
        // swallow: fail-safe-by-law — no stamp, no root line (legacy line)
        println!("root: {} (spawn target)", root.trim_end());
    }
    println!("arm: {} model={model}", path.display());
    Ok(())
}

fn verify(args: &[String], id: &str) -> Result<(), Error> {
    let (kind_flag, force) = parse_verify_args(args)?;
    let dir = lineage::dir(id).map_err(Error::Fail)?;
    let (body, key) = read_named(&dir, "arm.receipt")?;
    match_lineage(&body, id)?;
    let model = field_or_fail(&body, "model")?;
    let kind = resolve_kind(&body, kind_flag)?;
    let pane = receipt::extract_field(&body, "pane");
    println!("LINEAGE {id}");
    run_drain(&dir, &kind, &model, pane.as_deref(), force)?;
    succeed(&dir, &key, &kind, &model, id)
}
/// CARD-0301/0302: succession proven — claim, linger hygiene, warn spent.
fn succeed(dir: &Path, key: &[u8], kind: &str, model: &str, id: &str) -> Result<(), Error> {
    // CARD-0301/0302: the CLAIM is the succession act; arm.receipt froze
    // at arm time (the CARD-0150 restamp destroyed the armed identity).
    let claimer = std::env::var("HERDR_PANE_ID").unwrap_or_default(); // swallow: fail-safe-by-law — no pane env, empty claimer
    let note = if lease::classify(dir) {
        "clean handover"
    } else {
        "crash promote — escalate"
    };
    println!("lease: {note}");
    if lease::clear_linger(dir).map_err(Error::Fail)? {
        println!("linger: cleared");
    }
    let gen = lease::claim(dir, key, kind, model, id, &claimer).map_err(Error::Fail)?;
    println!("claim: gen={gen}");
    if let Some(owner) = lineage::owner_pane(dir) {
        println!("owner: pane={owner}");
    }
    if fold::clear_warned(id).map_err(|e| Error::Fail(e.to_string()))? {
        println!("fold: warn spent");
    }
    Ok(())
}

fn read_named(dir: &Path, name: &str) -> Result<(Vec<u8>, Vec<u8>), Error> {
    let bytes = fs::read(dir.join(name)).map_err(|e| Error::Fail(format!("no {name}: {e}")))?;
    let key = receipt::load_key(dir).map_err(Error::Fail)?;
    let (body, mac) = receipt::split_receipt(&bytes)
        .ok_or_else(|| Error::Fail(format!("{name} is malformed")))?;
    if hmac::hmac_sha256(&key, body) != mac {
        return Err(Error::Fail(format!("{name} HMAC mismatch")));
    }
    Ok((body.to_vec(), key))
}

fn match_lineage(body: &[u8], id: &str) -> Result<(), Error> {
    let got = field_or_fail(body, "lineage")?;
    if got != id {
        return Err(Error::Fail(format!("lineage {got} != --lineage {id}")));
    }
    Ok(())
}

/// Extract a required field from the receipt body or fail.
fn field_or_fail(body: &[u8], name: &str) -> Result<String, Error> {
    receipt::extract_field(body, name)
        .ok_or_else(|| Error::Fail(format!("arm receipt has no {name}")))
}

/// Resolve kind: prefer ARM receipt, fall back to --kind flag.
fn resolve_kind(body: &[u8], kind_flag: Option<String>) -> Result<String, Error> {
    receipt::extract_field(body, "kind")
        .or(kind_flag)
        .ok_or_else(|| Error::Fail("no kind in ARM receipt or --kind".into()))
}

/// Run the per-kind drain and handle the result (CARD-0120).
fn run_drain(
    dir: &Path,
    kind: &str,
    model: &str,
    pane: Option<&str>,
    force: bool,
) -> Result<(), Error> {
    match crate::drain::drain(kind, pane) {
        crate::drain::DrainResult::Clean => {
            crate::session::write_on_omp_verify(dir, kind, model).map_err(Error::Fail)?;
            println!("verify: ok model={model} kind={kind}");
            Ok(())
        }
        crate::drain::DrainResult::LiveAgent(msg) => {
            lease::write_linger(dir, &msg).map_err(Error::Fail)?;
            Err(Error::Fail(format!(
                "drain fail ({msg}){force_note}",
                force_note = force_note(force)
            )))
        }
        crate::drain::DrainResult::Unknown(msg) => Err(Error::Fail(format!(
            "drain unknown ({msg}){force_note}",
            force_note = force_note(force)
        ))),
    }
}

/// Parse optional --kind and --force flags for `rotate verify`.
fn parse_verify_args(args: &[String]) -> Result<(Option<String>, bool), Error> {
    let mut kind = None;
    let mut force = false;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if let Some(v) = flag_value(args, &mut i, "--kind")? {
            validate_kind(Some(v.clone()))?;
            kind = Some(v);
        } else if a == "--force" {
            force = true;
        } else {
            return Err(Error::Usage(format!("unknown argument {a}")));
        }
        i += 1;
    }
    Ok((kind, force))
}

fn force_note(force: bool) -> &'static str {
    if force {
        " [--force does not override]"
    } else {
        ""
    }
}

fn read_ready(dir: &Path, id: &str) -> Result<(String, String, String, Vec<u8>), Error> {
    let (body, key) = read_named(dir, "ready.receipt")?;
    match_lineage(&body, id)?;
    let model = field_or_fail(&body, "model")?;
    let kind = field_or_fail(&body, "kind")?;
    let pane = receipt::extract_field(&body, "pane").unwrap_or_default();
    Ok((kind, model, pane, key))
}

fn parse_ready(args: &[String]) -> Result<(String, String, String), Error> {
    let (kind, model, pane) = scan_ready_flags(args)?;
    let kind = validate_kind(kind)?;
    let model = validate_model(model)?;
    Ok((kind, model, pane.unwrap_or_default()))
}

type ReadyScan = (Option<String>, Option<String>, Option<String>);
fn scan_ready_flags(args: &[String]) -> Result<ReadyScan, Error> {
    let mut kind = None;
    let mut model = None;
    let mut pane = None;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if let Some(v) = flag_value(args, &mut i, "--kind")? {
            kind = Some(v);
        } else if let Some(v) = flag_value(args, &mut i, "--model")? {
            model = Some(v);
        } else if let Some(v) = flag_value(args, &mut i, "--pane")? {
            pane = Some(v);
        } else {
            return Err(Error::Usage(format!("unknown argument {a}")));
        }
        i += 1;
    }
    Ok((kind, model, pane))
}

fn validate_kind(kind: Option<String>) -> Result<String, Error> {
    let kind = kind.ok_or_else(|| Error::Usage("rotate ready requires --kind".into()))?;
    if !matches!(kind.as_str(), "omp" | "claude" | "qpi") {
        return Err(Error::Usage(format!("unknown kind {kind}")));
    }
    Ok(kind)
}

fn validate_model(model: Option<String>) -> Result<String, Error> {
    let model = model.ok_or_else(|| Error::Usage("rotate ready requires --model".into()))?;
    if model.is_empty() {
        return Err(Error::Usage(
            "rotate ready --model must not be empty".into(),
        ));
    }
    Ok(model)
}

fn flag_value(args: &[String], i: &mut usize, flag: &str) -> Result<Option<String>, Error> {
    let a = args[*i].as_str();
    let prefix = format!("{flag}=");
    if let Some(v) = a.strip_prefix(&prefix) {
        return Ok(Some(v.to_string()));
    }
    if a == flag {
        *i += 1;
        let v = args
            .get(*i)
            .ok_or_else(|| Error::Usage(format!("missing {flag} value")))?;
        return Ok(Some(v.clone()));
    }
    Ok(None)
}
