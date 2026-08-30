//! eddy_cache_health.rs — CARD-0241 RED-first. Cache-ratio is a
//! blocker CANDIDATE — health signal, never a halt. §4 rejected
//! cache-ratio as a halt law ON MEASUREMENT (the provider this estate
//! loops on emits 0/0), but _card_0173 measured the discriminator
//! where cache DOES flow: cacheWrite=0 after warmup IS the
//! monotone-prefix signal — and CARD-0230's freeze exists to protect
//! it. A loop silently eating its own cache burns ~408k input/turn
//! with nothing on screen saying so.

use caddis_organs::eddy::{cache_health, verdict, StatusClass, Tick, Verdict};
use std::fs;
use std::path::PathBuf;

fn t(seq: u64, cache_read: u64) -> Tick {
    Tick {
        run_id: "line-a".into(),
        seq,
        payload_hash: 5,
        status_class: StatusClass::Ok,
        outcome_hash: seq * 7, // all different: no stagnation interference
        artifact_hash: 0,
        cache_read,
        cache_write: 0,
        latency_ms: 0,
        ts_ms: 10_000 + seq * 1_000,
        resume_after: None,
        page: 0,
    }
}

/// Warm-then-cold: SOME tick was warm (cache_read > 0) and the
/// trailing window went cold (last STAGNANT_WINDOW ticks all zero) →
/// ONE health report naming the last warm seq.
#[test]
fn warm_then_cold_reports_once_with_last_warm_seq() {
    let ticks = vec![
        t(1, 400_000), // warm
        t(2, 410_000), // warm
        t(3, 0),       // cold
        t(4, 0),       // cold
        t(5, 0),       // cold (trailing window = 3)
    ];
    let report = cache_health(&ticks).expect("warm-then-cold reports");
    assert_eq!(report.last_warm_seq, 2, "names the last warm tick");
    assert!(
        report.why.to_lowercase().contains("cache"),
        "{}",
        report.why
    );
}

/// All-cold (the measured ollama-cloud shape): zeros are the
/// provider's answer, not a regression — NO report. Pinned.
#[test]
fn all_cold_stays_silent() {
    let ticks = vec![t(1, 0), t(2, 0), t(3, 0), t(4, 0), t(5, 0)];
    assert!(cache_health(&ticks).is_none());
}

/// Still warm at the tail: healthy, no report.
#[test]
fn warm_tail_stays_silent() {
    let ticks = vec![t(1, 400_000), t(2, 410_000), t(3, 420_000)];
    assert!(cache_health(&ticks).is_none());
}

/// Cold streak shorter than the window: not yet a phase.
#[test]
fn short_cold_streak_stays_silent() {
    let ticks = vec![t(1, 400_000), t(2, 0), t(3, 0)];
    assert!(cache_health(&ticks).is_none());
}

/// HEALTH NEVER HALTS: a cache-collapsing run still returns Continue
/// when nothing else fires — verdict(&[Tick]) does not read cache.
#[test]
fn cache_collapse_never_touches_the_verdict() {
    let ticks = vec![t(1, 400_000), t(2, 410_000), t(3, 0), t(4, 0), t(5, 0)];
    assert!(cache_health(&ticks).is_some());
    assert!(matches!(verdict(&ticks), Verdict::Continue));
}

/// enforce_health files at most ONE blocker per call sequence, source
/// eddy-health:<run_id>, reason naming the warm seq.
#[test]
fn health_blocker_files_once() {
    let dir = tmp("cacheblocker");
    let ticks = vec![t(1, 400_000), t(2, 410_000), t(3, 0), t(4, 0), t(5, 0)];
    let report = cache_health(&ticks).unwrap();
    caddis_organs::eddy::enforce_health("line-a", &report, &dir.join("blockers.jsonl"))
        .expect("files the blocker");
    // A second identical call must NOT file again (already open):
    caddis_organs::eddy::enforce_health("line-a", &report, &dir.join("blockers.jsonl"))
        .expect("idempotent");
    let open = caddis_organs::blocker::list_open_blockers(&dir.join("blockers.jsonl"));
    assert_eq!(open.len(), 1, "exactly one health blocker");
    assert_eq!(open[0].source, "eddy-health:line-a");
    assert!(open[0].reason.contains("seq 2"), "{}", open[0].reason);
}

fn tmp(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("caddis-cache-health-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}
