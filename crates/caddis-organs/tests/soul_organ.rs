//! soul_organ.rs — CARD-0253 RED-first. The soul organ with composting
//! and blocker safety-valve.
//!
//! The operator's cherry: identity that survives death, but stays
//! LIGHT. Pain composts into lessons (emotion decays to zero, wisdom
//! remains); joy persists fully. The blocker safety-valve: pain's
//! CAUSE becomes a blocker that REMINDERS until the operator resolves
//! it — forgetting pain silently is how systems rot.
//!
//! RED: today no SoulEntry, no compose, no compost exists. The test
//! cannot compile — the types and functions do not exist.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use caddis_organs::soul::{compose, compost, read_entries, write_entry, SoulEntry, SoulKind};

fn tmp(tag: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("caddis-soul-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

fn archetype(dir: &Path) -> PathBuf {
    let p = dir.join("archetype.txt");
    fs::write(
        &p,
        "ARCHETYPE: the careful builder. Born from discipline.\n",
    )
    .unwrap();
    p
}

fn seed_blocker(path: &Path, source: &str, reason: &str) {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).unwrap();
    }
    let line = format!(
        "{{\"source\":\"{source}\",\"reason\":\"{reason}\",\"ts\":\"2026-08-28T00:00:00Z\"}}\n"
    );
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    f.write_all(line.as_bytes()).unwrap();
}

/// Entry constructors: every test entry carries created_by_model "m"
/// unless a test mutates it (the lineage test needs a real name).
fn pain(cause: &str, blocker_id: &str, lesson: &str, emotion: u8, epoch: u64) -> SoulEntry {
    SoulEntry {
        kind: SoulKind::Pain {
            cause: cause.into(),
            blocker_id: blocker_id.into(),
        },
        lesson: lesson.into(),
        emotion,
        epoch,
        created_by_model: "m".into(),
    }
}

fn joy(source: &str, emotion: u8, epoch: u64) -> SoulEntry {
    SoulEntry {
        kind: SoulKind::Joy {
            source: source.into(),
        },
        lesson: String::new(),
        emotion,
        epoch,
        created_by_model: "m".into(),
    }
}

fn lesson(text: &str, epoch: u64) -> SoulEntry {
    SoulEntry {
        kind: SoulKind::Lesson { from_pain: true },
        lesson: text.into(),
        emotion: 0,
        epoch,
        created_by_model: "m".into(),
    }
}

/// RED: write_entry -> read_entries roundtrips pain losslessly (JSONL).
#[test]
fn jsonl_roundtrip_pain() {
    let dir = tmp("round-pain");
    let soul = dir.join("soul.jsonl");
    let e = pain("three failed builds", "b-1", "caution", 3, 1);
    write_entry(&soul, &e).unwrap();
    assert_eq!(read_entries(&soul), vec![e]);
}

/// RED: write_entry -> read_entries roundtrips joy losslessly (JSONL).
#[test]
fn jsonl_roundtrip_joy() {
    let dir = tmp("round-joy");
    let soul = dir.join("soul.jsonl");
    let e = joy("green pipeline", 10, 0);
    write_entry(&soul, &e).unwrap();
    assert_eq!(read_entries(&soul), vec![e]);
}

/// RED: a pain entry composts to a lesson when emotion decays to zero.
/// The lesson line survives; the emotional charge dissolves.
#[test]
fn pain_composts_to_lesson() {
    let dir = tmp("compost");
    let soul = dir.join("soul.jsonl");
    let blockers = dir.join("blockers.jsonl");

    write_entry(&soul, &pain("three failed builds", "", "caution", 4, 0)).unwrap();

    // Age=10 > N=3: emotion halves each call: 4->2->1->0.
    for _ in 0..10 {
        compost(&soul, &blockers, 10, 3).unwrap();
    }

    let entries = read_entries(&soul);
    assert_eq!(entries.len(), 1, "entry survives");
    assert!(matches!(
        entries[0].kind,
        SoulKind::Lesson { from_pain: true }
    ));
    assert_eq!(entries[0].emotion, 0, "emotion dissolved");
    assert_eq!(entries[0].lesson, "caution", "lesson survives");
}

/// RED: a pain entry with an OPEN blocker files a reminder on compost.
/// The safety-valve persists the indication until addressed.
#[test]
fn open_blocker_files_reminder_on_compost() {
    let dir = tmp("safety");
    let soul = dir.join("soul.jsonl");
    let blockers = dir.join("blockers.jsonl");

    seed_blocker(&blockers, "b-002", "broken build");

    write_entry(
        &soul,
        &pain("broken build", "b-002", "verify before push", 1, 0),
    )
    .unwrap();

    compost(&soul, &blockers, 10, 3).unwrap(); // 1/2=0 -> composts

    let open = caddis_organs::blocker::list_open_blockers(&blockers);
    let reminder = open.iter().find(|b| b.source.contains("soul-reminder"));
    assert!(reminder.is_some(), "reminder filed for open blocker");
    assert!(
        reminder.unwrap().reason.contains("b-002"),
        "reminder names the cause"
    );
}

/// RED: a pain entry whose blocker is RESOLVED composts silently.
#[test]
fn resolved_blocker_composts_silently() {
    let dir = tmp("silent");
    let soul = dir.join("soul.jsonl");
    let blockers = dir.join("blockers.jsonl"); // no blockers filed

    write_entry(&soul, &pain("fixed", "b-003", "test first", 1, 0)).unwrap();

    compost(&soul, &blockers, 10, 3).unwrap();

    let open = caddis_organs::blocker::list_open_blockers(&blockers);
    assert!(open.is_empty(), "no reminder when blocker is resolved");
}

/// RED: joy persists fully — it never composts.
#[test]
fn joy_never_composts() {
    let dir = tmp("joy");
    let soul = dir.join("soul.jsonl");
    let blockers = dir.join("blockers.jsonl");

    write_entry(&soul, &joy("green pipeline", 10, 0)).unwrap();

    for _ in 0..20 {
        compost(&soul, &blockers, 100, 3).unwrap();
    }

    let entries = read_entries(&soul);
    assert_eq!(entries.len(), 1);
    assert!(matches!(entries[0].kind, SoulKind::Joy { .. }));
    assert_eq!(entries[0].emotion, 10, "joy does not decay");
}

/// RED: the weight cap prunes oldest emotion==0 entries when soul.jsonl
/// exceeds 4KB at composition time.
#[test]
fn weight_cap_prunes_dead_lessons() {
    let dir = tmp("cap");
    let soul = dir.join("soul.jsonl");
    let _blockers = dir.join("blockers.jsonl");

    for i in 0..200 {
        let text = format!("lesson {i} with enough text to exceed the 4KB budget");
        write_entry(&soul, &lesson(&text, i)).unwrap();
    }
    write_entry(&soul, &joy("live", 10, 999)).unwrap();

    compose(&soul, &archetype(&dir));

    let size = fs::read_to_string(&soul).unwrap().len();
    assert!(size <= 4096, "pruned to {} bytes", size);
    let entries = read_entries(&soul);
    assert!(
        entries
            .iter()
            .any(|e| matches!(e.kind, SoulKind::Joy { .. })),
        "live joy survives"
    );
}

/// RED: compose frames pain as growth, not hurt.
#[test]
fn compose_frames_pain_as_growth() {
    let dir = tmp("framing");
    let soul = dir.join("soul.jsonl");

    write_entry(&soul, &lesson("caution after three failures", 0)).unwrap();

    let text = compose(&soul, &archetype(&dir));
    assert!(
        text.to_lowercase().contains("learn"),
        "growth framing: {}",
        text
    );
    assert!(
        !text.to_lowercase().contains("hurt"),
        "no hurt framing: {}",
        text
    );
}

/// RED: the soul belongs to the lineage — compose never references
/// the model name.
#[test]
fn compose_never_references_model() {
    let dir = tmp("lineage");
    let soul = dir.join("soul.jsonl");

    let mut e = joy("green", 10, 5);
    e.created_by_model = "glm-5.2".into();
    write_entry(&soul, &e).unwrap();

    let text = compose(&soul, &archetype(&dir));
    assert!(!text.contains("glm-5.2"), "no model name: {}", text);
    assert!(
        !text.contains("created_by_model"),
        "no field name: {}",
        text
    );
}
