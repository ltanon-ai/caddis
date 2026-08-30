//! version.rs tests — CARD-0227. Version must carry the build hash.

use std::process::{Command, Stdio};

fn run(args: &[&str]) -> (String, String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_caddis"))
        .args(args)
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

#[test]
fn version_prints_semver_and_hash() {
    let (o, e, c) = run(&["--version"]);
    assert_eq!(c, 0, "version: {o}{e}");
    let line = o.trim();
    assert!(
        line.starts_with("caddis ") && line.contains('-'),
        "version must be <semver>-<hash>: {line}"
    );
    let parts: Vec<&str> = line.split('-').collect();
    let hash = parts[1].trim();
    assert!(hash.len() >= 4, "hash too short: {hash}");
}
