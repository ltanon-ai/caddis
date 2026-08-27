//! bee.rs — CARD-0145. Hermetic. Never ~/.claude.
//!
//! A bee spawned without --harness must not run. An OMP spawn must stamp
//! CADDIS_HARNESS=omp into the child. Claude likewise.

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
    let out = bin()
        .args(args)
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
    assert!(
        !o.contains("CADDIS_HARNESS="),
        "child must not run: {o}"
    );
}

#[test]
fn spawn_stamps_omp_into_child_env() {
    let mut args = vec!["bee", "spawn", "--harness", "omp", "--"];
    args.extend(python_env_printer());
    let (o, e, c) = run(&args);
    assert_eq!(c, 0, "omp spawn: {o}{e}");
    assert!(o.contains("CADDIS_HARNESS=omp"), "{o}");
    assert!(o.contains("CADDIS_WARDEN_FROM=omp"), "{o}");
}

#[test]
fn spawn_stamps_claude_into_child_env() {
    let mut args = vec!["bee", "spawn", "--harness", "claude", "--"];
    args.extend(python_env_printer());
    let (o, e, c) = run(&args);
    assert_eq!(c, 0, "claude spawn: {o}{e}");
    assert!(o.contains("CADDIS_HARNESS=claude"), "{o}");
    assert!(o.contains("CADDIS_WARDEN_FROM=claude"), "{o}");
}

#[test]
fn unknown_harness_is_usage() {
    let (o, e, c) = run(&["bee", "spawn", "--harness", "gemini", "--", "python", "-c", "print(1)"]);
    assert_eq!(c, 2, "unknown: {o}{e}");
    assert!(!o.contains("1\n") && !o.trim().eq("1"), "must not spawn: {o}");
}
