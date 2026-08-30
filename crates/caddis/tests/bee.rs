//! bee.rs — CARD-0145. Hermetic. Never ~/.claude.
//!
//! A bee spawned without --harness must not run. An OMP spawn must stamp
//! CADDIS_HARNESS=omp into the child. Claude likewise.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

fn bin() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_caddis"));
    c.stdin(Stdio::null());
    c
}

fn python_env_printer() -> Vec<&'static str> {
    vec![
        "python",
        "-c",
        "import os; print('CADDIS_HARNESS='+os.environ.get('CADDIS_HARNESS','')); print('CADDIS_WARDEN_FROM='+os.environ.get('CADDIS_WARDEN_FROM',''))",
    ]
}

fn run(args: &[&str]) -> (String, String, i32) {
    let out = bin().args(args).output().expect("caddis must spawn");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

fn prepend_path(first: &Path) -> OsString {
    let mut out = first.as_os_str().to_os_string();
    if let Some(rest) = env::var_os("PATH") {
        out.push(if cfg!(windows) { ";" } else { ":" });
        out.push(rest);
    }
    out
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

fn run_with_nerve(args: &[&str]) -> (String, String, i32) {
    let bin_dir = env::temp_dir().join(format!(
        "caddis-bee-nerve-{}-{}",
        std::process::id(),
        args.len()
    ));
    install_warden(&bin_dir);
    let out = bin()
        .args(args)
        .env("PATH", prepend_path(&bin_dir))
        .output()
        .expect("caddis must spawn");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn spawn_without_harness_is_usage_and_does_not_run() {
    let mut args = vec!["bee", "spawn"];
    args.extend(python_env_printer());
    let (o, e, c) = run(&args);
    assert_eq!(c, 2, "usage: {o}{e}");
    assert!(!o.contains("CADDIS_HARNESS="), "child must not run: {o}");
}

#[test]
fn spawn_stamps_omp_into_child_env() {
    let mut args = vec!["bee", "spawn", "--harness", "omp", "--"];
    args.extend(python_env_printer());
    let (o, e, c) = run_with_nerve(&args);
    assert_eq!(c, 0, "omp spawn: {o}{e}");
    assert!(o.contains("CADDIS_HARNESS=omp"), "{o}");
    assert!(o.contains("CADDIS_WARDEN_FROM=omp"), "{o}");
}

#[test]
fn spawn_stamps_claude_into_child_env() {
    let mut args = vec!["bee", "spawn", "--harness", "claude", "--"];
    args.extend(python_env_printer());
    let (o, e, c) = run_with_nerve(&args);
    assert_eq!(c, 0, "claude spawn: {o}{e}");
    assert!(o.contains("CADDIS_HARNESS=claude"), "{o}");
    assert!(o.contains("CADDIS_WARDEN_FROM=claude"), "{o}");
}

#[test]
fn unknown_harness_is_usage() {
    let (o, e, c) = run(&[
        "bee",
        "spawn",
        "--harness",
        "gemini",
        "--",
        "python",
        "-c",
        "print(1)",
    ]);
    assert_eq!(c, 2, "unknown: {o}{e}");
    assert!(
        !o.contains("1\n") && !o.trim().eq("1"),
        "must not spawn: {o}"
    );
}

#[test]
fn spawn_without_warden_nerve_does_not_exec_payload() {
    let dir = env::temp_dir().join(format!("caddis-bee-no-nerve-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let marker = dir.join("must-not-exist");
    let _ = fs::remove_file(&marker);
    let script = format!("open(r'{}','w').write('no')", marker.display());
    let out = bin()
        .args([
            "bee",
            "spawn",
            "--harness",
            "omp",
            "--",
            "python",
            "-c",
            &script,
        ])
        .env("PATH", &dir)
        .output()
        .expect("caddis must spawn");
    let e = String::from_utf8_lossy(&out.stderr);
    let c = out.status.code().unwrap_or(-1);
    assert_ne!(c, 0, "spawn without nerve must fail: {e}");
    assert!(
        e.contains("CONSCIENCE OFFLINE") && e.contains("caddis-warden"),
        "fail closed on missing nerve, got: {e}"
    );
    assert!(
        !marker.exists(),
        "payload must not run when the nerve is missing"
    );
}
