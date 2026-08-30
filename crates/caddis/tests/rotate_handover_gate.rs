//! rotate_handover_gate.rs — CARD-0301. Hermetic HOME, snapshot env.
//! The succession state organ: handover classifies (never gates), claims
//! are fenced by generation, linger dies on promote, legacy arms migrate.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);
const KEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

fn tmp(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("caddis-lease-{tag}-{n}"));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

struct World {
    home: PathBuf,
    snap: PathBuf,
    dir: PathBuf,
}

impl World {
    fn new(tag: &str) -> Self {
        let root = tmp(tag);
        let home = root.join("home");
        fs::create_dir_all(&home).unwrap();
        let snap = root.join("snapshot.json");
        fs::write(&snap, "{}").unwrap();
        let dir = home.join(".caddis/rotation/lines/lin-t");
        Self { home, snap, dir }
    }

    fn run(&self, args: &[&str], extra: &[(&str, &str)]) -> (String, String, i32) {
        let mut argv: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
        let sub = argv.get(1).map(|s| s.as_str()).unwrap_or("");
        if matches!(sub, "ready" | "arm" | "verify" | "handover")
            && !argv.iter().any(|s| s == "--lineage")
        {
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
            .env("CADDIS_HMAC_KEY", KEY)
            .env("CADDIS_HERDR_SNAPSHOT", &self.snap);
        for (k, v) in extra {
            cmd.env(k, v);
        }
        let out = cmd.output().expect("caddis must spawn");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    }

    fn arm(&self, pane: &str) {
        let (o, e, c) = self.run(
            &[
                "rotate", "ready", "--kind", "omp", "--model", "m1", "--pane", pane,
            ],
            &[],
        );
        assert_eq!(c, 0, "ready: {o}{e}");
        let (o, e, c) = self.run(&["rotate", "arm"], &[]);
        assert_eq!(c, 0, "arm: {o}{e}");
    }

    /// A predecessor's signed handover, written the way the organ will.
    fn handover(&self) -> (String, String, i32) {
        self.run(&["rotate", "handover"], &[])
    }

    fn verify(&self, claimer: &str) -> (String, String, i32) {
        self.run(&["rotate", "verify"], &[("HERDR_PANE_ID", claimer)])
    }

    fn read(&self, name: &str) -> Option<String> {
        fs::read_to_string(self.dir.join(name)).ok()
    }
}

/// Handover + absent pane -> promote with the clean classification.
#[test]
fn handover_and_absent_pane_promotes_clean() {
    let w = World::new("clean");
    w.arm("w3J:p1");
    let (o, e, c) = w.handover();
    assert_eq!(c, 0, "handover must write: {o}{e}");
    let (o, e, c) = w.verify("w3J:p2");
    assert_eq!(c, 0, "absent pane promotes: {o}{e}");
    assert!(o.contains("clean"), "must print clean classification: {o}");
    assert!(w.read("claimed.receipt").is_some(), "claim receipt written");
}

/// Absent pane, NO handover (legacy arm) -> promote + crash/escalate note.
#[test]
fn absent_pane_no_handover_promotes_with_crash_note() {
    let w = World::new("crash");
    w.arm("w3J:p1");
    let (o, e, c) = w.verify("w3J:p2");
    assert_eq!(c, 0, "legacy absence promotes (migration ramp): {o}{e}");
    assert!(
        o.contains("escalate") || o.contains("crash"),
        "must classify the ungraceful exit: {o}"
    );
}

/// A handover receipt NEVER gates: live pane + receipt still fails closed.
#[test]
fn handover_with_live_pane_still_fails_closed() {
    let w = World::new("forgery");
    w.arm("w3J:p1");
    let (o, e, c) = w.handover();
    assert_eq!(c, 0, "handover writes: {o}{e}");
    fs::write(
        &w.snap,
        r#"{"agents":[{"pane_id":"w3J:p1","agent_status":"idle"}]}"#,
    )
    .unwrap();
    let (o, e, c) = w.verify("w3J:p2");
    assert_ne!(c, 0, "forged/early handover must not promote: {o}{e}");
}

/// A successful promote clears a stale linger.lease.
#[test]
fn successful_promote_clears_linger() {
    let w = World::new("linger");
    w.arm("w3J:p1");
    // Seed a linger lease the way a previously failed verify does.
    fs::write(w.dir.join("linger.lease"), "live agent in pane w3J:p1\n").unwrap();
    let (o, e, c) = w.verify("w3J:p2");
    assert_eq!(c, 0, "promote: {o}{e}");
    assert!(w.read("linger.lease").is_none(), "linger must be cleared");
    assert!(o.contains("linger"), "must note the clear: {o}");
}

/// The claim is generation-fenced: claimed.gen exists and is a number.
#[test]
fn claim_writes_generation() {
    let w = World::new("gen");
    w.arm("w3J:p1");
    let (o, e, c) = w.verify("w3J:p2");
    assert_eq!(c, 0, "promote: {o}{e}");
    let gen = w.read("claimed.gen").expect("claimed.gen written");
    assert!(
        gen.trim().parse::<u64>().is_ok(),
        "gen must be numeric: {gen}"
    );
}
