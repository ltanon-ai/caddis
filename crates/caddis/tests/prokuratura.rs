//! prokuratura.rs — CARD-0252 RED-first. The operator's single interface:
//! `caddis brief`, `caddis fix <symptom>`, `caddis build "<idea>"`.
//! Today none exist; `caddis brief` is a usage error (exit 2).

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn tmp(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("caddis-prokuratura-{tag}-{n}"));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

struct World {
    home: PathBuf,
}

impl World {
    fn new(tag: &str) -> Self {
        let home = tmp(tag).join("home");
        fs::create_dir_all(&home).unwrap();
        Self { home }
    }

    fn caddis(&self, args: &[&str]) -> (String, String, i32) {
        let out = Command::new(env!("CARGO_BIN_EXE_caddis"))
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .output()
            .expect("spawn caddis");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    }
}

/// RED: `caddis brief` exits 0 and prints a state report containing
/// the required structure markers. Today it is exit 2 (usage error).
#[test]
fn brief_reports_state() {
    let w = World::new("brief");
    let (stdout, _stderr, code) = w.caddis(&["brief"]);
    assert_eq!(
        code, 0,
        "caddis brief must exit 0; got exit {code}\nstdout: {stdout}"
    );
    assert!(
        stdout.contains("cards done") || stdout.contains("done"),
        "brief must report cards done: {stdout}"
    );
    assert!(
        stdout.contains("queued") || stdout.contains("queued:"),
        "brief must report queued cards: {stdout}"
    );
    assert!(
        stdout.contains("scan") || stdout.contains("green") || stdout.contains("red"),
        "brief must report scan status: {stdout}"
    );
}

/// RED: `caddis brief --voice` is accepted (exit 0), not a usage error.
#[test]
fn brief_voice_flag_accepted() {
    let w = World::new("brief-voice");
    let (_stdout, _stderr, code) = w.caddis(&["brief", "--voice"]);
    assert_eq!(code, 0, "caddis brief --voice must exit 0; got exit {code}");
}

/// RED: `caddis fix <symptom>` runs the diagnostic cascade and exits 0.
/// Today it is exit 2 (usage error — no subcommand).
#[test]
fn fix_runs_diagnostic_cascade() {
    let w = World::new("fix");
    let (stdout, _stderr, code) = w.caddis(&["fix", "stuck-card"]);
    assert_eq!(
        code, 0,
        "caddis fix must exit 0; got exit {code}\nstdout: {stdout}"
    );
    assert!(
        stdout.contains("HUMAN") || stdout.contains("OK") || stdout.contains("FIX"),
        "fix must report a verdict (OK/FIX/HUMAN): {stdout}"
    );
}

/// RED: `caddis fix` with no symptom is a usage error (exit 2).
#[test]
fn fix_without_symptom_is_usage_error() {
    let w = World::new("fix-noarg");
    let (_stdout, _stderr, code) = w.caddis(&["fix"]);
    assert_eq!(
        code, 2,
        "caddis fix with no symptom must exit 2; got exit {code}"
    );
}

/// RED: `caddis build "<idea>"` accepts an idea and exits 0.
/// Today it is exit 2 (usage error — no subcommand).
#[test]
fn build_accepts_idea() {
    let w = World::new("build");
    let (stdout, _stderr, code) = w.caddis(&["build", "add a scan summary command"]);
    assert_eq!(
        code, 0,
        "caddis build must exit 0; got exit {code}\nstdout: {stdout}"
    );
    assert!(
        stdout.contains("queued") || stdout.contains("card"),
        "build must report queued cards: {stdout}"
    );
}

/// RED: `caddis build` with no idea is a usage error (exit 2).
#[test]
fn build_without_idea_is_usage_error() {
    let w = World::new("build-noarg");
    let (_stdout, _stderr, code) = w.caddis(&["build"]);
    assert_eq!(
        code, 2,
        "caddis build with no idea must exit 2; got exit {code}"
    );
}
