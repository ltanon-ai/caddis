//! rotate_drain.rs — drain/linger/kind tests split from rotate.rs (280 cap).

//! rotate.rs — CARD-0119 + CARD-0120, driven through the real binary.
//!
//! Every test uses its own HOME so the suite never touches the
//! operator's ~/.caddis. CADDIS_HMAC_KEY pins the key so tests skip
//! the OS CSPRNG entirely. Clean drain fixtures (no live agents) are
//! set for all kinds so existing verify tests stay green; tests that
//! need a live predecessor or an UNKNOWN source override them.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

/// 64 hex chars = 32 bytes, a valid CADDIS_HMAC_KEY for deterministic tests.
const TEST_KEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

fn tmp(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!("caddis-rotate-{}-{n}-{tag}", std::process::id()));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

struct World {
    home: PathBuf,
    rot: PathBuf,
    herdr_fixture: PathBuf,
    claude_fixture: PathBuf,
    qpi_fixture: PathBuf,
}

impl World {
    fn new(tag: &str) -> Self {
        let root = tmp(tag);
        let home = root.join("home");
        fs::create_dir_all(&home).unwrap();
        let rot = home.join(".caddis").join("rotation");
        // Clean drain fixtures (empty = no live agents) for all kinds.
        let herdr_fixture = root.join("herdr.json");
        let claude_fixture = root.join("claude-reg.json");
        let qpi_fixture = root.join("qpi.json");
        for f in [&herdr_fixture, &claude_fixture, &qpi_fixture] {
            fs::write(f, "").unwrap();
        }
        Self {
            home,
            rot,
            herdr_fixture,
            claude_fixture,
            qpi_fixture,
        }
    }

    fn run(&self, args: &[&str]) -> (String, String, i32) {
        self.run_with(args, true)
    }

    /// Run without drain env vars (production path → UNKNOWN for fake HOME).
    fn run_no_drain(&self, args: &[&str]) -> (String, String, i32) {
        self.run_with(args, false)
    }

    fn run_with(&self, args: &[&str], set_drain: bool) -> (String, String, i32) {
        let mut argv: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
        let sub = argv.get(1).map(|s| s.as_str()).unwrap_or("");
        if matches!(sub, "ready" | "arm" | "verify") && !argv.iter().any(|s| s == "--lineage") {
            argv.push("--lineage".into());
            argv.push("lin-t".into());
        }
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_caddis"));
        cmd.args(&argv)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("CADDIS_HMAC_KEY", TEST_KEY);
        if set_drain {
            cmd.env("CADDIS_DRAIN_HERDR", &self.herdr_fixture)
                .env("CADDIS_DRAIN_CLAUDE_REGISTRY", &self.claude_fixture)
                .env("CADDIS_DRAIN_QPI", &self.qpi_fixture);
        }
        let out = cmd.output().expect("caddis must spawn");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    }

    /// Overwrite the herdr fixture with a live-agent record.
    fn set_live_herdr(&self) {
        fs::write(&self.herdr_fixture, r#"{"status": "live"}"#).unwrap();
    }
}

#[test]
fn omp_live_herdr_empty_claude_registry_must_fail_drain() {
    let w = World::new("drain-omp");
    w.run(&["rotate", "ready", "--kind", "omp", "--model", "glm-5.2"]);
    w.run(&["rotate", "arm"]);
    // Herdr fixture has a live agent; Claude registry is empty (irrelevant
    // for omp — the drain must use herdr, never the Claude registry).
    w.set_live_herdr();
    let (stdout, stderr, code) = w.run(&["rotate", "verify"]);
    assert_ne!(
        code, 0,
        "verify must fail when omp drain finds a live herdr agent: {stdout}{stderr}"
    );
}

#[test]
fn force_cannot_override_unknown_source() {
    let w = World::new("force-unknown");
    w.run(&["rotate", "ready", "--kind", "omp", "--model", "glm-5.2"]);
    w.run(&["rotate", "arm"]);
    // No drain env vars set → production path → fake HOME has no .herdr
    // → UNKNOWN → non-zero, even with --force.
    let (stdout, stderr, code) = w.run_no_drain(&["rotate", "verify", "--force"]);
    assert_ne!(
        code, 0,
        "--force must not override UNKNOWN drain source: {stdout}{stderr}"
    );
}

#[test]
fn linger_lease_written_on_live_predecessor() {
    let w = World::new("linger");
    w.run(&["rotate", "ready", "--kind", "omp", "--model", "glm-5.2"]);
    w.run(&["rotate", "arm"]);
    w.set_live_herdr();
    let (_stdout, _stderr, code) = w.run(&["rotate", "verify"]);
    assert_ne!(code, 0, "verify must fail on live predecessor");
    let lease = w.rot.join("lines").join("lin-t").join("linger.lease");
    assert!(
        lease.is_file(),
        "linger.lease must be written on successor-fail"
    );
    let body = fs::read_to_string(&lease).unwrap();
    assert!(
        body.contains("reason="),
        "linger.lease must carry a reason: {body}"
    );
}

#[test]
fn drain_clean_when_no_live_agent() {
    let w = World::new("drain-clean");
    w.run(&["rotate", "ready", "--kind", "omp", "--model", "glm-5.2"]);
    w.run(&["rotate", "arm"]);
    // Herdr fixture is empty (no live agent) → drain Clean → verify passes.
    let (stdout, stderr, code) = w.run(&["rotate", "verify"]);
    assert_eq!(
        code, 0,
        "verify must pass when drain is clean: {stdout}{stderr}"
    );
}

#[test]
fn arm_receipt_kind_takes_precedence_over_flag() {
    let w = World::new("kind-flag");
    w.run(&["rotate", "ready", "--kind", "omp", "--model", "glm-5.2"]);
    w.run(&["rotate", "arm"]);
    // ARM receipt has kind=omp, so --kind claude is NOT used. The drain
    // uses omp (herdr fixture is clean) → passes.
    let (stdout, stderr, code) = w.run(&["rotate", "verify", "--kind", "claude"]);
    assert_eq!(
        code, 0,
        "ARM receipt kind takes precedence: {stdout}{stderr}"
    );
}
