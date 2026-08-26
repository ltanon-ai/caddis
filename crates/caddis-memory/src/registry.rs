//! registry.rs — the organ-owned collection registry (Q6, ratified 3/3 as
//! amended).
//!
//! Fact-check row 6 (CONVENING.md): qmd collections carry no metadata flags,
//! so the public/private boundary lives HERE — a small JSON file the organ
//! owns and reads, `{ "collections": { "<name>": { "public": bool, "owner":
//! string } } }` (groq amendment 5 schema).
//!
//! Law (Q6 + I5 public-clean): a collection ABSENT from the registry reads
//! as `public: false`, owner `unclaimed` — the fail-safe default, never the
//! fail-open one. The registry is metadata ABOUT qmd collections; it never
//! edits qmd itself.

use crate::json::{self, Value};
use crate::refresh::CollectionStatus;
use std::fs;
use std::path::{Path, PathBuf};

/// The law: unknown collections are private (Q6 default).
pub const DEFAULT_PUBLIC: bool = false;
/// Owner reported for collections absent from the registry.
pub const UNCLAIMED_OWNER: &str = "unclaimed";
/// Owner stamped on live collections that predate the organ (seeded from a
/// `qmd status` snapshot — they existed before any organ claim).
pub const QMD_OWNER: &str = "qmd";

#[derive(Debug, Clone, PartialEq)]
pub struct CollectionEntry {
    pub name: String,
    pub public: bool,
    pub owner: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RegistryDiff {
    /// Live in qmd but absent from the registry (public:false by law).
    pub unregistered: Vec<String>,
    /// Registered but not live in qmd — stale registration, honest residue.
    pub vanished: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RegistryError {
    Io(String),
    Schema { why: String, head: String },
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegistryError::Io(why) => write!(f, "registry io: {why}"),
            RegistryError::Schema { why, head } => write!(f, "registry schema ({why}): {head}"),
        }
    }
}

/// The registry. Entries are kept sorted by name so saves are canonical and
/// diffs are stable.
#[derive(Debug, Clone, PartialEq)]
pub struct Registry {
    path: PathBuf,
    entries: Vec<CollectionEntry>,
}

impl Registry {
    /// Load from disk. A missing file is an EMPTY registry (first run), not
    /// an error; an existing-but-unparseable file IS an error (fail-closed:
    /// a corrupt registry must surface, never silently reset).
    pub fn load(path: &Path) -> Result<Registry, RegistryError> {
        match fs::read_to_string(path) {
            Ok(text) => Registry::from_str(path, &text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Ok(Registry { path: path.to_path_buf(), entries: Vec::new() })
            }
            Err(e) => Err(RegistryError::Io(e.to_string())),
        }
    }

    fn from_str(path: &Path, text: &str) -> Result<Registry, RegistryError> {
        let head = |n: usize| text.get(..n).unwrap_or(text).replace('\n', "\\n");
        let schema = |why: &str| RegistryError::Schema { why: why.to_string(), head: head(120) };
        let root = json::parse(text).map_err(|e| schema(&format!("json: {e:?}")))?;
        let cols = root.get("collections").ok_or_else(|| schema("collections key missing"))?;
        let pairs = cols.as_obj().ok_or_else(|| schema("collections must be an object"))?;
        let mut entries = Vec::new();
        for (name, ev) in pairs {
            let public = ev
                .get("public")
                .and_then(Value::as_bool)
                .ok_or_else(|| schema(&format!("entry {name}: public must be a bool")))?;
            let owner = ev
                .get("owner")
                .and_then(Value::as_str)
                .ok_or_else(|| schema(&format!("entry {name}: owner must be a string")))?;
            if owner.trim().is_empty() {
                return Err(schema(&format!("entry {name}: owner must not be empty")));
            }
            entries.push(CollectionEntry { name: name.clone(), public, owner: owner.to_string() });
        }
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(Registry { path: path.to_path_buf(), entries })
    }

    /// Atomic save (tmp + rename). The organ never leaves a half-written
    /// registry behind.
    pub fn save(&self) -> Result<(), RegistryError> {
        let cols: Vec<(String, Value)> = self
            .entries
            .iter()
            .map(|e| {
                (
                    e.name.clone(),
                    Value::Obj(vec![
                        ("public".to_string(), Value::Bool(e.public)),
                        ("owner".to_string(), Value::Str(e.owner.clone())),
                    ]),
                )
            })
            .collect();
        let root = Value::Obj(vec![("collections".to_string(), Value::Obj(cols))]);
        let text = json::to_string(&root) + "\n";
        // First run: the organ's home (~/.config/caddis/) may not exist yet —
        // a missing parent is ours to create, not an error (proven live
        // 2026-08-26: save failed with os error 3 on a fresh machine).
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|e| RegistryError::Io(e.to_string()))?;
        }
        let mut tmp = self.path.as_os_str().to_os_string();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        fs::write(&tmp, text).map_err(|e| RegistryError::Io(e.to_string()))?;
        fs::rename(&tmp, &self.path).map_err(|e| RegistryError::Io(e.to_string()))?;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn entries(&self) -> &[CollectionEntry] {
        &self.entries
    }

    /// The Q6 law in one call: absent entries read as
    /// `{ public: false, owner: "unclaimed" }`.
    pub fn get(&self, name: &str) -> CollectionEntry {
        self.entries
            .iter()
            .find(|e| e.name == name)
            .cloned()
            .unwrap_or_else(|| CollectionEntry {
                name: name.to_string(),
                public: DEFAULT_PUBLIC,
                owner: UNCLAIMED_OWNER.to_string(),
            })
    }

    pub fn is_public(&self, name: &str) -> bool {
        self.get(name).public
    }

    /// Insert or replace by name; keeps entries sorted.
    pub fn upsert(&mut self, entry: CollectionEntry) {
        match self.entries.iter_mut().find(|e| e.name == entry.name) {
            Some(slot) => *slot = entry,
            None => {
                self.entries.push(entry);
                self.entries.sort_by(|a, b| a.name.cmp(&b.name));
            }
        }
    }

    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| e.name != name);
        before != self.entries.len()
    }

    /// Register every live collection not yet known, as
    /// `{ public: false, owner: "qmd" }`. Returns how many were added
    /// (0 = registry already covered the snapshot).
    pub fn seed_from_status(&mut self, live: &[CollectionStatus]) -> usize {
        let mut added = 0;
        for c in live {
            if !self.entries.iter().any(|e| e.name == c.name) {
                self.entries.push(CollectionEntry {
                    name: c.name.clone(),
                    public: DEFAULT_PUBLIC,
                    owner: QMD_OWNER.to_string(),
                });
                added += 1;
            }
        }
        self.entries.sort_by(|a, b| a.name.cmp(&b.name));
        added
    }

    /// Compare the registry against a live snapshot: what qmd has that the
    /// registry does not (unregistered), and what the registry claims that
    /// qmd no longer has (vanished).
    pub fn diff(&self, live: &[CollectionStatus]) -> RegistryDiff {
        let unregistered = live
            .iter()
            .filter(|c| !self.entries.iter().any(|e| e.name == c.name))
            .map(|c| c.name.clone())
            .collect();
        let vanished = self
            .entries
            .iter()
            .filter(|e| !live.iter().any(|c| c.name == e.name))
            .map(|e| e.name.clone())
            .collect();
        RegistryDiff { unregistered, vanished }
    }

    /// Default home: `~/.config/caddis/collections.json` (beside qmd's own
    /// `~/.config/qmd`). Falls back to the temp dir only when no home resolves.
    pub fn default_path() -> PathBuf {
        crate::refresh::home_dir()
            .map(|h| h.join(".config").join("caddis").join("collections.json"))
            .unwrap_or_else(|| std::env::temp_dir().join("caddis-collections.json"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::refresh::CollectionStatus;

    fn tmp_path(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("caddis-registry-{}-{tag}.json", std::process::id()));
        let _ = fs::remove_file(&p);
        p
    }

    fn cs(name: &str) -> CollectionStatus {
        CollectionStatus { name: name.to_string(), files: 1, updated_ago_secs: None }
    }

    #[test]
    fn missing_file_is_empty_registry() {
        let path = tmp_path("absent");
        let reg = Registry::load(&path).unwrap();
        assert!(reg.entries().is_empty());
    }

    #[test]
    fn round_trip_preserves_entries_and_leaves_no_tmp() {
        let path = tmp_path("roundtrip");
        let mut reg = Registry::load(&path).unwrap();
        reg.upsert(CollectionEntry { name: "memory".into(), public: false, owner: "qmd".into() });
        reg.upsert(CollectionEntry { name: "showr".into(), public: true, owner: "operator".into() });
        reg.save().unwrap();

        let tmp = {
            let mut s = path.as_os_str().to_os_string();
            s.push(".tmp");
            PathBuf::from(s)
        };
        assert!(!tmp.exists(), "atomic save must not leave tmp behind");

        let back = Registry::load(&path).unwrap();
        assert_eq!(back.entries(), reg.entries());
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn absent_entry_reads_private_unclaimed() {
        let reg = Registry { path: tmp_path("law"), entries: Vec::new() };
        let e = reg.get("anything");
        assert!(!e.public);
        assert_eq!(e.owner, "unclaimed");
        assert!(!reg.is_public("anything"));
    }

    #[test]
    fn schema_is_strict() {
        // root not an object
        assert!(Registry::from_str(Path::new("x"), "[]").is_err());
        // collections key missing
        assert!(Registry::from_str(Path::new("x"), "{}").is_err());
        // collections not an object
        assert!(Registry::from_str(Path::new("x"), r#"{"collections":[]}"#).is_err());
        // entry missing owner
        assert!(Registry::from_str(Path::new("x"), r#"{"collections":{"a":{"public":false}}}"#).is_err());
        // entry public wrong type
        assert!(
            Registry::from_str(Path::new("x"), r#"{"collections":{"a":{"public":"no","owner":"x"}}}"#).is_err()
        );
        // owner empty
        assert!(Registry::from_str(Path::new("x"), r#"{"collections":{"a":{"public":false,"owner":"  "}}}"#).is_err());
        // trailing garbage
        assert!(Registry::from_str(Path::new("x"), r#"{"collections":{}} extra"#).is_err());
    }

    #[test]
    fn load_parses_sorted_entries() {
        let text = r#"{"collections":{"zeta":{"public":true,"owner":"op"},"alpha":{"public":false,"owner":"qmd"}}}"#;
        let reg = Registry::from_str(Path::new("x"), text).unwrap();
        let names: Vec<&str> = reg.entries().iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "zeta"]);
        assert!(reg.is_public("zeta"));
        assert!(!reg.is_public("alpha"));
    }

    #[test]
    fn upsert_replaces_by_name() {
        let mut reg = Registry { path: tmp_path("upsert"), entries: Vec::new() };
        reg.upsert(CollectionEntry { name: "a".into(), public: false, owner: "qmd".into() });
        reg.upsert(CollectionEntry { name: "a".into(), public: true, owner: "op".into() });
        assert_eq!(reg.entries().len(), 1);
        assert!(reg.is_public("a"));
        assert_eq!(reg.get("a").owner, "op");
    }

    #[test]
    fn seed_adds_only_unknown_collections() {
        let mut reg = Registry { path: tmp_path("seed"), entries: Vec::new() };
        let live = [cs("memory"), cs("showr")];
        assert_eq!(reg.seed_from_status(&live), 2);
        assert_eq!(reg.seed_from_status(&live), 0, "second seed is a no-op");
        assert_eq!(reg.get("memory").owner, "qmd");
        assert!(!reg.get("memory").public);
        // an organ-claimed entry is never overwritten by seeding
        reg.upsert(CollectionEntry { name: "sergeant-state".into(), public: false, owner: "caddis".into() });
        let live2 = [cs("memory"), cs("sergeant-state")];
        assert_eq!(reg.seed_from_status(&live2), 0);
        assert_eq!(reg.get("sergeant-state").owner, "caddis");
    }

    #[test]
    fn diff_sees_both_directions() {
        let mut reg = Registry { path: tmp_path("diff"), entries: Vec::new() };
        reg.upsert(CollectionEntry { name: "ghost".into(), public: false, owner: "qmd".into() });
        reg.upsert(CollectionEntry { name: "memory".into(), public: false, owner: "qmd".into() });
        let d = reg.diff(&[cs("memory"), cs("new-kid")]);
        assert_eq!(d.unregistered, vec!["new-kid"]);
        assert_eq!(d.vanished, vec!["ghost"]);
    }

    #[test]
    fn remove_reports_whether_it_removed() {
        let mut reg = Registry { path: tmp_path("rm"), entries: Vec::new() };
        reg.upsert(CollectionEntry { name: "a".into(), public: false, owner: "qmd".into() });
        assert!(reg.remove("a"));
        assert!(!reg.remove("a"));
    }
}
