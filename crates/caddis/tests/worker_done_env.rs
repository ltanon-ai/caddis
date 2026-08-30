//! worker_done_env.rs — CARD-0260 + CARD-0261. Done-When gate with
//! bare PATH; Rust PATH scan + known locations, no cygpath.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);
const TEST_KEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

fn tmp(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!("caddis-dwenv-{}-{n}-{tag}", std::process::id()));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

/// PATH scan + known Windows locations — no subprocess spawn (CARD-0261).
fn which_dir(prog: &str) -> PathBuf {
    if let Some(p) = std::env::var_os("PATH") {
        for d in std::env::split_paths(&p) {
            let found = d.join(prog).is_file()
                || cfg!(windows)
                    && ["exe", "bat", "cmd"]
                        .iter()
                        .any(|e| d.join(format!("{prog}.{e}")).is_file());
            if found {
                return d;
            }
        }
    }
    known_dir(prog).unwrap_or_default()
}
/// CARD-0261: derive sh/python from env vars that survive the gate's stripped env.
#[cfg_attr(not(windows), allow(unused_variables))]
fn known_dir(prog: &str) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        if matches!(prog, "sh" | "bash") {
            let pf = std::env::var_os("ProgramFiles")
                .or_else(|| std::env::var_os("ProgramFiles(x86)"))
                .or_else(|| {
                    std::env::var_os("SystemDrive")
                        .map(|d| PathBuf::from(d).join("Program Files").into_os_string())
                })?;
            let d = PathBuf::from(pf).join("Git/bin");
            return d.join("sh.exe").is_file().then_some(d);
        }
        if matches!(prog, "python" | "python3") {
            let la = std::env::var_os("LOCALAPPDATA").or_else(|| {
                std::env::var_os("USERPROFILE")
                    .map(|u| PathBuf::from(u).join("AppData\\Local").into_os_string())
            })?;
            let base = PathBuf::from(la).join("Programs/Python");
            return fs::read_dir(&base)
                .ok()?
                .flatten()
                .find(|e| e.path().join("python.exe").is_file())
                .map(|e| e.path());
        }
    }
    None
}

/// warden + python + sh + system dirs; NOT the fake probe.
fn bare_path(warden_bin: &Path, py_dir: &Path, sh_dir: &Path) -> OsString {
    let sep = if cfg!(windows) { ";" } else { ":" };
    let mut out = warden_bin.as_os_str().to_os_string();
    for d in [py_dir, sh_dir] {
        if !d.as_os_str().is_empty() {
            out.push(sep);
            out.push(d);
        }
    }
    out.push(sep);
    out.push(if cfg!(windows) {
        "C:/Windows/system32;C:/Windows"
    } else {
        "/usr/bin:/bin"
    });
    out
}

/// `~/.profile` adds probe dir to PATH (both unix + Windows forms).
fn write_profile(home: &Path, probe_dir: &Path) {
    let unix = to_unix_path(probe_dir);
    let win = probe_dir.display().to_string();
    let p = format!("export PATH=\"{unix}:$PATH\"\nexport PATH=\"{win}:$PATH\"\n");
    fs::write(home.join(".profile"), &p).unwrap();
    fs::write(home.join(".bash_profile"), &p).unwrap();
}

/// Windows path -> MSYS unix-style in Rust (no cygpath).
fn to_unix_path(win: &Path) -> String {
    let s = win.display().to_string();
    let b = s.as_bytes();
    if b.len() >= 3 && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/') {
        format!(
            "/{}/{}",
            (b[0] as char).to_ascii_lowercase(),
            s[3..].replace('\\', "/")
        )
    } else {
        s
    }
}

struct World {
    home: PathBuf,
    root: PathBuf,
    herdr_fixture: PathBuf,
    warden_bin: PathBuf,
    py_dir: PathBuf,
    sh_dir: PathBuf,
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
            py_dir: which_dir("python"),
            sh_dir: which_dir("sh"),
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
            .env(
                "PATH",
                bare_path(&self.warden_bin, &self.py_dir, &self.sh_dir),
            );
        let out = cmd.output().expect("caddis must spawn");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    }

    fn arm(&self) {
        let a = [
            "rotate",
            "ready",
            "--kind",
            "omp",
            "--model",
            "m1",
            "--lineage",
            "line-a",
        ];
        let (o, e, c) = self.run(&a);
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
        self.home.join(".caddis/rotation/lines/line-a")
    }

    fn tick(&self) -> (String, String, i32) {
        self.run(&["worker", "tick", "--lineage", "line-a"])
    }

    /// Create a fake probe in `root/fakebin` discoverable only via
    /// the login shell's `~/.profile`. Returns the probe name.
    fn fake_probe(&self, name: &str) -> String {
        let dir = self.root.join("fakebin");
        fs::create_dir_all(&dir).unwrap();
        let probe = dir.join(name);
        #[cfg(windows)]
        fs::write(&probe, "#!/bin/sh\nexit 0\n").unwrap();
        #[cfg(not(windows))]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::write(&probe, "#!/bin/sh\nexit 0\n").unwrap();
            fs::set_permissions(&probe, fs::PermissionsExt::from_mode(0o755)).unwrap();
        }
        let _ = probe;
        write_profile(&self.home, &dir);
        name.to_string()
    }
}

/// argv0 NOT on bare PATH but resolvable via login shell `~/.profile`.
/// Shell fallback resolves the probe and the card EARNS done.
#[test]
fn shell_fallback_resolves_check_the_direct_spawn_cannot() {
    let w = World::new("fallback");
    w.arm();
    let probe = w.fake_probe("caddisprobe");
    let marker = w.root.join("marker.txt");
    let bee = w.root.join("bee.py");
    fs::write(&bee, "import sys\nopen(sys.argv[1],'a').write('run\\n')\n").unwrap();
    let card = w.root.join("_card_9601.md");
    fs::write(card, format!("# Done-When\n\n- $ {probe} -c pass\n")).unwrap();
    w.queue(&format!(
        "CARD-9601 python {} {}\n",
        bee.display(),
        marker.display()
    ));
    let (o, e, c) = w.tick();
    assert_eq!(c, 0, "tick: {o}{e}");
    assert!(o.contains("DW-OK"), "shell fallback must resolve: {o}{e}");
    let q = fs::read_to_string(w.line_dir().join("queue")).unwrap();
    assert!(q.contains("done CARD-9601"), "done earned: {q}");
    assert!(marker.exists(), "the bee ran");
}

/// Absent command withholds done; reason names the failing check.
#[test]
fn missing_command_withholds_with_the_check_named() {
    let w = World::new("missing");
    w.arm();
    let marker = w.root.join("marker.txt");
    let bee = w.root.join("bee.py");
    fs::write(&bee, "import sys\nopen(sys.argv[1],'a').write('run\\n')\n").unwrap();
    let card = w.root.join("_card_9602.md");
    fs::write(card, "# Done-When\n\n- $ caddis-no-such-tool-xyz -c pass\n").unwrap();
    w.queue(&format!(
        "CARD-9602 python {} {}\n",
        bee.display(),
        marker.display()
    ));
    let (o, e, c) = w.tick();
    assert_eq!(c, 0, "tick: {o}{e}");
    assert!(o.contains("DW-FAIL"), "absent command withholds: {o}{e}");
    assert!(
        o.contains("caddis-no-such-tool-xyz"),
        "reason names cmd: {o}{e}"
    );
    assert!(
        !o.contains("checks failed\n") && !o.contains("checks failed\r"),
        "no bare checks failed: {o}{e}"
    );
    let q = fs::read_to_string(w.line_dir().join("queue")).unwrap();
    assert!(!q.contains("done CARD-9602"), "absent cmd no done: {q}");
}
