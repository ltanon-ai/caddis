//! beekeeper_verdict.rs — CARD-0237 RED-first. The beekeeper is a
//! third HOST of `eddy::verdict` — never a third counter.
//!
//! RED choreography (per the card): first assert the beekeeper side
//! owns a halt threshold of its own — it does, today: worker_done.rs
//! carries `withheld.state` + a threshold parameter, a whole second
//! counting machine next to the organ's streak law. That test is RED
//! against the finished card and is DELETED with the duplication.
//! The surviving gate: withheld dispatches become `eddy::Tick` values
//! (status `unprovable`), the halt comes from the ONE pure verdict,
//! and `UnprovableDone` is a Verdict variant, not a beekeeper special
//! case.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);
const TEST_KEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

fn tmp(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!("caddis-bv-{}-{n}-{tag}", std::process::id()));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

fn prepend_path(first: &Path) -> OsString {
    let mut out = first.as_os_str().to_os_string();
    if let Some(rest) = env::var_os("PATH") {
        out.push(if cfg!(windows) { ";" } else { ":" });
        out.push(rest);
    }
    out
}

struct World {
    home: PathBuf,
    root: PathBuf,
    herdr_fixture: PathBuf,
    warden_bin: PathBuf,
}

impl World {
    fn new(tag: &str) -> Self {
        let root = tmp(tag);
        let home = root.join("home");
        fs::create_dir_all(&home).unwrap();
        let herdr_fixture = root.join("herdr.json");
        fs::write(&herdr_fixture, "").unwrap();
        let warden_bin = root.join("bin");
        fs::create_dir_all(&warden_bin).unwrap();
        #[cfg(windows)]
        fs::write(
            warden_bin.join("caddis-warden.cmd"),
            "@echo off\r\nexit /b 0\r\n",
        )
        .unwrap();
        #[cfg(not(windows))]
        {
            use std::os::unix::fs::PermissionsExt;
            let p = warden_bin.join("caddis-warden");
            fs::write(&p, "#!/bin/sh\nexit 0\n").unwrap();
            fs::set_permissions(&p, fs::PermissionsExt::from_mode(0o755)).unwrap();
        }
        Self {
            home,
            root,
            herdr_fixture,
            warden_bin,
        }
    }

    fn run(&self, args: &[&str]) -> (String, String, i32) {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_caddis"));
        cmd.args(args)
            .current_dir(&self.root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("CADDIS_HMAC_KEY", TEST_KEY)
            .env("CADDIS_DRAIN_HERDR", &self.herdr_fixture)
            .env("PATH", prepend_path(&self.warden_bin));
        let out = cmd.output().expect("caddis must spawn");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    }

    fn arm(&self) {
        let (o, e, c) = self.run(&[
            "rotate",
            "ready",
            "--kind",
            "omp",
            "--model",
            "m1",
            "--lineage",
            "line-a",
        ]);
        assert_eq!(c, 0, "ready: {o}{e}");
        let (o, e, c) = self.run(&["rotate", "arm", "--lineage", "line-a"]);
        assert_eq!(c, 0, "arm: {o}{e}");
    }

    fn queue(&self, body: &str) {
        let dir = self.line_dir();
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("queue"), body).unwrap();
    }

    fn line_dir(&self) -> PathBuf {
        self.home
            .join(".caddis")
            .join("rotation")
            .join("lines")
            .join("line-a")
    }

    fn tick(&self) -> (String, String, i32) {
        self.run(&["worker", "tick", "--lineage", "line-a"])
    }
}

/// THE gate: three withheld dispatches become three `unprovable` ticks
/// in the lineage's host-owned eddy JSONL, and the halt is the ORGAN's
/// `Verdict::UnprovableDone` — no withheld.state anywhere.
#[test]
fn withheld_halt_flows_through_eddy_verdict() {
    let w = World::new("flow");
    w.arm();
    let marker = w.root.join("marker.txt");
    let bee = w.root.join("bee.py");
    fs::write(&bee, "import sys\nopen(sys.argv[1],'a').write('run\\n')\n").unwrap();
    fs::write(
        w.root.join("_card_9401.md"),
        "# Withheld\n\n# Done-When\n\n- $ python -c \"import sys;sys.exit(1)\"\n",
    )
    .unwrap();
    w.queue(&format!(
        "CARD-9401 python {} {}\n",
        bee.display(),
        marker.display()
    ));

    let n = caddis_organs::watchdog::DEFAULT_MAX_FAILURES;
    for _ in 0..n {
        let (o, e, c) = w.tick();
        assert_eq!(c, 0, "{o}{e}");
    }
    // The tick trail the organ judged:
    let trail = fs::read_to_string(w.line_dir().join("eddy.jsonl"))
        .expect("withheld dispatches land as eddy ticks");
    let unprovables = trail
        .lines()
        .filter(|l| l.contains("\"status_class\":\"unprovable\""))
        .count();
    assert_eq!(
        unprovables as u32, n,
        "one unprovable tick per withheld dispatch: {trail}"
    );
    // The organ's verdict variant drove the halt:
    let (o, _, _) = w.tick_no_card_left();
    let _ = o;
    // (the halt printed on dispatch n; the queue is out of rotation:)
    let q = fs::read_to_string(w.line_dir().join("queue")).unwrap();
    assert!(
        q.trim_start().starts_with("withheld CARD-9401"),
        "halted by the organ: {q}"
    );
    // NO local counter survives:
    assert!(
        !w.line_dir().join("withheld.state").exists(),
        "the second counting machine is deleted; the tick trail is the state"
    );
}

/// The organ law itself: `unprovable` is a wire status, its streak is
/// its OWN (a provider Fail does not feed it and vice versa), and
/// `UnprovableDone` is a Verdict variant.
#[test]
fn organ_law_unprovable_done_is_a_verdict_variant() {
    use caddis_organs::eddy::{verdict, StatusClass, Tick, Verdict};
    let t = |status: StatusClass, outcome: u64| Tick {
        run_id: "line-a".into(),
        seq: 1,
        payload_hash: 5,
        status_class: status,
        outcome_hash: outcome,
        cache_read: 0,
        cache_write: 0,
        latency_ms: 0,
        ts_ms: 0,
        resume_after: None,
        artifact_hash: 0,
        page: 0,
    };
    let n = caddis_organs::watchdog::DEFAULT_MAX_FAILURES as usize;
    let mut ticks = vec![t(StatusClass::Ok, 0)];
    for i in 0..n {
        ticks.push(t(
            StatusClass::parse_wire("unprovable").expect("wire status"),
            100 + i as u64,
        ));
    }
    match verdict(&ticks) {
        Verdict::UnprovableDone { streak } => assert_eq!(streak as usize, n),
        other => panic!("expected UnprovableDone, got {other:?}"),
    }
    // Mixed: a provider Fail breaks the unprovable streak (different
    // failure mode, different law) and two unprovables do not halt:
    let mixed = vec![
        t(StatusClass::Unprovable, 1),
        t(StatusClass::Unprovable, 2),
        t(StatusClass::Fail, 3),
        t(StatusClass::Unprovable, 4),
        t(StatusClass::Unprovable, 5),
    ];
    assert!(matches!(verdict(&mixed), Verdict::Continue));
}

impl World {
    fn tick_no_card_left(&self) -> (String, String, i32) {
        self.run(&["worker", "tick", "--lineage", "line-a"])
    }
}
