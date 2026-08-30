//! prove_consume.rs — CARD-0317. The done-gate eats prove-receipts: a
//! check the gate env cannot pass (here: the PROVE_OK marker only the
//! host env carries — the E5 PATH-hole shape) is covered ONLY by a
//! host-minted receipt (mac verifies, exit 0, cmd match). Tampered
//! mac or mismatched cmd stays withheld. Hermetic HOME.

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
    let p = std::env::temp_dir().join(format!("caddis-pc-{}-{n}-{tag}", std::process::id()));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

fn prepend_path(first: &Path) -> OsString {
    let mut out = first.as_os_str().to_os_string();
    if let Some(rest) = env::var_os("PATH") {
        out.push(";");
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
            fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
        }
        Self {
            home,
            root,
            herdr_fixture,
            warden_bin,
        }
    }

    fn run(&self, args: &[&str]) -> (String, String, i32) {
        self.run_env(args, &[])
    }

    /// The host-env variant: extra env (PROVE_OK) is exactly what the
    /// gate env lacks — the receipt's reason to exist.
    fn run_env(&self, args: &[&str], extra: &[(&str, &str)]) -> (String, String, i32) {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_caddis"));
        cmd.args(args)
            .current_dir(&self.root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("CADDIS_HMAC_KEY", TEST_KEY)
            .env("PATH", prepend_path(&self.warden_bin))
            .env("CADDIS_DRAIN_HERDR", &self.herdr_fixture)
            .env_remove("PROVE_OK");
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

    /// Gate-env-failing check: exits 9 without PROVE_OK, 0 with it.
    fn seed_check(&self) {
        fs::write(
            self.root.join("prove-x.py"),
            "import os, sys\nsys.exit(0 if os.environ.get('PROVE_OK') else 9)\n",
        )
        .unwrap();
        fs::write(
            self.root.join("_card_0317.md"),
            "# Done-When\n- $ python prove-x.py\n",
        )
        .unwrap();
        self.queue("CARD-0317 python -c pass\n");
    }

    fn mint(&self) {
        let (o, e, c) = self.run_env(
            &["prove", "--lineage", "line-a", "--", "python", "prove-x.py"],
            &[("PROVE_OK", "1")],
        );
        assert_eq!(c, 0, "host mint: {o}{e}");
    }
}

#[test]
fn withheld_check_passes_by_valid_receipt() {
    let w = World::new("cover");
    w.arm();
    w.seed_check();
    w.mint();
    let (o, e, c) = w.run(&["worker", "tick", "--lineage", "line-a"]);
    assert_eq!(c, 0, "tick: {o}{e}");
    assert!(
        o.contains("DW-OK 1/1 (1 by prove-receipt)"),
        "receipt covers the gate-impossible check: {o}{e}"
    );
    let q = fs::read_to_string(w.line_dir().join("queue")).unwrap();
    assert!(q.contains("done CARD-0317"), "marked done: {q}");
}

#[test]
fn tampered_receipt_still_withholds() {
    let w = World::new("tamper");
    w.arm();
    w.seed_check();
    w.mint();
    let receipts = w.line_dir().join("prove.jsonl");
    let raw = fs::read_to_string(&receipts).unwrap();
    fs::write(&receipts, raw.replace("\"exit\":0", "\"exit\":7")).unwrap();
    let (o, e, c) = w.run(&["worker", "tick", "--lineage", "line-a"]);
    assert_eq!(c, 0, "bee itself passed: {o}{e}");
    assert!(o.contains("DW-FAIL 0/1"), "stale mac never covers: {o}{e}");
    let q = fs::read_to_string(w.line_dir().join("queue")).unwrap();
    assert!(!q.contains("done CARD-0317"), "withheld: {q}");
}

#[test]
fn green_card_prints_no_receipt_label() {
    let w = World::new("plain");
    w.arm();
    fs::write(
        w.root.join("_card_0317.md"),
        "# Done-When\n- $ python -c pass\n- $ python -c pass\n",
    )
    .unwrap();
    w.queue("CARD-0317 python -c pass\n");
    let (o, e, c) = w.run(&["worker", "tick", "--lineage", "line-a"]);
    assert_eq!(c, 0, "tick: {o}{e}");
    assert!(o.contains("DW-OK 2/2"), "plain green: {o}{e}");
    assert!(
        !o.contains("prove-receipt"),
        "no label when none used: {o}{e}"
    );
}
