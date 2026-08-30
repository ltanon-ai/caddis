//! sentinel_cli.rs — CARD-0331. The audit organ in Rust: argument-
//! compatible with the bee's sentinel call, slot-compatible with the
//! push gate, fail-closed (a failed audit never destroys a prior
//! verdict). Hermetic: stub grok, stub sentinel home, temp repo + runs.

pub use std::fs;
use std::path::PathBuf;
pub use std::process::Command;
pub use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

static SEQ: AtomicU64 = AtomicU64::new(0);
/// CADDIS_SENTINEL_* env is process-global — serialize stub swaps.
pub static ENV_LOCK: Mutex<()> = Mutex::new(());
pub fn tmp(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("caddis-sent-{tag}-{n}"));
    let _ = fs::remove_dir_all(&p); // swallow: best-effort-cleanup — stale temp dir
    fs::create_dir_all(&p).unwrap();
    p
}

pub const SCHEMA: &str =
    "{\"type\":\"object\",\"required\":[\"verdict\",\"summary\",\"findings\",\"cannot_verify\"]}";

pub struct World {
    pub repo: PathBuf,
    pub home: PathBuf,
    runs: PathBuf,
    slot_name: String,
}

impl World {
    /// A committed fixture repo whose origin names the slug, plus a stub
    /// sentinel home. `engine` writes a grok-shaped .cmd answering with
    /// `envelope` on stdout.
    pub fn new(tag: &str, envelope: &str) -> Self {
        let root = tmp(tag);
        let repo = root.join("repo");
        fs::create_dir_all(&repo).unwrap();
        let g = |args: &[&str]| Command::new("git").args(args).current_dir(&repo).output();
        assert!(g(&["init", "-q"]).unwrap().status.success(), "git init");
        assert!(
            g(&[
                "remote",
                "add",
                "origin",
                "git@gitlab.com:varliukai/caddis-workshop.git"
            ])
            .unwrap()
            .status
            .success(),
            "git remote add"
        );
        fs::write(repo.join("lib.rs"), "pub fn a() {}\n").unwrap();
        assert!(g(&["add", "."]).unwrap().status.success(), "git add");
        let commit = Command::new("git")
            .args(["commit", "-qm", "fixture"])
            .current_dir(&repo)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .unwrap();
        assert!(commit.status.success(), "git commit");
        let home = root.join("sentinel-home");
        fs::create_dir_all(home.join("souls")).unwrap();
        fs::write(home.join("schema.json"), SCHEMA).unwrap();
        fs::write(home.join("souls").join("audit.md"), "audit soul stub\n").unwrap();
        #[cfg(windows)]
        let engine = {
            let bin = root.join("grok-stub.cmd");
            fs::write(&bin, format!("@echo off\r\necho {}\r\n", envelope)).unwrap();
            bin
        };
        #[cfg(not(windows))]
        let engine = {
            use std::os::unix::fs::PermissionsExt;
            let bin = root.join("grok-stub.sh");
            fs::write(&bin, format!("#!/bin/sh\ncat <<'EOF'\n{envelope}\nEOF\n")).unwrap();
            fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
            bin
        };
        let runs = root.join("runs");
        fs::create_dir_all(&runs).unwrap();
        std::env::set_var("CADDIS_SENTINEL_GROK_BIN", &engine);
        std::env::set_var("CADDIS_SENTINEL_HOME", &home);
        std::env::set_var("CADDIS_SENTINEL_RUNS", &runs);
        Self {
            repo,
            home,
            runs,
            slot_name: "last-verify-caddis-workshop.json".into(),
        }
    }

    pub fn run(&self, args: &[&str]) -> (String, String, i32) {
        let out = Command::new(env!("CARGO_BIN_EXE_caddis"))
            .args(args)
            .current_dir(&self.repo)
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .expect("caddis must spawn");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    }

    pub fn slot(&self) -> PathBuf {
        self.runs.join(&self.slot_name)
    }
}

pub const CLEAR_ENV: &str =
    "{\"type\":\"result\",\"structuredOutput\":{\"verdict\":\"CLEAR\",\"summary\":\"nothing owed\",\"findings\":[],\"cannot_verify\":[]}}";
pub const FINDINGS_ENV: &str =
    "{\"type\":\"result\",\"structuredOutput\":{\"verdict\":\"FINDINGS\",\"summary\":\"one owed\",\"findings\":[{\"title\":\"x\",\"evidence\":\"y\"}],\"cannot_verify\":[]}}";
