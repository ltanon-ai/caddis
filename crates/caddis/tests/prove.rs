//! prove.rs — CARD-0316. The trusted evidence runner: `caddis prove`
//! executes the command and mints ONE host-owned HMAC-stamped receipt
//! (cmd/exit/out_hash). Agent prose is never proof (OP6/E6); this organ
//! is what the host cites instead. Hermetic HOME; never the live bag.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

#[path = "../src/hmac.rs"]
mod hmac;

static SEQ: AtomicU64 = AtomicU64::new(0);
const KEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

fn tmp(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("caddis-prove-{tag}-{n}"));
    let _ = fs::remove_dir_all(&p); // swallow: best-effort-cleanup — stale temp dir from a prior run
    fs::create_dir_all(&p).unwrap();
    p
}

struct World {
    home: PathBuf,
    line: PathBuf,
}

impl World {
    fn new(tag: &str) -> Self {
        let root = tmp(tag);
        let home = root.join("home");
        fs::create_dir_all(&home).unwrap();
        Self {
            line: home.join(".caddis/rotation/lines/lin-p"),
            home,
        }
    }

    fn run(&self, args: &[&str]) -> (String, String, i32) {
        let out = Command::new(env!("CARGO_BIN_EXE_caddis"))
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("CADDIS_HMAC_KEY", KEY)
            .env("CADDIS_SKIP_WARDEN", "1")
            .env_remove("HERDR_PANE_ID")
            .output()
            .expect("caddis must spawn");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    }

    fn seed(&self) {
        let (o, e, c) = self.run(&[
            "rotate",
            "ready",
            "--kind",
            "omp",
            "--model",
            "m1",
            "--lineage",
            "lin-p",
        ]);
        assert_eq!(c, 0, "ready: {o}{e}");
        let (o, e, c) = self.run(&["rotate", "arm", "--lineage", "lin-p"]);
        assert_eq!(c, 0, "arm: {o}{e}");
    }

    /// True-exit command per platform.
    fn ok_cmd() -> Vec<&'static str> {
        if cfg!(windows) {
            vec!["cmd", "/c", "exit 0"]
        } else {
            vec!["true"]
        }
    }
}

/// CARD-0316: the organ runs, records, stamps — and the receipt
/// mac-verifies under the lineage key.
#[test]
fn prove_records_cmd_exit_and_mac() {
    let w = World::new("record");
    w.seed();
    let cmd = World::ok_cmd();
    let mut call: Vec<&str> = vec!["prove", "--lineage", "lin-p", "--"];
    call.extend_from_slice(&cmd);
    let (o, e, c) = w.run(&call);
    assert_eq!(c, 0, "prove: {o}{e}");
    assert!(o.contains("prove: exit=0"), "names the exit: {o}");
    assert!(o.contains("receipt="), "names the evidence path: {o}");
    let raw = fs::read_to_string(w.line.join("prove.jsonl")).expect("receipt written");
    let line = raw.lines().last().expect("one line");
    assert!(line.contains("\"exit\":0"), "exit recorded: {line}");
    assert!(
        line.contains(&format!("\"cmd\":\"{}\"", cmd.join(" "))),
        "cmd recorded: {line}"
    );
    // re-derive the mac exactly as the organ does
    let field = |k: &str| {
        let key = format!("\"{k}\":\"");
        let a = line.find(&key).expect(k) + key.len();
        let b = line[a..].find('"').expect("close") + a;
        line[a..b].to_string()
    };
    let mac = field("mac");
    let cmd_f = field("cmd").replace("\\\"", "\"").replace("\\\\", "\\");
    let ts = field("ts");
    let out_hash = field("out_hash");
    let exit: u64 = line
        .split("\"exit\":")
        .nth(1)
        .unwrap()
        .split(',')
        .next()
        .unwrap()
        .parse()
        .unwrap();
    let out_bytes: u64 = line
        .split("\"out_bytes\":")
        .nth(1)
        .unwrap()
        .split(',')
        .next()
        .unwrap()
        .parse()
        .unwrap();
    let expect = hmac::hmac_sha256(
        &hex_key(),
        format!("{cmd_f}|{exit}|{out_hash}|{out_bytes}|{ts}").as_bytes(),
    );
    assert_eq!(mac, to_hex(&expect), "mac verifies under the lineage key");
}

/// A failing command is recorded honestly: prove mirrors its exit code.
#[test]
fn prove_mirrors_nonzero_exit() {
    let w = World::new("mirror");
    w.seed();
    let argv: Vec<&str> = if cfg!(windows) {
        vec!["cmd", "/c", "exit 3"]
    } else {
        vec!["sh", "-c", "exit 3"]
    };
    let mut call: Vec<&str> = vec!["prove", "--lineage", "lin-p", "--"];
    call.extend_from_slice(&argv);
    let (o, e, c) = w.run(&call);
    assert_eq!(c, 3, "mirrors the command exit: {o}{e}");
    let raw = fs::read_to_string(w.line.join("prove.jsonl")).expect("receipt written");
    assert!(raw.contains("\"exit\":3"), "nonzero recorded: {raw}");
}

/// A run that never happened is not evidence: no spawn -> no receipt.
#[test]
fn prove_no_receipt_when_spawn_fails() {
    let w = World::new("nospawn");
    w.seed();
    let (o, e, c) = w.run(&["prove", "--lineage", "lin-p", "--", "no-such-bin-x1"]);
    assert_eq!(c, 1, "spawn failure is exit 1: {o}{e}");
    assert!(e.contains("failed to spawn"), "names it: {e}");
    assert!(!w.line.join("prove.jsonl").exists(), "no receipt minted");
}

/// No `--` (or nothing after it) is usage, never a quiet run.
#[test]
fn prove_usage_without_dashdash() {
    let w = World::new("usage");
    w.seed();
    let (o, e, c) = w.run(&["prove", "--lineage", "lin-p"]);
    assert_eq!(c, 2, "usage exit: {o}{e}");
    let (_o2, _e2, c2) = w.run(&["prove", "--lineage", "lin-p", "--"]);
    assert_eq!(c2, 2, "empty argv is usage too");
}

fn hex_key() -> Vec<u8> {
    (0..KEY.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&KEY[i..i + 2], 16).unwrap())
        .collect()
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
