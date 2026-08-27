//! ledger_rotate.rs — CARD-0130. Fake ledger only; never ~/.caddis.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn tmp(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("caddis-lrot-{tag}-{n}.jsonl"));
    let _ = fs::remove_file(&p);
    p
}

fn run(ledger: &PathBuf, args: &[&str]) -> (String, String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_caddis-warden"))
        .args(args)
        .env("CADDIS_WARDEN_LEDGER", ledger)
        .stdin(Stdio::null())
        .output()
        .expect("warden must spawn");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn rotate_archives_bytes_untouched() {
    let live = tmp("ok");
    let body = "{\"seq\":1}\n{\"seq\":2}\n";
    fs::write(&live, body).unwrap();
    let (o, e, c) = run(&live, &["ledger", "rotate"]);
    assert_eq!(c, 0, "rotate: {o}{e}");
    let live_now = fs::read_to_string(&live).unwrap();
    assert!(live_now.is_empty(), "live must be empty after rotate: {live_now:?}");
    let dir = live.parent().unwrap();
    let name = live.file_name().unwrap().to_string_lossy();
    let archive = fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .map(|n| n.to_string_lossy().starts_with(&*name) && *p != live)
                .unwrap_or(false)
        })
        .expect("archive sibling");
    let archived = fs::read_to_string(&archive).unwrap();
    assert_eq!(archived, body, "archive bytes must equal pre-rotate ledger");
    let _ = fs::remove_file(&archive);
    let _ = fs::remove_file(&live);
}

#[test]
fn rotate_missing_ledger_is_usage() {
    let live = tmp("missing");
    let (o, e, c) = run(&live, &["ledger", "rotate"]);
    assert_eq!(c, 2, "missing ledger: {o}{e}");
    assert!(e.contains("no ledger"), "must name the miss: {e}");
    assert!(!live.exists(), "must not create a live file on miss");
}
