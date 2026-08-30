//! eddy_record.rs — CARD-0228 RED-first. The first eddy card writes NO
//! law: it only records ticks. These tests pin exactly that.
//!
//! - a tick round-trips through the HOST-OWNED JSONL (blocker.rs
//!   precedent) with every field intact;
//! - the payload hash is STABLE ACROSS BUILDS — pinned to the published
//!   FNV-1a vectors, so a DefaultHasher (per-build random) can never
//!   sneak in;
//! - 589 ticks — the measured 2026-08-28 burn volume — land in ONE
//!   JSONL file and nowhere else: the TCB ledger must stay unloaded;
//! - recording Fail ticks files NO blocker and halts nothing: the halt
//!   law arrives with CARD-0229, not here.

use std::fs;
use std::path::PathBuf;

use caddis_organs::eddy::{read_ticks, record_tick, stable_hash, StatusClass, Tick};

fn tmp(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("caddis-eddy-record-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

fn tick(seq: u64, status: StatusClass) -> Tick {
    Tick {
        run_id: "run-77".into(),
        seq,
        payload_hash: stable_hash("redo the incident report"),
        status_class: status,
        outcome_hash: stable_hash(&format!("outcome {seq}")),
        cache_read: 408_000,
        cache_write: 0,
        latency_ms: 800 + seq,
        ts_ms: 1_700_000_000_000 + seq,
        resume_after: None,
        artifact_hash: 0,
        page: 0,
    }
}

#[test]
fn tick_roundtrips_through_host_jsonl() {
    let dir = tmp("roundtrip");
    let path = dir.join("run-77.jsonl");
    for (i, t) in [
        tick(1, StatusClass::Ok),
        tick(2, StatusClass::Ok),
        tick(3, StatusClass::Fail),
    ]
    .into_iter()
    .enumerate()
    {
        record_tick(&path, &t).unwrap();
        // In-order read: every earlier tick is still there after each append.
        assert_eq!(read_ticks(&path).len(), i + 1);
    }
    let back = read_ticks(&path);
    assert_eq!(back.len(), 3);
    assert_eq!(back[0].seq, 1);
    assert_eq!(
        back[0].payload_hash,
        stable_hash("redo the incident report")
    );
    assert_eq!(back[2].status_class, StatusClass::Fail);
    assert_eq!(back[2].latency_ms, 803);
    assert_eq!(back[2].ts_ms, 1_700_000_000_003);
    assert_eq!(back[1].run_id, "run-77");
    assert_eq!(back[2].outcome_hash, stable_hash("outcome 3"));
    assert_eq!(back[1].cache_read, 408_000);
}

#[test]
fn hash_is_stable_across_builds_estate_vectors() {
    // ESTATE hash vectors (the warden-identical fnv1a, prime literal
    // and all — see util.rs docs). Pinned so that no rebuild can ever
    // change a persisted payload hash: DefaultHasher (seeded per
    // process) cannot reproduce constants, and neither can any drift of
    // basis/prime/order.
    assert_eq!(stable_hash(""), 0xcbf2_9ce4_8422_2325);
    assert_eq!(stable_hash("a"), 0xaf74_d84c_8601_ec8c);
    assert_eq!(stable_hash("foobar"), 0xf8ac_2471_f739_67e8);
    // And the same input hashes the same in the same process, twice.
    assert_eq!(stable_hash("payload"), stable_hash("payload"));
}

/// 589 ticks = the measured 2026-08-28 burn (2.5h of 800ms re-fires).
/// They must land in ONE host-owned JSONL: nothing in the directory tree
/// but that file, no blocker, no ledger row — telemetry may not load the
/// TCB, and this card files no law.
#[test]
fn volume_589_fail_ticks_one_file_no_law() {
    let dir = tmp("volume");
    let path = dir.join("run-burn.jsonl");
    for seq in 1..=589 {
        record_tick(&path, &tick(seq, StatusClass::Fail)).unwrap();
    }
    let lines = fs::read_to_string(&path).unwrap().lines().count();
    assert_eq!(lines, 589);
    assert_eq!(read_ticks(&path).len(), 589);
    // Exactly one file in the whole tree: the JSONL itself.
    fn count_files(p: &std::path::Path) -> usize {
        fs::read_dir(p)
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .map(|sub| if sub.is_dir() { count_files(&sub) } else { 1 })
                    .sum()
            })
            .unwrap_or(0)
    }
    assert_eq!(count_files(&dir), 1);
}
