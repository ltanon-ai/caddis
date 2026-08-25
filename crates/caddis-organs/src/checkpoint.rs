//! checkpoint.rs — the self-undo organ (wave 1).
//! Pre-mutation snapshots: the "unwired core purpose" from the QPI corpus
//! (git-checkpoint was only a CLI backup; the point is the snapshot taken
//! BEFORE every mutation so any change can be rolled back).
//! Harness-agnostic: plain std::fs copies into a host-owned store; no git,
//! no deps. The host calls [`CheckpointStore::snapshot`] right before a
//! write-class tool call and `restore` to undo.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::util::{iso8601_now, json_escape, json_str_array_field, json_str_field};

/// One checkpoint's manifest entry (a line in `<root>/index.jsonl`).
#[derive(Debug, Clone, PartialEq)]
pub struct CheckpointMeta {
    pub id: String,
    pub label: String,
    pub ts: String,
    /// Files captured, in the order given (absolute paths).
    pub files: Vec<String>,
}

impl CheckpointMeta {
    fn to_jsonl(&self) -> String {
        let files = self
            .files
            .iter()
            .map(|f| format!("\"{}\"", json_escape(f)))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "{{\"id\":\"{}\",\"label\":\"{}\",\"ts\":\"{}\",\"files\":[{}]}}",
            json_escape(&self.id),
            json_escape(&self.label),
            json_escape(&self.ts),
            files
        )
    }
}

/// The snapshot store. Layout:
/// ```text
/// <root>/index.jsonl        one manifest line per checkpoint
/// <root>/<id>/<n>.snap      captured bytes, n = position in `files`
/// <root>/<id>/<n>.absent    marker: the file did NOT exist pre-mutation
/// ```
/// The manifest line is written LAST: a snapshot visible in the index is a
/// snapshot whose bytes are already on disk.
pub struct CheckpointStore {
    root: PathBuf,
}

impl CheckpointStore {
    /// Open (and create) the store directory.
    pub fn open(root: &Path) -> io::Result<Self> {
        fs::create_dir_all(root)?;
        Ok(CheckpointStore {
            root: root.to_path_buf(),
        })
    }

    /// Snapshot the given files BEFORE a mutation. Returns the checkpoint id.
    /// A listed file that does not exist is recorded as `.absent` — restoring
    /// then DELETES it, so a mutation that creates files is also undone.
    pub fn snapshot(&self, label: &str, paths: &[PathBuf]) -> io::Result<String> {
        let id = format!("ckpt-{}", crate::util::unix_ms());
        let dir = self.root.join(&id);
        fs::create_dir_all(&dir)?;
        let mut names: Vec<String> = Vec::with_capacity(paths.len());
        for (n, p) in paths.iter().enumerate() {
            let abs = p.canonicalize().unwrap_or_else(|_| p.clone());
            if abs.is_file() {
                fs::copy(p, dir.join(format!("{n}.snap")))?;
            } else {
                fs::write(dir.join(format!("{n}.absent")), b"")?;
            }
            names.push(abs.to_string_lossy().into_owned());
        }
        let meta = CheckpointMeta {
            id: id.clone(),
            label: label.to_string(),
            ts: iso8601_now(),
            files: names,
        };
        // Manifest last (see struct doc).
        let mut idx = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.root.join("index.jsonl"))?;
        idx.write_all(meta.to_jsonl().as_bytes())?;
        idx.write_all(b"\n")?;
        Ok(id)
    }

    /// Restore a checkpoint by id: copy snapshots back over the originals and
    /// delete files the checkpoint recorded as absent. Returns files restored.
    pub fn restore(&self, id: &str) -> io::Result<usize> {
        let Some(meta) = self.list().into_iter().find(|m| m.id == id) else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("unknown checkpoint: {id}"),
            ));
        };
        let dir = self.root.join(id);
        let mut restored = 0;
        for (n, rel) in meta.files.iter().enumerate() {
            let target = PathBuf::from(rel);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            let snap = dir.join(format!("{n}.snap"));
            if snap.is_file() {
                fs::copy(&snap, &target)?;
                restored += 1;
            } else if dir.join(format!("{n}.absent")).is_file() {
                // Pre-mutation the file did not exist: undo = remove.
                match fs::remove_file(&target) {
                    Ok(()) => restored += 1,
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e),
                }
            }
        }
        Ok(restored)
    }

    /// All checkpoints, oldest first.
    pub fn list(&self) -> Vec<CheckpointMeta> {
        let Ok(text) = fs::read_to_string(self.root.join("index.jsonl")) else {
            return Vec::new();
        };
        text.lines().filter_map(parse_meta).collect()
    }
}

/// Minimal reader for the four-field manifest line.
fn parse_meta(line: &str) -> Option<CheckpointMeta> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    Some(CheckpointMeta {
        id: json_str_field(line, "id")?,
        label: json_str_field(line, "label").unwrap_or_default(),
        ts: json_str_field(line, "ts").unwrap_or_default(),
        files: json_str_array_field(line, "files").unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("caddis-ck-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn snapshot_mutate_restore_is_byte_identical() {
        let dir = tmp("roundtrip");
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("file.txt");
        fs::write(&target, b"BEFORE").unwrap();
        let store = CheckpointStore::open(&dir.join("store")).unwrap();
        let id = store
            .snapshot("pre-write", std::slice::from_ref(&target))
            .unwrap();

        fs::write(&target, b"AFTER - MUTATED").unwrap(); // the mutation
        assert_eq!(fs::read(&target).unwrap(), b"AFTER - MUTATED");

        let n = store.restore(&id).unwrap();
        assert_eq!(n, 1);
        assert_eq!(fs::read(&target).unwrap(), b"BEFORE", "mutation undone");
    }

    #[test]
    fn absent_file_creation_is_undone_by_restore() {
        let dir = tmp("absent");
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("new.txt"); // does NOT exist yet
        let store = CheckpointStore::open(&dir.join("store")).unwrap();
        let id = store
            .snapshot("pre-create", std::slice::from_ref(&target))
            .unwrap();

        fs::write(&target, b"CREATED BY MUTATION").unwrap();
        store.restore(&id).unwrap();
        assert!(!target.exists(), "created file removed by restore");
    }

    #[test]
    fn unknown_id_errors_and_list_roundtrips() {
        let dir = tmp("meta");
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("a.txt");
        fs::write(&target, b"x").unwrap();
        let store = CheckpointStore::open(&dir.join("store")).unwrap();
        let id = store.snapshot("label-here", &[target]).unwrap();
        let list = store.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, id);
        assert_eq!(list[0].label, "label-here");
        assert_eq!(list[0].files.len(), 1);
        assert!(store.restore("ckpt-nonexistent").is_err());
    }
}
