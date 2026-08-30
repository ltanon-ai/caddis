//! eddy_epoch_row.rs — CARD-0242's second gate: on the first tick
//! whose page exceeds the run's previous max, the nerve appends ONE
//! `loop.epoch` envelope row to the eddy ledger — the rollover is an
//! event the operator can replay, never per-tick telemetry.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn tmp(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!("caddis-epoch-{tag}-{n}"));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

struct World {
    home: PathBuf,
}

impl World {
    fn new(tag: &str) -> Self {
        let home = tmp(tag).join("home");
        fs::create_dir_all(&home).unwrap();
        Self { home }
    }

    fn tick(&self, args: &[&str], stdin: &str) -> (String, String, i32) {
        let mut child = Command::new(env!("CARGO_BIN_EXE_caddis"))
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .spawn()
            .expect("spawn caddis");
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
        let out = child.wait_with_output().expect("caddis must finish");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    }

    fn ledger(&self) -> PathBuf {
        self.home.join(".caddis").join("eddy-ledger.jsonl")
    }
}

/// One epoch row per rollover (page 0 → 1 → 2 = TWO rows), each an
/// intact envelope with type loop.epoch naming from/to pages.
#[test]
fn each_rollover_writes_one_epoch_row() {
    let w = World::new("rolls");
    let args = ["eddy", "tick", "--run", "run-ep", "--until", "50"];
    let on_page = |p: u64| {
        format!(
            "{{\"payload\":\"p\",\"status_class\":\"ok\",\"outcome\":\"o{p}\",\"cache_read\":0,\"cache_write\":0,\"latency_ms\":1,\"page\":{p}}}"
        )
    };
    w.tick(&args, &on_page(0));
    w.tick(&args, &on_page(0));
    w.tick(&args, &on_page(1)); // rollover 0 -> 1
    w.tick(&args, &on_page(1));
    w.tick(&args, &on_page(2)); // rollover 1 -> 2
    w.tick(&args, &on_page(2));

    let ledger = fs::read_to_string(w.ledger()).expect("ledger exists");
    let epochs: Vec<&str> = ledger
        .lines()
        .filter(|l| l.contains("\"type\":\"loop.epoch\""))
        .collect();
    assert_eq!(epochs.len(), 2, "one row per rollover: {ledger}");
    assert!(
        epochs[0].contains("from_page=0") && epochs[0].contains("to_page=1"),
        "{ledger}"
    );
    assert!(
        epochs[1].contains("from_page=1") && epochs[1].contains("to_page=2"),
        "{ledger}"
    );
    for line in &epochs {
        assert!(
            caddis_core::ledger::is_intact_row(line),
            "intact envelope: {line}"
        );
    }
    // No per-tick spam: the ledger holds ONLY epoch rows for this run.
    assert_eq!(
        ledger.lines().count(),
        2,
        "no run row yet (no halt): {ledger}"
    );
}

/// No rollover, no epoch row — a flat run writes nothing to the ledger.
#[test]
fn flat_run_writes_no_epoch_rows() {
    let w = World::new("flat");
    let args = ["eddy", "tick", "--run", "run-flat", "--until", "50"];
    let body = "{\"payload\":\"p\",\"status_class\":\"ok\",\"outcome\":\"o\",\"cache_read\":0,\"cache_write\":0,\"latency_ms\":1}";
    for _ in 0..4 {
        w.tick(&args, body);
    }
    assert!(!w.ledger().exists(), "no rollover, no ledger");
}
