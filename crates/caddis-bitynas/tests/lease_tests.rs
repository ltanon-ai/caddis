//! lease_tests.rs — CARD-BITYNAS-1 acceptance: seven RED-first integration
//! tests over a real temp journal. Stale leases are SEEDED as raw
//! hand-written journal rows (fixed 2020 timestamps), so every test is
//! deterministic — no sleeps, no clock injection — and the seeded rows
//! double as an external contract test of the on-disk format.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use caddis_bitynas::{lane_allowed, LeaseOwner, LeaseStore};

fn mk_owner(ses: &str) -> LeaseOwner {
    LeaseOwner {
        session_id: ses.to_string(),
        host: format!("host-{ses}"),
        pid: 100 + ses.bytes().last().unwrap() as u32,
    }
}

fn tmp_dir(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    std::env::temp_dir().join(format!(
        "bitynas-t-{name}-{}-{nanos}",
        std::process::id()
    ))
}

fn journal_in(dir: &Path) -> PathBuf {
    dir.join("leases.jsonl")
}

/// Append one hand-written STALE claim row (hb `2020-01-01T00:00:00Z`,
/// ttl 900) and return the owner that seeded it.
fn seed_stale_row(path: &Path, slot_id: &str) -> LeaseOwner {
    let seeded = LeaseOwner {
        session_id: "ses-old".to_string(),
        host: "host-old".to_string(),
        pid: 1,
    };
    let row = format!(
        "{{\"op\":\"claim\",\"slot_id\":\"{slot_id}\",\"lane\":\"free\",\
         \"session_id\":\"ses-old\",\"host\":\"host-old\",\"pid\":1,\
         \"taken_at_utc\":\"2020-01-01T00:00:00Z\",\"ttl_s\":900,\
         \"heartbeat_at_utc\":\"2020-01-01T00:00:00Z\"}}\n"
    );
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut f = OpenOptions::new().create(true).append(true).open(path).unwrap();
    f.write_all(row.as_bytes()).unwrap();
    seeded
}

fn cleanup(dir: &Path) {
    let _ = std::fs::remove_file(journal_in(dir));
    let _ = std::fs::remove_dir(dir);
}

#[test]
fn double_claim_same_slot_second_gets_busy_with_holder_identity() {
    let dir = tmp_dir("double");
    let path = journal_in(&dir);
    let mut store = LeaseStore::open(&path).unwrap();
    let a = mk_owner("ses-A");
    let b = mk_owner("ses-B");

    let rec = store.claim("gpu-0", "premium", a.clone()).unwrap();
    assert_eq!(rec.slot_id, "gpu-0");
    assert_eq!(rec.lane, "premium");
    assert_eq!(rec.ttl_s, caddis_bitynas::DEFAULT_TTL_S);
    assert_eq!(rec.taken_at_utc, rec.heartbeat_at_utc);

    // a DIFFERENT session claiming the live slot gets Busy WITH the holder:
    let err = store.claim("gpu-0", "premium", b).unwrap_err();
    assert_eq!(err.holder.session_id, "ses-A");
    assert_eq!(err.holder.host, "host-ses-A");
    assert_eq!(err.holder.pid, a.pid);

    // same-owner re-claim is Busy too — refreshing is heartbeat's job:
    assert!(store.claim("gpu-0", "premium", a).is_err());
    cleanup(&dir);
}

#[test]
fn ttl_expiry_sweep_frees_slot_and_emits_peremption() {
    let dir = tmp_dir("ttl");
    let path = journal_in(&dir);
    seed_stale_row(&path, "gpu-x");
    let mut store = LeaseStore::open(&path).unwrap();

    // exactly AT the ttl the lease is still live (strict >):
    assert!(store.sweep("2020-01-01T00:15:00Z").is_empty());
    assert!(store.held("gpu-x").is_some());

    // one second past it, the sweep frees and SPEAKS:
    let events = store.sweep("2020-01-01T00:15:01Z");
    assert_eq!(events.len(), 1);
    let ev = &events[0];
    assert_eq!(ev.slot_id, "gpu-x");
    assert_eq!(ev.cause, "ttl_expired");
    assert_eq!(ev.previous.session_id, "ses-old");
    assert!(ev.new_owner.is_none());
    assert!(store.held("gpu-x").is_none());

    // a freed slot is claimable again:
    assert!(store.claim("gpu-x", "free", mk_owner("ses-N")).is_ok());
    // sweep events were RETURNED, not queued (never reported twice):
    assert!(store.events().is_empty());
    cleanup(&dir);
}

#[test]
fn stale_slot_is_reclaimed_by_claim_and_event_is_pending() {
    let dir = tmp_dir("reclaim");
    let path = journal_in(&dir);
    let old = seed_stale_row(&path, "gpu-5");
    let mut store = LeaseStore::open(&path).unwrap();
    assert!(store.events().is_empty()); // replay never re-enqueues history

    let b = mk_owner("ses-B");
    let rec = store.claim("gpu-5", "mid", b.clone()).unwrap();
    assert_eq!(rec.session_id, "ses-B");

    let events = store.events();
    assert_eq!(events.len(), 1);
    let ev = &events[0];
    assert_eq!(ev.previous.session_id, "ses-old");
    assert_eq!(ev.new_owner.as_ref(), Some(&b));
    assert_eq!(ev.cause, "ttl_expired");

    // the displaced owner can no longer release:
    let err = store.release("gpu-5", &old).unwrap_err();
    assert_eq!(err.holder.session_id, "ses-B");
    assert!(store.events().is_empty()); // drained exactly once
    cleanup(&dir);
}

#[test]
fn wrong_owner_release_is_rejected_right_owner_succeeds() {
    let dir = tmp_dir("release");
    let path = journal_in(&dir);
    let mut store = LeaseStore::open(&path).unwrap();
    let a = mk_owner("ses-A");
    let b = mk_owner("ses-B");
    store.claim("gpu-0", "premium", a.clone()).unwrap();

    let err = store.release("gpu-0", &b).unwrap_err();
    assert_eq!(err.holder.session_id, "ses-A");

    store.release("gpu-0", &a).unwrap();
    assert!(store.held("gpu-0").is_none());
    assert!(store.claim("gpu-0", "premium", b).is_ok());
    cleanup(&dir);
}

#[test]
fn heartbeat_refresh_keeps_lease_alive_across_sweep() {
    let dir = tmp_dir("hb");
    let path = journal_in(&dir);
    let old = seed_stale_row(&path, "gpu-9");
    let mut store = LeaseStore::open(&path).unwrap();

    let rec = store
        .heartbeat("gpu-9", &old)
        .unwrap()
        .expect("seeded lease must heartbeat");
    assert!(rec.heartbeat_at_utc.as_str() >= "2026-01-01T00:00:00Z"); // refreshed to real now

    // the old deadline no longer applies:
    assert!(store.sweep("2021-01-01T00:00:00Z").is_empty());
    assert!(store.held("gpu-9").is_some());
    // wrong owner cannot refresh:
    assert!(store.heartbeat("gpu-9", &mk_owner("ses-B")).is_err());

    // the refreshed lease survives a crash-reopen (the row was journaled):
    drop(store);
    let mut store = LeaseStore::open(&path).unwrap();
    assert!(store.sweep("2021-01-01T00:00:00Z").is_empty());
    // but it is still mortal:
    assert_eq!(store.sweep("2999-01-01T00:00:00Z").len(), 1);
    cleanup(&dir);
}

#[test]
fn journal_reopen_rebuilds_index_and_sweeps_only_stale() {
    let dir = tmp_dir("reopen");
    let path = journal_in(&dir);
    {
        let mut store = LeaseStore::open(&path).unwrap();
        store.claim("gpu-1", "premium", mk_owner("ses-A")).unwrap();
        store.claim("gpu-2", "free", mk_owner("ses-B")).unwrap();
    } // store dropped — the "crash" is process end; the journal survives
    seed_stale_row(&path, "gpu-3");

    let mut store = LeaseStore::open(&path).unwrap();
    assert_eq!(store.unreadable(), 0);
    assert_eq!(store.held("gpu-1").unwrap().session_id, "ses-A");
    assert_eq!(store.held("gpu-2").unwrap().session_id, "ses-B");
    assert_eq!(store.held("gpu-3").unwrap().session_id, "ses-old");

    let events = store.sweep("2021-01-01T00:00:00Z");
    assert_eq!(events.len(), 1); // ONLY the stale one
    assert_eq!(events[0].slot_id, "gpu-3");
    assert!(store.held("gpu-1").is_some());
    assert!(store.held("gpu-2").is_some());
    assert!(store.held("gpu-3").is_none());

    // identity survived the reopen:
    let err = store.release("gpu-2", &mk_owner("ses-A")).unwrap_err();
    assert_eq!(err.holder.session_id, "ses-B");
    store.release("gpu-2", &mk_owner("ses-B")).unwrap();
    cleanup(&dir);
}

#[test]
fn lane_allowed_o2_droid_refused_closed_vocabulary() {
    for tier in ["droid", "DROID", " droid "] {
        let err = lane_allowed(tier).unwrap_err();
        assert!(err.contains("droid"), "message must name droid: {err}");
        assert!(err.contains("O2"), "message must cite the law: {err}");
    }
    for tier in ["local", "free", "Mid", " premium "] {
        lane_allowed(tier).unwrap();
    }
    for tier in ["banana", ""] {
        assert!(lane_allowed(tier).is_err());
    }
}
