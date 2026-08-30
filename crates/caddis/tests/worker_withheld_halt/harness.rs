//! harness.rs — CARD-0235 hermetic World (the withheld-halt suite's
//! fixture; split under the 280-line law). Same conventions as
//! tests/worker.rs: hermetic HOME, fake warden on PATH, never the
//! live ~/.caddis bag.

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
    let p = std::env::temp_dir().join(format!("caddis-wh-{}-{n}-{tag}", std::process::id()));
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

pub struct World {
    pub home: PathBuf,
    pub rot: PathBuf,
    pub root: PathBuf,
    herdr_fixture: PathBuf,
    warden_bin: PathBuf,
}

impl World {
    pub fn new(tag: &str) -> Self {
        let root = tmp(tag);
        let home = root.join("home");
        fs::create_dir_all(&home).unwrap();
        let rot = home.join(".caddis").join("rotation");
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
            rot,
            root,
            herdr_fixture,
            warden_bin,
        }
    }

    pub fn run(&self, args: &[&str]) -> (String, String, i32) {
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

    pub fn arm(&self) {
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

    pub fn queue(&self, body: &str) {
        let dir = self.line_dir();
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("queue"), body).unwrap();
    }

    pub fn line_dir(&self) -> PathBuf {
        self.rot.join("lines").join("line-a")
    }

    /// Idempotent bee payload: appends one line per dispatch, so the
    /// test can count EXACTLY how many dispatches burned.
    pub fn bee_script(&self) -> PathBuf {
        let script = self.root.join("bee.py");
        fs::write(
            &script,
            "import sys\nopen(sys.argv[1],'a').write('run\\n')\n",
        )
        .unwrap();
        script
    }

    pub fn tick(&self) -> (String, String, i32) {
        self.run(&["worker", "tick", "--lineage", "line-a"])
    }
}
