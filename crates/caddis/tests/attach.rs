//! attach.rs — CARD-0118 / join CARD-0, driven through the real binary.
//!
//! Every test uses its own HOME + PATH so the suite never touches the
//! operator's ~/.omp, ~/.claude, or a real caddis-warden on PATH.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn tmp(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!("caddis-attach-{}-{n}-{tag}", std::process::id()));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

fn write_skill_src(root: &Path) -> PathBuf {
    let src = root.join("skill-src").join("caddis");
    fs::create_dir_all(&src).unwrap();
    fs::write(src.join("SKILL.md"), "projected skill\n").unwrap();
    src
}

fn write_bee_src(root: &Path) -> PathBuf {
    let p = root.join("droid-bees-always.md");
    fs::write(&p, "bee lanes: droid-first\n").unwrap();
    p
}

fn prepend_path(first: &Path) -> std::ffi::OsString {
    let mut dirs = vec![first.to_path_buf()];
    if let Some(orig) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&orig));
    }
    std::env::join_paths(dirs).unwrap_or_else(|_| first.as_os_str().to_os_string())
}

fn install_warden(bin: &Path) {
    fs::create_dir_all(bin).unwrap();
    #[cfg(windows)]
    fs::write(bin.join("caddis-warden.cmd"), "@echo off\r\nexit /b 0\r\n").unwrap();
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;
        let p = bin.join("caddis-warden");
        fs::write(&p, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

fn install_voice(bin: &Path) -> PathBuf {
    fs::create_dir_all(bin).unwrap();
    let p = bin.join("voice_stub.py");
    fs::write(
        &p,
        "import os, sys\nopen(os.environ['CADDIS_VOICE_LOG'], 'w', encoding='utf-8').write(' '.join(sys.argv[1:]) + '\\n')\n",
    )
    .unwrap();
    p
}

struct World {
    home: PathBuf,
    path: PathBuf,
    skill_src: PathBuf,
    bee_src: PathBuf,
    voice_bin: PathBuf,
    voice_log: PathBuf,
    fold_ext: PathBuf,
    warden_installed: bool,
}

impl World {
    fn new(tag: &str, with_warden: bool) -> Self {
        let root = tmp(tag);
        let home = root.join("home");
        let path = root.join("bin");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&path).unwrap();
        if with_warden {
            install_warden(&path);
        }
        let voice_bin = install_voice(&path);
        let voice_log = root.join("voice.log");
        let fold_ext = root.join("caddis-fold.ts");
        fs::write(&fold_ext, "caddis fold tick --lineage\nblock: true\n").unwrap();
        Self {
            home,
            path,
            skill_src: write_skill_src(&root),
            bee_src: write_bee_src(&root),
            voice_bin,
            voice_log,
            fold_ext,
            warden_installed: with_warden,
        }
    }

    fn dest(&self) -> PathBuf {
        self.home
            .join(".omp")
            .join("agent")
            .join("skills")
            .join("caddis")
    }

    fn run(&self, args: &[&str]) -> (String, String, i32) {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_caddis"));
        cmd.args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("CADDIS_VOICE_BIN", &self.voice_bin)
            .env("CADDIS_VOICE_LOG", &self.voice_log)
            .env("CADDIS_BEE_SRC", &self.bee_src)
            .env("CADDIS_FOLD_EXT", &self.fold_ext)
            .env("PYTHONUTF8", "1");
        if self.warden_installed {
            cmd.env("PATH", prepend_path(&self.path));
        } else {
            cmd.env("PATH", self.path.join("empty-missing"));
        }
        let out = cmd.output().expect("caddis must spawn");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    }
}

#[test]
fn warden_missing_is_conscience_offline() {
    let w = World::new("offline", false);
    let dest = w.dest();
    fs::create_dir_all(&dest).unwrap();
    fs::write(dest.join("marker"), "keep\n").unwrap();

    let (stdout, stderr, code) = w.run(&[
        "attach",
        "--harness",
        "omp-peleda",
        "--skill-src",
        w.skill_src.to_str().unwrap(),
    ]);

    assert_ne!(code, 0, "missing warden must fail: {stdout}{stderr}");
    assert!(
        stderr.contains("CONSCIENCE OFFLINE"),
        "stderr must name the conscience: {stderr}"
    );
    assert_eq!(
        fs::read_to_string(dest.join("marker")).unwrap(),
        "keep\n",
        "a missing warden must not mutate the dest"
    );
    assert!(
        !dest.join("SKILL.md").exists(),
        "must not project skills while the conscience is offline"
    );
}

#[test]
fn attach_restores_wiped_omp_projection_and_registers_voice() {
    let w = World::new("restore", true);
    let dest = w.dest();
    fs::create_dir_all(&dest).unwrap();
    fs::write(dest.join("SKILL.md"), "stale\n").unwrap();
    fs::remove_dir_all(&dest).unwrap();
    assert!(!dest.exists(), "the wipe is the pretest");

    let (stdout, stderr, code) = w.run(&[
        "attach",
        "--harness",
        "omp-peleda",
        "--skill-src",
        w.skill_src.to_str().unwrap(),
    ]);

    assert_eq!(code, 0, "attach must succeed: {stdout}{stderr}");
    assert_eq!(
        fs::read_to_string(dest.join("SKILL.md")).unwrap(),
        "projected skill\n"
    );
    let voice = fs::read_to_string(&w.voice_log).unwrap_or_default();
    assert!(
        voice.contains("register") && voice.contains("OMP Pelėda"),
        "voice register must be OMP Pelėda: {voice}"
    );
    assert!(
        dest.join("droid-bees-always.md").is_file(),
        "bee inherit must land in the projected skill dir"
    );
}

#[test]
fn unknown_harness_is_a_usage_error() {
    let w = World::new("bad-harness", true);
    let (_stdout, stderr, code) = w.run(&[
        "attach",
        "--harness",
        "not-a-chair",
        "--skill-src",
        w.skill_src.to_str().unwrap(),
    ]);
    assert_eq!(code, 2, "unknown harness is usage, not a hang: {stderr}");
}

#[test]
fn attach_omp_copies_fold_extension() {
    let w = World::new("fold-ext", true);
    let (stdout, stderr, code) = w.run(&[
        "attach",
        "--harness",
        "omp-peleda",
        "--skill-src",
        w.skill_src.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "attach must succeed: {stdout}{stderr}");
    let ext = w
        .home
        .join(".omp")
        .join("agent")
        .join("extensions")
        .join("caddis-fold.ts");
    assert!(
        ext.is_file(),
        "fold extension must be copied: {}",
        ext.display()
    );
    let body = fs::read_to_string(&ext).unwrap();
    assert!(
        body.contains("caddis fold tick") && body.contains("block"),
        "extension must tick fold and block on deny: {body}"
    );
}
