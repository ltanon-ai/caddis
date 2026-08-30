//! occupancy.rs — CARD-0333 CLI. Occupied is exit 0.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn tmp(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let p = env::temp_dir().join(format!("caddis-occ-{tag}-{n}"));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

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
fn occupied_cli_exits_zero_with_fallback() {
    let dir = tmp("occ");
    let f = dir.join("occupancy");
    fs::write(&f, "mode=occupied\nstation=benchmark\n").unwrap();
    let path = f.to_str().unwrap();
    let (o, e, c) = run(&["occupancy", "--file", path]);
    assert_eq!(c, 0, "{o}{e}");
    assert!(o.contains("occupancy occupied"), "{o}");
    assert!(o.contains("station=benchmark"), "{o}");
    assert!(o.contains("coding=fallback"), "{o}");
    assert!(o.contains("droid-glm"), "{o}");
    assert!(o.contains("commandcode-deepseek"), "{o}");
    assert!(!o.contains("ollama"), "{o}");
}

#[test]
fn bee_cli_routes_station() {
    let dir = tmp("bee");
    let f = dir.join("occupancy");
    fs::write(&f, "mode=bee\n").unwrap();
    let path = f.to_str().unwrap();
    let (o, e, c) = run(&["occupancy", "--file", path]);
    assert_eq!(c, 0, "{o}{e}");
    assert!(o.contains("occupancy bee"), "{o}");
    assert!(o.contains("coding=station"), "{o}");
}

#[test]
fn missing_file_is_occupied_fail_safe() {
    let dir = tmp("miss");
    let f = dir.join("no-such-occupancy");
    let path = f.to_str().unwrap();
    let (o, e, c) = run(&["occupancy", "--file", path]);
    assert_eq!(c, 0, "{o}{e}");
    assert!(o.contains("occupancy occupied"), "{o}");
    assert!(o.contains("missing=1"), "{o}");
    assert!(o.contains("coding=fallback"), "{o}");
}

#[test]
fn malformed_exits_nonzero() {
    let dir = tmp("bad");
    let f = dir.join("occupancy");
    fs::write(&f, "mode=wedge\n").unwrap();
    let path = f.to_str().unwrap();
    let (o, e, c) = run(&["occupancy", "--file", path]);
    assert_ne!(c, 0, "{o}{e}");
}
