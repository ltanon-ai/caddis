//! prove.rs — CARD-0316. The trusted evidence runner (OP6/C6): the
//! ORGAN executes the command and mints ONE host-owned HMAC-stamped
//! receipt — cmd, exit, fnv1a hash of the combined output. Agent-posted
//! transcripts are leads, never proof (E6); this is what the host
//! cites instead. The receipt is tamper-evidence under the shared
//! lineage key, never signer identity. Consumption is CARD-0317.

use std::fs;
use std::process::{Command, ExitCode};

use crate::lease::write_atomic;
use crate::lineage;
use crate::receipt;

/// `caddis prove --lineage <id> -- <cmd...>` — the exit code MIRRORS
/// the command's; the receipt (not the exit) is the evidence.
pub fn cmd(args: &[String]) -> ExitCode {
    match run(args) {
        Ok(code) => ExitCode::from(code),
        Err(Error::Usage(s)) => {
            eprintln!("{s}");
            eprint!("{}", crate::USAGE);
            ExitCode::from(2)
        }
        Err(Error::Fail(s)) => {
            eprintln!("{s}");
            ExitCode::from(1)
        }
    }
}

enum Error {
    Usage(String),
    Fail(String),
}

fn run(args: &[String]) -> Result<u8, Error> {
    let (id, rest) = lineage::take(args).map_err(Error::Usage)?;
    let argv = take_argv(&rest)?;
    let dir = lineage::dir(&id).map_err(Error::Fail)?;
    if !dir.is_dir() {
        return Err(Error::Fail(format!("no lineage {id}")));
    }
    let out = Command::new(&argv[0])
        .args(&argv[1..])
        .output()
        .map_err(|e| Error::Fail(format!("prove failed to spawn `{}`: {e}", argv[0])))?;
    mint(&dir, &argv, &out)
}

/// `-- <cmd...>` -> argv; nothing before the `--`, nothing missing after.
fn take_argv(rest: &[String]) -> Result<Vec<String>, Error> {
    let pos = rest
        .iter()
        .position(|a| a == "--")
        .ok_or_else(|| Error::Usage("prove requires `--` before the command".into()))?;
    if pos > 0 {
        return Err(Error::Usage(format!("unknown argument {}", rest[0])));
    }
    if rest.len() < 2 {
        return Err(Error::Usage("prove requires a command after `--`".into()));
    }
    Ok(rest[1..].to_vec())
}

/// Append one HMAC-stamped receipt; the exit code mirrors the command.
fn mint(dir: &std::path::Path, argv: &[String], out: &std::process::Output) -> Result<u8, Error> {
    let code = out.status.code().unwrap_or(1) as u8;
    let combined = [out.stdout.as_slice(), out.stderr.as_slice()].concat();
    let out_hash = caddis_organs::util::fnv1a(&String::from_utf8_lossy(&combined));
    let ts = receipt::timestamp();
    let key = receipt::load_key(dir).map_err(Error::Fail)?;
    let cmd_str = argv.join(" ");
    let mac_str = format!("{cmd_str}|{code}|{out_hash:x}|{}|{ts}", combined.len());
    let mac = crate::hmac::hmac_sha256(&key, mac_str.as_bytes());
    let line = format!(
        "{{\"cmd\":\"{}\",\"exit\":{code},\"out_hash\":\"{out_hash:x}\",\"out_bytes\":{},\"ts\":\"{ts}\",\"mac\":\"{}\"}}\n",
        cmd_str.replace('\\', "\\\\").replace('"', "\\\""),
        combined.len(),
        receipt::hex_string(&mac),
    );
    let mut all = fs::read_to_string(dir.join("prove.jsonl")).unwrap_or_default();
    all.push_str(&line);
    write_atomic(dir, "prove.jsonl", all.as_bytes()).map_err(Error::Fail)?;
    println!(
        "prove: exit={code} out_hash={out_hash:x} receipt={}",
        dir.join("prove.jsonl").display()
    );
    Ok(code)
}

/// CARD-0317: does the line's prove.jsonl carry a valid host receipt
/// covering THIS check (same cmd string, exit 0, mac verifies under
/// the lineage key)? Tamper-evidence, never signer identity — the
/// shared-key law (time-machine-vision §OP6). Prose stays never-proof.
pub fn receipt_covers(dir: &std::path::Path, cmd: &str) -> bool {
    let Ok(text) = std::fs::read_to_string(dir.join("prove.jsonl")) else {
        return false;
    };
    let Ok(key) = receipt::load_key(dir) else {
        return false;
    };
    text.lines().any(|l| line_covers(&key, l, cmd))
}

fn line_covers(key: &[u8], line: &str, cmd: &str) -> bool {
    let (Some(r_cmd), Some(ts), Some(mac_hex), Some(hash_hex)) = (
        field_str(line, "cmd"),
        field_str(line, "ts"),
        field_str(line, "mac"),
        field_str(line, "out_hash"),
    ) else {
        return false;
    };
    let (Some(exit), Some(bytes)) = (field_num(line, "exit"), field_num(line, "out_bytes")) else {
        return false;
    };
    if r_cmd != cmd || exit != 0 {
        return false;
    }
    let expect = crate::hmac::hmac_sha256(
        key,
        format!("{r_cmd}|{exit}|{hash_hex}|{bytes}|{ts}").as_bytes(),
    );
    receipt::hex_string(&expect) == mac_hex
}

/// `"key":"value"` from a flat one-line receipt (unescapes \" and \\).
fn field_str(line: &str, key: &str) -> Option<String> {
    let marker = format!("\"{key}\":\"");
    let a = line.find(&marker)? + marker.len();
    let mut out = String::new();
    let mut chars = line[a..].chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('t') => out.push('\t'),
                Some('\\') => out.push('\\'),
                Some('"') => out.push('"'),
                _ => return None, // unknown escape: not our line shape
            },
            _ => out.push(c),
        }
    }
    None
}

/// `"key":<digits>` from a flat one-line receipt.
fn field_num(line: &str, key: &str) -> Option<u64> {
    let marker = format!("\"{key}\":");
    let a = line.find(&marker)? + marker.len();
    let rest = line[a..].trim_start();
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    (end > 0).then(|| rest[..end].parse().ok()).flatten()
}
