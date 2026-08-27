//! session.rs — CARD-0125: omp verify writes session.receipt.
//!
//! Hermetic: fake HOME, pinned HMAC key, drain fixtures. Never
//! touches ~/.claude or the operator's ~/.caddis.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

const TEST_KEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

fn tmp(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("caddis-session-{tag}-{n}"));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

struct World {
    home: PathBuf,
    rot: PathBuf,
    herdr: PathBuf,
    claude: PathBuf,
    qpi: PathBuf,
}

impl World {
    fn new(tag: &str) -> Self {
        let root = tmp(tag);
        let home = root.join("home");
        fs::create_dir_all(&home).unwrap();
        let rot = home.join(".caddis").join("rotation");
        let herdr = root.join("herdr.json");
        let claude = root.join("claude-reg.json");
        let qpi = root.join("qpi.json");
        for f in [&herdr, &claude, &qpi] {
            fs::write(f, "").unwrap();
        }
        Self {
            home,
            rot,
            herdr,
            claude,
            qpi,
        }
    }

    fn run(&self, args: &[&str]) -> (String, String, i32) {
        self.spawn(args, None)
    }

    fn run_with_receipt(&self, args: &[&str], src: &Path) -> (String, String, i32) {
        self.spawn(args, Some(src))
    }

    fn spawn(&self, args: &[&str], warden: Option<&Path>) -> (String, String, i32) {
        let argv = with_lin(args);
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_caddis"));
        cmd.args(&argv)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("CADDIS_HMAC_KEY", TEST_KEY)
            .env("CADDIS_DRAIN_HERDR", &self.herdr)
            .env("CADDIS_DRAIN_CLAUDE_REGISTRY", &self.claude)
            .env("CADDIS_DRAIN_QPI", &self.qpi);
        if let Some(src) = warden {
            cmd.env("CADDIS_WARDEN_RECEIPT", src);
        } else {
            cmd.env("CADDIS_SKIP_WARDEN", "1");
        }
        let out = cmd.output().expect("caddis must spawn");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    }

    fn arm(&self, kind: &str) {
        let (o, e, c) = self.run(&["rotate", "ready", "--kind", kind, "--model", "m1"]);
        assert_eq!(c, 0, "ready: {o}{e}");
        let (o, e, c) = self.run(&["rotate", "arm"]);
        assert_eq!(c, 0, "arm: {o}{e}");
    }

    fn session(&self) -> PathBuf {
        self.rot.join("lines").join("lin-t").join("session.receipt")
    }
}

fn assert_no_session(path: &Path) {
    assert!(
        !path.exists(),
        "session.receipt must not exist: {}",
        path.display()
    );
}

fn with_lin(args: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
    let sub = v.get(1).map(|s| s.as_str()).unwrap_or("");
    if matches!(sub, "ready" | "arm" | "verify") && !v.iter().any(|s| s == "--lineage") {
        v.push("--lineage".into());
        v.push("lin-t".into());
    }
    v
}

#[test]
fn omp_verify_writes_session_receipt() {
    let w = World::new("omp");
    w.arm("omp");
    let (o, e, c) = w.run(&["rotate", "verify"]);
    assert_eq!(c, 0, "verify: {o}{e}");
    let body = fs::read_to_string(w.session()).expect("session.receipt after omp verify");
    assert!(body.contains("kind=omp"), "{body}");
    assert!(body.contains("event=rotate-verify"), "{body}");
}

#[test]
fn non_omp_verify_does_not_write_session_receipt() {
    let w = World::new("qpi");
    w.arm("qpi");
    let (o, e, c) = w.run(&["rotate", "verify"]);
    assert_eq!(c, 0, "verify: {o}{e}");
    assert_no_session(&w.session());
}

#[test]
fn drain_fail_does_not_write_session_receipt() {
    let w = World::new("fail");
    w.arm("omp");
    fs::write(&w.herdr, r#"{"status": "live"}"#).unwrap();
    let (_o, _e, c) = w.run(&["rotate", "verify"]);
    assert_ne!(c, 0, "verify must fail on live herdr");
    assert!(w.rot.join("lines").join("lin-t").join("linger.lease").is_file());
    assert_no_session(&w.session());
}

#[test]
fn omp_verify_embeds_warden_receipt_fixture() {
    let w = World::new("warden");
    w.arm("omp");
    let src = w.home.join("warden-fixture.txt");
    fs::write(&src, "rows: 3\nfrom=omp\n").unwrap();
    let (o, e, c) = w.run_with_receipt(&["rotate", "verify"], &src);
    assert_eq!(c, 0, "verify: {o}{e}");
    let body = fs::read_to_string(w.session()).expect("session.receipt");
    assert!(body.contains("kind=omp"), "{body}");
    assert!(body.contains("event=rotate-verify"), "{body}");
    assert!(body.contains("rows: 3"), "warden body missing: {body}");
}
