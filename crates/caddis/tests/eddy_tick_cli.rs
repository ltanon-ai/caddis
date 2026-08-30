//! eddy_tick_cli.rs — CARD-0233 RED-first. The nerve contract:
//! `caddis eddy tick` reads ONE tick as JSON on stdin, records it to
//! the host-owned JSONL, applies the ONE verdict, and FAILS CLOSED.
//!
//! Fail-closed INVERTS the warden's doctrine: the warden allows loudly
//! when its binary is unspawnable (one unjudged tool call is one
//! bounded action); the nerve refuses (an unjudged loop is an
//! unbounded 800ms re-fire). Malformed input, an unknown status class,
//! a missing bound — exit 2 with a DISABLE directive on stderr, never
//! a silent claim of enforcement.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn tmp(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("caddis-eddy-nerve-{tag}-{n}"));
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

    fn eddy_dir(&self) -> PathBuf {
        self.home.join(".caddis").join("eddy")
    }
}

const FAIL_TICK: &str =
    "{\"payload\":\"redo the report\",\"status_class\":\"fail\",\"outcome\":\"Error: provider 429\",\"cache_read\":408000,\"cache_write\":0,\"latency_ms\":812}";

const OK_TICK_A: &str =
    "{\"payload\":\"poll\",\"status_class\":\"ok\",\"outcome\":\"still building\",\"cache_read\":10,\"cache_write\":0,\"latency_ms\":900}";

#[test]
fn three_fail_ticks_halt_on_the_third_invocation() {
    let w = World::new("streak3");
    let args = ["eddy", "tick", "--run", "run-x", "--until", "50"];
    let (out1, _, rc1) = w.tick(&args, FAIL_TICK);
    assert_eq!(rc1, 0, "first fail continues: {out1}");
    assert!(out1.contains("\"verdict\":\"continue\""), "{out1}");
    let (out2, _, rc2) = w.tick(&args, FAIL_TICK);
    assert_eq!(rc2, 0, "second fail continues: {out2}");
    let (out3, err3, rc3) = w.tick(&args, FAIL_TICK);
    assert_eq!(rc3, 3, "third fail HALTS (exit 3): {out3} {err3}");
    assert!(out3.contains("\"verdict\":\"halt\""), "{out3}");
    // ONE blocker, exactly:
    let blockers = fs::read_to_string(w.eddy_dir().join("blockers.jsonl")).unwrap();
    assert_eq!(blockers.lines().count(), 1);
    assert!(blockers.contains("eddy:run-x"));
    // ONE RUN row in the caddis-core ledger — not one per tick:
    let ledger = fs::read_to_string(self_ledger(&w)).unwrap();
    assert_eq!(ledger.lines().count(), 1, "one row per RUN, not per tick");
    assert!(ledger.contains("run-x"));
}

#[test]
fn fatal_class_halts_on_the_first_tick() {
    let w = World::new("fatal1");
    let fatal = "{\"payload\":\"p\",\"status_class\":\"fatal.auth\",\"outcome\":\"403 5-hour usage limit\",\"cache_read\":0,\"cache_write\":0,\"latency_ms\":12,\"resume_after\":1787900000000}";
    let (out, _, rc) = w.tick(&["eddy", "tick", "--run", "k3", "--until", "50"], fatal);
    assert_eq!(rc, 3, "{out}");
    assert!(out.contains("fatal.auth"), "{out}");
    assert!(
        out.contains("1787900000000"),
        "resume-after surfaces: {out}"
    );
}

#[test]
fn malformed_input_fails_closed() {
    let w = World::new("garbage");
    let args = ["eddy", "tick", "--run", "run-g", "--until", "50"];
    let (out, err, rc) = w.tick(&args, "this is not json");
    assert_eq!(rc, 2, "fail-closed exit: {out} {err}");
    assert!(
        err.to_lowercase().contains("disable"),
        "the directive to disable governed loop mode must be on stderr: {err}"
    );
    // Nothing was recorded for an unjudgeable tick:
    assert!(!w.eddy_dir().join("run-g.jsonl").exists());
}

#[test]
fn unknown_status_class_is_refused_not_read_as_ok() {
    let w = World::new("badclass");
    let body = "{\"payload\":\"p\",\"status_class\":\"403 Forbidden\",\"outcome\":\"x\",\"cache_read\":0,\"cache_write\":0,\"latency_ms\":1}";
    let (out, err, rc) = w.tick(&["eddy", "tick", "--run", "run-b", "--until", "50"], body);
    assert_eq!(rc, 2, "{out} {err}");
    assert!(err.to_lowercase().contains("disable"));
}

#[test]
fn first_tick_without_a_bound_is_refused() {
    let w = World::new("unbounded");
    let (out, err, rc) = w.tick(&["eddy", "tick", "--run", "run-u"], FAIL_TICK);
    assert_eq!(rc, 2, "{out} {err}");
    assert!(err.contains("bound"), "refusal names the bound: {err}");
}

/// until-external + identical outcomes = Stagnant = WAITING: reported,
/// exit 0, the loop keeps its contract.
#[test]
fn stagnant_under_until_external_is_waiting_exit_zero() {
    let w = World::new("stagnant");
    let args = ["eddy", "tick", "--run", "run-s", "--until", "50"];
    let same = "{\"payload\":\"poll\",\"status_class\":\"ok\",\"outcome\":\"identical body\",\"cache_read\":0,\"cache_write\":0,\"latency_ms\":5}";
    let mut last = (String::new(), String::new(), 0);
    for _ in 0..3 {
        last = w.tick(&args, same);
    }
    assert_eq!(last.2, 0, "stagnant is waiting, not a halt: {:?}", last);
    assert!(last.0.contains("\"verdict\":\"stagnant\""), "{:?}", last);
    // And no RUN row: the run has not ended.
    assert!(!self_ledger(&w).exists());
}

/// The RUN row is a caddis-core intact envelope row.
#[test]
fn run_row_is_an_intact_envelope() {
    let w = World::new("runrow");
    let args = ["eddy", "tick", "--run", "run-r", "--until", "2"];
    w.tick(&args, OK_TICK_A);
    w.tick(&args, OK_TICK_A);
    let (_, _, rc) = w.tick(&args, OK_TICK_A);
    assert_eq!(rc, 3, "iteration bound 2 halts on the third tick");
    let ledger = fs::read_to_string(self_ledger(&w)).unwrap();
    let line = ledger.lines().next_back().unwrap();
    assert!(
        caddis_core::ledger::is_intact_row(line),
        "row must be intact: {line}"
    );
}

fn self_ledger(w: &World) -> PathBuf {
    w.home.join(".caddis").join("eddy-ledger.jsonl")
}

/// CARD-0237: `unprovable` is a wire status; three of them halt with
/// the UnprovableDone verdict (exit 3, same as any halt).
#[test]
fn unprovable_done_halts_at_three_on_the_wire() {
    let w = World::new("unprov");
    let args = ["eddy", "tick", "--run", "run-u3", "--until", "50"];
    let body = "{\"payload\":\"p\",\"status_class\":\"unprovable\",\"outcome\":\"no proof\",\"cache_read\":0,\"cache_write\":0,\"latency_ms\":1}";
    let mut last = (String::new(), String::new(), 0);
    for _ in 0..3 {
        last = w.tick(&args, body);
    }
    assert_eq!(last.2, 3, "{:?}", last);
    assert!(last.0.contains("unprovable done"), "{:?}", last);
}

/// CARD-0241: warm-then-cold prints the health line and files the
/// eddy-health blocker; the EXIT stays the verdict's own (0).
#[test]
fn cache_collapse_reports_health_without_halting() {
    let w = World::new("cachehealth");
    let args = ["eddy", "tick", "--run", "run-ch", "--until", "50"];
    let warm = "{\"payload\":\"p\",\"status_class\":\"ok\",\"outcome\":\"a\",\"cache_read\":400000,\"cache_write\":10,\"latency_ms\":1}";
    let cold = "{\"payload\":\"p\",\"status_class\":\"ok\",\"outcome\":\"z\",\"cache_read\":0,\"cache_write\":0,\"latency_ms\":1}";
    w.tick(&args, warm);
    w.tick(&args, warm);
    let mut last = (String::new(), String::new(), 0);
    for _ in 0..3 {
        last = w.tick(&args, cold);
    }
    assert_eq!(last.2, 0, "health never changes the exit: {:?}", last);
    assert!(last.0.contains("cache-cold-after-warm"), "{:?}", last);
    assert!(last.0.contains("last_warm_seq"), "{:?}", last);
    let blockers = fs::read_to_string(w.eddy_dir().join("blockers.jsonl")).unwrap();
    assert!(blockers.contains("eddy-health:run-ch"), "{blockers}");
}
