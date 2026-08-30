//! fault_matrix_lease.rs — CARD-0319. The lease row of the quorum-S8
//! fault matrix, split from fault_matrix.rs at the 280-line law: a
//! torn claimed.gen under an existing claim never re-fences the line
//! from zero. Hermetic HOME.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);
const KEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

fn tmp(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("caddis-fml-{tag}-{n}"));
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
            line: home.join(".caddis/rotation/lines/lin-f"),
            home,
        }
    }

    fn run(&self, args: &[&str], envs: &[(&str, &str)]) -> (String, String, i32) {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_caddis"));
        cmd.args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("CADDIS_HMAC_KEY", KEY)
            .env("CADDIS_SKIP_WARDEN", "1")
            .env_remove("HERDR_PANE_ID")
            .env_remove("CADDIS_HERDR_BIN");
        for (k, v) in envs {
            cmd.env(k, v);
        }
        let out = cmd.output().expect("caddis must spawn");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    }

    fn ready(&self, envs: &[(&str, &str)]) {
        let (o, e, c) = self.run(
            &[
                "rotate",
                "ready",
                "--kind",
                "omp",
                "--model",
                "m1",
                "--pane",
                "wF:p1",
                "--lineage",
                "lin-f",
            ],
            envs,
        );
        assert_eq!(c, 0, "ready: {o}{e}");
        let (o, e, c) = self.run(&["rotate", "arm", "--lineage", "lin-f"], envs);
        assert_eq!(c, 0, "arm: {o}{e}");
    }
}

/// A torn claimed.gen under an existing claim: the next claim REFUSES
/// and never re-fences from zero. Drain snapshot `{}` = predecessor
/// pane absent, so verify may promote (handover-gate harness pattern).
#[test]
fn torn_claimed_gen_never_refences() {
    let w = World::new("torngen");
    let snap = w.home.parent().unwrap().join("snapshot.json");
    fs::write(&snap, "{}").unwrap();
    w.ready(&[]);
    let (o, e, c) = w.run(
        &["rotate", "verify", "--lineage", "lin-f"],
        &[
            ("HERDR_PANE_ID", "wF:p2"),
            ("CADDIS_HERDR_SNAPSHOT", snap.to_str().unwrap()),
        ],
    );
    assert_eq!(c, 0, "first verify claims gen=1: {o}{e}");
    assert_eq!(
        fs::read_to_string(w.line.join("claimed.gen"))
            .unwrap()
            .trim(),
        "1"
    );
    // spend the reservation the CARD-0304/0308 way, then tear the gen
    w.ready(&[]);
    fs::write(w.line.join("claimed.gen"), "1\nge").unwrap();
    let (o, e, c) = w.run(
        &["rotate", "verify", "--lineage", "lin-f"],
        &[
            ("HERDR_PANE_ID", "wF:p3"),
            ("CADDIS_HERDR_SNAPSHOT", snap.to_str().unwrap()),
        ],
    );
    assert_ne!(c, 0, "torn gen must refuse the re-claim: {o}{e}");
    assert!(
        format!("{o}{e}").contains("claimed.gen"),
        "names the torn file: {o}{e}"
    );
    assert_eq!(
        fs::read_to_string(w.line.join("claimed.gen")).unwrap(),
        "1\nge",
        "no silent rewrite"
    );
}
