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
        self.spawn(args, None, None)
    }

    fn run_with_receipt(&self, args: &[&str], src: &Path) -> (String, String, i32) {
        self.spawn(args, Some(src), None)
    }

    fn run_pane(&self, args: &[&str], pane: &str) -> (String, String, i32) {
        self.spawn(args, None, Some(pane))
    }

    fn spawn(
        &self,
        args: &[&str],
        warden: Option<&Path>,
        pane: Option<&str>,
    ) -> (String, String, i32) {
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
            .env("CADDIS_DRAIN_QPI", &self.qpi)
            .env_remove("HERDR_PANE_ID");
        if let Some(p) = pane {
            cmd.env("HERDR_PANE_ID", p);
        }
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

    fn arm_pane(&self, kind: &str, pane: &str) {
        let (o, e, c) = self.run(&[
            "rotate", "ready", "--kind", kind, "--model", "m1", "--pane", pane,
        ]);
        assert_eq!(c, 0, "ready: {o}{e}");
        let (o, e, c) = self.run(&["rotate", "arm"]);
        assert_eq!(c, 0, "arm: {o}{e}");
    }

    fn arm_receipt(&self) -> String {
        fs::read_to_string(self.rot.join("lines").join("lin-t").join("arm.receipt")).unwrap()
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
    assert!(w
        .rot
        .join("lines")
        .join("lin-t")
        .join("linger.lease")
        .is_file());
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

/// CARD-0302: a clean verify CLAIMS the line — arm.receipt freezes at
/// arm time and the owner pane moves to claimed.receipt. The CARD-0150
/// restamp destroyed the armed identity and is retired.
#[test]
fn verify_claims_pane_to_successor() {
    let w = World::new("restamp");
    w.arm_pane("omp", "w3J:p3");
    let (o, e, c) = w.run_pane(&["rotate", "verify"], "w3J:p4");
    assert_eq!(c, 0, "verify: {o}{e}");
    let body = w.arm_receipt();
    assert!(
        body.contains("pane=w3J:p3"),
        "arm frozen at arm time: {body}"
    );
    assert!(o.contains("owner: pane=w3J:p4"), "owner named: {o}");
    let (o, e, c) = w.run(&["rotate", "verify"]);
    assert_eq!(c, 0, "second verify proves receipts stay valid: {o}{e}");
}

/// CARD-0150: no HERDR_PANE_ID, or the same pane, leaves the receipt alone.
#[test]
fn verify_without_pane_env_keeps_receipt() {
    let w = World::new("keep");
    w.arm_pane("omp", "w3J:p3");
    let (o, e, c) = w.run(&["rotate", "verify"]);
    assert_eq!(c, 0, "verify: {o}{e}");
    assert!(w.arm_receipt().contains("pane=w3J:p3"), "untouched");
    let (o, e, c) = w.run_pane(&["rotate", "verify"], "w3J:p3");
    assert_eq!(c, 0, "verify same pane: {o}{e}");
    assert!(
        w.arm_receipt().contains("pane=w3J:p3"),
        "same pane untouched"
    );
}

/// CARD-0151: the warn is spent at succession — a clean verify clears
/// fold.state, so the successor's first tick is quiet, not deny.
#[test]
fn verify_clears_fold_warn() {
    let w = World::new("foldclear");
    w.arm("omp");
    let tick = ["fold", "tick", "--lineage", "lin-t", "--used-pct", "99"];
    let (o, _e, c) = w.run(&tick);
    assert_eq!(c, 0, "warn: {o}");
    assert!(o.contains("FOLD warn"), "warn stdout: {o}");
    let state = w.rot.join("lines").join("lin-t").join("fold.state");
    assert!(state.is_file(), "warned state written");
    let (o, e, c) = w.run(&["rotate", "verify"]);
    assert_eq!(c, 0, "verify: {o}{e}");
    assert!(!state.exists(), "verify must clear fold.state");
    let (o, _e, c) = w.run(&tick);
    assert_eq!(c, 0, "post-succession tick warns fresh, not deny: {o}");
    assert!(o.contains("FOLD warn"), "{o}");
}

/// CARD-0151: a failed verify (drain live) does not spend the warn.
#[test]
fn failed_verify_keeps_fold_warn() {
    let w = World::new("foldkeep");
    w.arm("omp");
    let tick = ["fold", "tick", "--lineage", "lin-t", "--used-pct", "99"];
    let (_o, _e, c) = w.run(&tick);
    assert_eq!(c, 0);
    fs::write(&w.herdr, r#"{"status": "live"}"#).unwrap();
    let (_o, _e, c) = w.run(&["rotate", "verify"]);
    assert_ne!(c, 0, "verify must fail on live herdr");
    let state = w.rot.join("lines").join("lin-t").join("fold.state");
    assert!(state.is_file(), "warn survives failed verify");
}
