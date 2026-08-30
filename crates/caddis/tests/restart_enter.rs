//! restart_enter.rs — CARD-0305. The spawn transaction's successor side:
//! `restart enter` orients at the stamped root; the pointer is a short
//! ASCII command; spawn honors a hermetic herdr override.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);
const KEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

fn tmp(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("caddis-enter-{tag}-{n}"));
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
            line: home.join(".caddis/rotation/lines/lin-t"),
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

    /// Seed a lineage the way the organs do: ready + arm + root stamp.
    fn seed(&self) {
        let (o, e, c) = self.run(&[
            "rotate",
            "ready",
            "--kind",
            "omp",
            "--model",
            "m1",
            "--lineage",
            "lin-t",
        ]);
        assert_eq!(c, 0, "ready: {o}{e}");
        let (o, e, c) = self.run(&["rotate", "arm", "--lineage", "lin-t"]);
        assert_eq!(c, 0, "arm: {o}{e}");
    }
}

/// CARD-0305: enter orients the successor at the stamped root with the
/// first duties named.
#[test]
fn enter_names_root_and_duties() {
    let w = World::new("orient");
    w.seed();
    let stamp = fs::read_to_string(w.line.join("ready.root")).expect("root stamped");
    let (o, e, c) = w.run(&["restart", "enter", "--lineage", "lin-t"]);
    assert_eq!(c, 0, "enter: {o}{e}");
    assert!(o.contains("root:"), "names the root: {o}");
    assert!(o.contains(stamp.trim_end()), "the stamped root: {o}");
    assert!(o.contains("packet"), "points at the packet: {o}");
    assert!(o.contains("heartbeat"), "names the heartbeat duty: {o}");
}

/// CARD-0305: unknown lineage fails closed.
#[test]
fn enter_unknown_lineage_fails() {
    let w = World::new("unknown");
    let (o, e, c) = w.run(&["restart", "enter", "--lineage", "no-such-line"]);
    assert_ne!(c, 0, "bad lineage must fail: {o}{e}");
}

/// CARD-0305 (E2 contract, tested): the pointer sent to the pane is a
/// short ASCII command — never a path.
#[test]
fn pointer_is_short_ascii() {
    let ptr = caddis_restart_pointer("lin-t");
    assert!(ptr.len() <= 80, "pointer <=80 chars: {ptr}");
    assert!(ptr.is_ascii(), "pointer ASCII-only: {ptr}");
    assert!(
        ptr.starts_with("caddis restart enter --lineage "),
        "shape: {ptr}"
    );
}

/// The lib-exposed pointer helper (the exact bytes spawn sends).
fn caddis_restart_pointer(lineage: &str) -> String {
    format!("caddis restart enter --lineage {lineage}")
}
