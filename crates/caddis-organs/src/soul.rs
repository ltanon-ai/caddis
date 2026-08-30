//! soul.rs — CARD-0253. The soul organ with composting and blocker
//! safety-valve. Identity that survives death, but stays LIGHT: pain
//! composts into lessons (emotion decays to zero, wisdom remains); joy
//! persists fully. Pain's CAUSE becomes a blocker that REMINDERS until
//! resolved — forgetting pain silently is how systems rot. The soul
//! belongs to the LINEAGE: entries carry `created_by_model` but compose
//! never references it. Storage: append-only JSONL.

use std::io::{self, Write};
use std::path::Path;

use crate::blocker::{file_blocker, list_open_blockers, Blocker};
use crate::util::{json_escape, json_str_field};

/// The kind of soul entry. Pain carries a cause and a blocker_id;
/// Joy carries a source; Lesson is the composted remnant of Pain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SoulKind {
    Pain { cause: String, blocker_id: String },
    Joy { source: String },
    Lesson { from_pain: bool },
}

/// One soul entry. Emotion is 0..=10 for pain/joy; lessons are always 0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoulEntry {
    pub kind: SoulKind,
    pub lesson: String,
    pub emotion: u8,
    pub epoch: u64,
    pub created_by_model: String,
}

/// Weight cap: prune oldest emotion==0 entries once soul.jsonl exceeds this many bytes.
pub const WEIGHT_CAP: usize = 4096;

impl SoulEntry {
    fn to_jsonl(&self) -> String {
        let (kind_str, kind_fields) = match &self.kind {
            SoulKind::Pain { cause, blocker_id } => (
                "Pain",
                format!(
                    "\"cause\":\"{}\",\"blocker_id\":\"{}\"",
                    json_escape(cause),
                    json_escape(blocker_id),
                ),
            ),
            SoulKind::Joy { source } => ("Joy", format!("\"source\":\"{}\"", json_escape(source))),
            SoulKind::Lesson { from_pain } => ("Lesson", format!("\"from_pain\":{from_pain}")),
        };
        format!(
            "{{\"kind\":\"{kind_str}\",{kind_fields},\"lesson\":\"{}\",\
             \"emotion\":{},\"epoch\":{},\"created_by_model\":\"{}\"}}",
            json_escape(&self.lesson),
            self.emotion,
            self.epoch,
            json_escape(&self.created_by_model),
        )
    }

    fn from_jsonl(line: &str) -> Option<Self> {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        let kind_str = json_str_field(line, "kind")?;
        let lesson = json_str_field(line, "lesson").unwrap_or_default();
        let emotion = num_field(line, "emotion").unwrap_or(0) as u8;
        let epoch = num_field(line, "epoch").unwrap_or(0);
        let created_by_model = json_str_field(line, "created_by_model").unwrap_or_default();
        let kind = match kind_str.as_str() {
            "Pain" => SoulKind::Pain {
                cause: json_str_field(line, "cause").unwrap_or_default(),
                blocker_id: json_str_field(line, "blocker_id").unwrap_or_default(),
            },
            "Joy" => SoulKind::Joy {
                source: json_str_field(line, "source").unwrap_or_default(),
            },
            "Lesson" => SoulKind::Lesson {
                from_pain: num_field(line, "from_pain").map(|v| v != 0).unwrap_or(true),
            },
            _ => return None,
        };
        Some(Self {
            kind,
            lesson,
            emotion,
            epoch,
            created_by_model,
        })
    }
}

fn num_field(line: &str, key: &str) -> Option<u64> {
    let pat = format!("\"{key}\":");
    let start = line.find(&pat)? + pat.len();
    let digits: String = line[start..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

/// Append one soul entry (best-effort file create, append-per-line).
pub fn write_entry(path: &Path, entry: &SoulEntry) -> io::Result<()> {
    use std::fs::OpenOptions;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut f = OpenOptions::new().create(true).append(true).open(path)?;
    f.write_all(entry.to_jsonl().as_bytes())?;
    f.write_all(b"\n")
}

/// Read all soul entries, in file order (absent file = none).
pub fn read_entries(path: &Path) -> Vec<SoulEntry> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines().filter_map(SoulEntry::from_jsonl).collect()
}

/// Compost one pass: decay pain older than `current_epoch - max_age`
/// by half. When emotion reaches 0, the entry becomes a Lesson. If the
/// pain's blocker_id is still open at that moment, file a REMINDER
/// blocker (source `soul-reminder:<id>`). Returns the number of
/// entries composted to Lesson.
pub fn compost(
    soul_path: &Path,
    blockers_path: &Path,
    current_epoch: u64,
    max_age: u64,
) -> io::Result<usize> {
    let entries = read_entries(soul_path);
    if entries.is_empty() {
        return Ok(0);
    }
    let mut composted = 0usize;
    let mut changed = false;
    let mut out = String::new();
    for mut e in entries {
        if let SoulKind::Pain { cause, blocker_id } = &e.kind {
            let (cause, blocker_id) = (cause.clone(), blocker_id.clone());
            decay_pain(&mut e, current_epoch, max_age, &mut changed);
            if e.emotion == 0 {
                e.kind = SoulKind::Lesson { from_pain: true };
                composted += 1;
                changed = true;
                file_reminder_if_open(blockers_path, &cause, &blocker_id)?;
            }
        }
        out.push_str(&e.to_jsonl());
        out.push('\n');
    }
    if changed {
        std::fs::write(soul_path, out)?;
    }
    Ok(composted)
}

/// Decay a pain entry's emotion by half if it's older than max_age.
fn decay_pain(e: &mut SoulEntry, current_epoch: u64, max_age: u64, changed: &mut bool) {
    let age = current_epoch.saturating_sub(e.epoch);
    if age > max_age && e.emotion > 0 {
        e.emotion /= 2;
        *changed = true;
    }
}

/// Safety valve: file a REMINDER blocker if the pain's blocker_id is
/// still open. Forgetting pain silently is how systems rot.
fn file_reminder_if_open(blockers_path: &Path, cause: &str, blocker_id: &str) -> io::Result<()> {
    if blocker_id.is_empty() {
        return Ok(());
    }
    let still_open = list_open_blockers(blockers_path)
        .iter()
        .any(|b| b.source == blocker_id);
    if still_open {
        file_blocker(
            blockers_path,
            &Blocker {
                source: format!("soul-reminder:{blocker_id}"),
                reason: format!("unresolved cause: {cause} [{blocker_id}]"),
                ts: crate::util::iso8601_now(),
            },
        )?;
    }
    Ok(())
}

/// Compose the living identity for the session-start orientation packet
/// HEAD: the archetype (static birth certificate), the last 5 live
/// entries, and all distilled lessons. Also enforces the weight cap by
/// pruning the oldest emotion==0 entries once soul.jsonl is too heavy.
pub fn compose(soul_path: &Path, archetype_path: &Path) -> String {
    enforce_weight_cap(soul_path);
    let archetype = std::fs::read_to_string(archetype_path).unwrap_or_default();
    let entries = read_entries(soul_path);
    let live: Vec<&SoulEntry> = entries.iter().rev().take(5).collect();
    let lessons: Vec<&SoulEntry> = entries
        .iter()
        .filter(|e| matches!(e.kind, SoulKind::Lesson { from_pain: true }) && !e.lesson.is_empty())
        .collect();

    let mut text = String::new();
    text.push_str(&archetype);
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str("## SOUL (lineage)\n");
    if lessons.is_empty() && live.is_empty() {
        text.push_str("No lessons yet.\n");
    }
    for e in &lessons {
        text.push_str(&format!("- I learned {}\n", e.lesson));
    }
    for e in live.iter().rev() {
        let line = match &e.kind {
            SoulKind::Pain { .. } => format!("- active: {} (charge {})\n", e.lesson, e.emotion),
            SoulKind::Joy { source } => format!("- joy from {}\n", source),
            SoulKind::Lesson { .. } => format!("- I learned {}\n", e.lesson),
        };
        text.push_str(&line);
    }
    text
}

/// Prune oldest emotion==0 entries when soul.jsonl exceeds WEIGHT_CAP
/// bytes. Live entries (emotion > 0) are never pruned.
fn enforce_weight_cap(path: &Path) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    if text.len() <= WEIGHT_CAP {
        return;
    }
    let entries = read_entries(path);
    // Keep all live entries, then newest dead lessons while they fit;
    // the oldest dead are the prune candidates (CARD-0253 weight cap).
    let mut kept: Vec<SoulEntry> = entries.iter().filter(|e| e.emotion > 0).cloned().collect();
    for e in entries.iter().filter(|e| e.emotion == 0).rev() {
        kept.push(e.clone());
        if kept.iter().map(|k| k.to_jsonl().len() + 1).sum::<usize>() > WEIGHT_CAP {
            kept.pop();
            break;
        }
    }
    let mut out = String::new();
    for e in &kept {
        out.push_str(&e.to_jsonl());
        out.push('\n');
    }
    // swallow: best-effort-cleanup — a write failure leaves the old file, which is safe (next compose retries)
    let _ = std::fs::write(path, out);
}
