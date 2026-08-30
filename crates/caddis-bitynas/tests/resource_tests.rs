//! resource_tests.rs — CARD-BITYNAS-6 acceptance: typed resources on the
//! lease core. Same deterministic pattern as lease_tests.rs: stale or
//! far-future leases are SEEDED as hand-written journal rows with fixed
//! timestamps — no sleeps — and the seeded rows double as external
//! contract tests of the namespaced on-disk format.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use caddis_bitynas::{
    BusyError, ClaimOutcome, LeaseOwner, LeaseRecord, LeaseStore, ResourceType, COUNCIL_TTL_S,
    DEFAULT_TTL_S, QUORUM_TTL_S, VOICE_TTL_S,
};

fn mk_owner(ses: &str) -> LeaseOwner {
    LeaseOwner {
        session_id: ses.to_string(),
        host: format!("host-{ses}"),
        pid: 100 + ses.bytes().last().unwrap() as u32,
    }
}

/// claim_typed with a one-line owner — pure brevity for the call sites.
fn ct(
    store: &mut LeaseStore,
    ty: ResourceType,
    id: &str,
    lane: &str,
    ses: &str,
    hash: Option<&str>,
) -> ClaimOutcome {
    store.claim_typed(ty, id, lane, mk_owner(ses), hash)
}

fn tmp_dir(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    std::env::temp_dir().join(format!(
        "bitynas-rt-{name}-{}-{nanos}",
        std::process::id()
    ))
}

fn journal_in(dir: &Path) -> PathBuf {
    dir.join("leases.jsonl")
}

/// Append one hand-written claim row (`ses-old` identity, `at` stamped on
/// both clocks, optional quorum `question_hash`).
fn seed_row(path: &Path, slot_id: &str, ttl_s: u64, at: &str, hash: Option<&str>) -> LeaseOwner {
    let hash = hash.map(|h| format!(",\"question_hash\":\"{h}\"")).unwrap_or_default();
    let row = format!(
        "{{\"op\":\"claim\",\"slot_id\":\"{slot_id}\",\"lane\":\"free\",\
         \"session_id\":\"ses-old\",\"host\":\"host-old\",\"pid\":1,\
         \"taken_at_utc\":\"{at}\",\"ttl_s\":{ttl_s},\
         \"heartbeat_at_utc\":\"{at}\"{hash}}}\n"
    );
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut f = OpenOptions::new().create(true).append(true).open(path).unwrap();
    f.write_all(row.as_bytes()).unwrap();
    LeaseOwner {
        session_id: "ses-old".to_string(),
        host: "host-old".to_string(),
        pid: 1,
    }
}

fn cleanup(dir: &Path) {
    let _ = std::fs::remove_file(journal_in(dir));
    let _ = std::fs::remove_dir(dir);
}

fn claimed(outcome: ClaimOutcome) -> LeaseRecord {
    match outcome {
        ClaimOutcome::Claimed(rec) => rec,
        other => panic!("expected Claimed, got {other:?}"),
    }
}

fn busy(outcome: ClaimOutcome) -> BusyError {
    match outcome {
        ClaimOutcome::Busy(err) => err,
        other => panic!("expected Busy, got {other:?}"),
    }
}

#[test]
fn council_capacity_one_second_claim_busy_with_holder() {
    let dir = tmp_dir("council-cap");
    let mut store = LeaseStore::open(&journal_in(&dir)).unwrap();

    let rec = claimed(ct(&mut store, ResourceType::Council, "panel", "premium", "ses-A", None));
    assert_eq!(rec.slot_id, "council:panel");
    assert_eq!(rec.ttl_s, COUNCIL_TTL_S);

    // capacity 1: the second claimer is refused AND the holder named:
    let err = busy(ct(&mut store, ResourceType::Council, "panel", "premium", "ses-B", None));
    assert_eq!(err.holder.session_id, "ses-A");
    assert_eq!(err.holder.slot_id, "council:panel");
    assert!(store.held("council:panel").is_some());
    cleanup(&dir);
}

#[test]
fn voice_short_ttl_expires_in_sweep() {
    let dir = tmp_dir("voice-ttl");
    let path = journal_in(&dir);
    seed_row(&path, "voice:main", VOICE_TTL_S, "2020-01-01T00:00:00Z", None);
    let mut store = LeaseStore::open(&path).unwrap();

    // exactly AT 60 s the lease is still live (strict >):
    assert!(store.sweep("2020-01-01T00:01:00Z").is_empty());
    assert!(store.held("voice:main").is_some());

    // one second past it, the sweep frees and SPEAKS:
    let events = store.sweep("2020-01-01T00:01:01Z");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].slot_id, "voice:main");
    assert!(store.held("voice:main").is_none());

    // a fresh voice claim stamps the SHORT policy ttl:
    let rec = claimed(ct(&mut store, ResourceType::Voice, "main", "voice-lane", "ses-V", None));
    assert_eq!(rec.ttl_s, VOICE_TTL_S);
    assert_eq!(rec.slot_id, "voice:main");
    cleanup(&dir);
}

#[test]
fn quorum_same_question_hash_joins_existing() {
    let dir = tmp_dir("quorum-join");
    let mut store = LeaseStore::open(&journal_in(&dir)).unwrap();

    let rec = claimed(ct(&mut store, ResourceType::Quorum, "panel", "mid", "ses-A", Some("hash-1")));
    assert_eq!(rec.slot_id, "quorum:panel");
    assert_eq!(rec.question_hash.as_deref(), Some("hash-1"));

    // a SECOND client, another slot id, the SAME question: not a second
    // consultation — a JOIN of the live one (the holder is named):
    match ct(&mut store, ResourceType::Quorum, "panel-b", "mid", "ses-B", Some("hash-1")) {
        ClaimOutcome::Join { existing } => {
            assert_eq!(existing.session_id, "ses-A");
            assert_eq!(existing.slot_id, "quorum:panel");
        }
        other => panic!("expected Join, got {other:?}"),
    }
    // the join wrote nothing: still exactly ONE lease for one question:
    assert!(store.held("quorum:panel-b").is_none());
    assert_eq!(store.held("quorum:panel").unwrap().session_id, "ses-A");
    // the holder re-asking their own live question is idempotent:
    assert!(matches!(
        ct(&mut store, ResourceType::Quorum, "panel", "mid", "ses-A", Some("hash-1")),
        ClaimOutcome::Join { .. }
    ));
    cleanup(&dir);

    // a STALE consultation with the same hash is preempted, never joined:
    let dir = tmp_dir("quorum-stale-hash");
    let path = journal_in(&dir);
    seed_row(&path, "quorum:z", QUORUM_TTL_S, "2020-01-01T00:00:00Z", Some("hash-z"));
    let mut store = LeaseStore::open(&path).unwrap();
    let rec = claimed(ct(&mut store, ResourceType::Quorum, "z", "mid", "ses-C", Some("hash-z")));
    assert_eq!(rec.session_id, "ses-C");
    let events = store.events();
    assert_eq!(events.len(), 1); // preempted, not joined
    assert_eq!(events[0].previous.question_hash.as_deref(), Some("hash-z"));
    cleanup(&dir);
}

#[test]
fn quorum_different_question_hash_is_busy() {
    let dir = tmp_dir("quorum-busy");
    let mut store = LeaseStore::open(&journal_in(&dir)).unwrap();
    claimed(ct(&mut store, ResourceType::Quorum, "panel", "mid", "ses-A", Some("hash-1")));

    // a DIFFERENT question on the same slot: no join — a second
    // consultation on a capacity-1 resource is Busy with the holder:
    let err = busy(ct(&mut store, ResourceType::Quorum, "panel", "mid", "ses-B", Some("hash-2")));
    assert_eq!(err.holder.session_id, "ses-A");
    assert_eq!(err.holder.slot_id, "quorum:panel");
    cleanup(&dir);
}

#[test]
fn legacy_bare_row_parses_as_beeslot_shared_namespace() {
    let dir = tmp_dir("legacy-bare");
    let path = journal_in(&dir);
    // an H-1-format row: bare id, far-future heartbeat => LIVE:
    seed_row(&path, "gpu-7", DEFAULT_TTL_S, "2999-01-01T00:00:00Z", None);
    let mut store = LeaseStore::open(&path).unwrap();

    // legacy claim() still sees it:
    let err = store.claim("gpu-7", "premium", mk_owner("ses-B")).unwrap_err();
    assert_eq!(err.holder.session_id, "ses-old");
    // and the TYPED bee claim shares that one bare namespace:
    let err = busy(ct(&mut store, ResourceType::BeeSlot, "gpu-7", "premium", "ses-C", None));
    assert_eq!(err.holder.session_id, "ses-old");

    // typed bee WRITES stay bare — one namespace, no `bee:` on disk:
    let rec = claimed(ct(&mut store, ResourceType::BeeSlot, "gpu-9", "premium", "ses-D", None));
    assert_eq!(rec.slot_id, "gpu-9");
    assert!(store.held("bee:gpu-9").is_none());
    // and legacy claim() sees the typed bee lease just the same:
    let err = store.claim("gpu-9", "premium", mk_owner("ses-E")).unwrap_err();
    assert_eq!(err.holder.session_id, "ses-D");
    cleanup(&dir);
}

#[test]
fn type_prefixed_slots_do_not_collide() {
    let dir = tmp_dir("no-collide");
    let mut store = LeaseStore::open(&journal_in(&dir)).unwrap();

    // the same bare name under four types: four DISTINCT slots, all live:
    claimed(ct(&mut store, ResourceType::BeeSlot, "main", "free", "ses-Bee", None));
    claimed(ct(&mut store, ResourceType::Council, "main", "premium", "ses-Cou", None));
    claimed(ct(&mut store, ResourceType::Quorum, "main", "mid", "ses-Quo", Some("h1")));
    claimed(ct(&mut store, ResourceType::Voice, "main", "voice-lane", "ses-Voi", None));

    assert_eq!(store.held("main").unwrap().session_id, "ses-Bee");
    assert_eq!(store.held("council:main").unwrap().session_id, "ses-Cou");
    assert_eq!(store.held("quorum:main").unwrap().session_id, "ses-Quo");
    assert_eq!(store.held("voice:main").unwrap().session_id, "ses-Voi");
    cleanup(&dir);
}

#[test]
fn typed_ttl_policies_stamped_per_type_and_heartbeat_keeps_council_alive() {
    let dir = tmp_dir("ttl-policy");
    let mut store = LeaseStore::open(&journal_in(&dir)).unwrap();

    let bee = claimed(ct(&mut store, ResourceType::BeeSlot, "gpu-0", "free", "s1", None));
    let cou = claimed(ct(&mut store, ResourceType::Council, "panel", "premium", "s2", None));
    let quo = claimed(ct(&mut store, ResourceType::Quorum, "q", "mid", "s3", None));
    let voi = claimed(ct(&mut store, ResourceType::Voice, "v", "voice-lane", "s4", None));
    assert_eq!((bee.ttl_s, cou.ttl_s, quo.ttl_s, voi.ttl_s),
        (DEFAULT_TTL_S, COUNCIL_TTL_S, QUORUM_TTL_S, VOICE_TTL_S));
    cleanup(&dir);

    // "su heartbeat": a seeded-stale council lease, refreshed by
    // heartbeat, survives a sweep that would have killed it:
    let dir = tmp_dir("council-hb");
    let path = journal_in(&dir);
    let old = seed_row(&path, "council:panel", COUNCIL_TTL_S, "2020-01-01T00:00:00Z", None);
    let mut store = LeaseStore::open(&path).unwrap();
    store
        .heartbeat("council:panel", &old)
        .unwrap()
        .expect("seeded council lease must heartbeat");
    assert!(store.sweep("2021-01-01T00:00:00Z").is_empty());
    assert!(store.held("council:panel").is_some());
    cleanup(&dir);
}
