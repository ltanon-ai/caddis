//! bootstrap.rs — slice (b): the registry is the ONLY roots source for
//! remember() (quorum 2026-08-26, Q4 slicing, verdict §adopted amendments).
//!
//! What "wired" means here — no hand-typed root lists anywhere:
//! - [`bootstrap`] creates the two Q3 write targets
//!   (`~/.omp/sergeant/state/memory/`, `~/.omp/sergeant/state/briefs/`),
//!   registers them in the organ-owned registry with their sandbox roots,
//!   enforces the I5+ pairwise non-overlap law, and saves atomically.
//!   Idempotent: a second run creates nothing, changes nothing, reports zero.
//! - [`RememberConfig::from_registry`] derives the sandbox root set from
//!   exactly those registry entries that carry a root — the same file that
//!   records collection ownership (Q6) governs the write sandbox. One source
//!   of truth, so an unregistered directory can never become a write target
//!   by accident of a stale config literal.
//!
//! Stored roots are CLEAN absolute forms (`std::path::absolute`, no `\\?\`
//! verbatim prefix): the sandbox canonicalizes BOTH sides at compare time
//! (writer.rs `assert_sandbox`), so the registry stays human-readable
//! without weakening the prefix-match leg.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::registry::{CollectionEntry, Registry, RegistryError};
use crate::writer::RememberConfig;

/// Q3 v1 write target names (verbatim from the verdict).
pub const SERGEANT_STATE: &str = "sergeant-state";
pub const SERGEANT_BRIEFS: &str = "sergeant-briefs";
/// Owner stamped on organ-created entries (they are ours, not qmd-seeded).
pub const SERGEANT_OWNER: &str = "sergeant";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootstrapReport {
    /// Directories that did not exist and were created.
    pub created_dirs: Vec<PathBuf>,
    /// Entry names added or changed (empty on an idempotent re-run).
    pub upserted: Vec<String>,
}

/// `~/.omp/sergeant/state` — the Q3 target tree root. `None` only when no
/// home resolves at all (the registry's own default-path law).
pub fn sergeant_state_dir() -> Option<PathBuf> {
    crate::refresh::home_dir().map(|h| h.join(".omp").join("sergeant").join("state"))
}

fn target_entry(
    name: &str,
    root: &Path,
    created: &mut Vec<PathBuf>,
) -> Result<CollectionEntry, RegistryError> {
    if !root.is_dir() {
        fs::create_dir_all(root).map_err(|e| RegistryError::Io(e.to_string()))?;
        created.push(root.to_path_buf());
    }
    let clean = std::path::absolute(root)
        .map_err(|e| RegistryError::Io(format!("absolutize {}: {e}", root.display())))?;
    Ok(CollectionEntry {
        name: name.to_string(),
        public: false,
        owner: SERGEANT_OWNER.to_string(),
        root: Some(clean),
    })
}

/// Register the two Q3 write targets in `reg`. Creates missing directories,
/// upserts entries (reporting only real changes), enforces the I5+
/// non-overlap law, saves atomically. Errors leave the registry file
/// untouched (save is last; upserts are in-memory until then).
pub fn bootstrap(reg: &mut Registry, state_dir: &Path) -> Result<BootstrapReport, RegistryError> {
    let mut created = Vec::new();
    let mut upserted = Vec::new();
    for (name, leaf) in [(SERGEANT_STATE, "memory"), (SERGEANT_BRIEFS, "briefs")] {
        let entry = target_entry(name, &state_dir.join(leaf), &mut created)?;
        if reg.get(name) != entry {
            reg.upsert(entry);
            upserted.push(name.to_string());
        }
    }
    reg.validate_roots()?;
    reg.save()?;
    Ok(BootstrapReport { created_dirs: created, upserted })
}

impl RememberConfig {
    /// The slice-(b) wiring: sandbox roots are exactly the registry entries
    /// that carry one. Entries without a root are indexed foreign ground
    /// (Q6 law) — never write targets.
    pub fn from_registry(
        reg: &Registry,
        warden_launcher: Vec<String>,
        warden_timeout: Duration,
        steal_age_floor: Duration,
    ) -> RememberConfig {
        RememberConfig {
            warden_launcher,
            warden_timeout,
            roots: reg.entries().iter().filter_map(|e| e.root.clone()).collect(),
            steal_age_floor,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp_home(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("caddis-bootstrap-{}-{}", std::process::id(), tag));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn bootstrap_creates_and_registers() {
        let home = tmp_home("create");
        let reg_path = home.join("collections.json");
        let mut reg = Registry::load(&reg_path).unwrap();
        assert!(reg.entries().is_empty(), "first run loads empty");

        let rep = bootstrap(&mut reg, &home.join("state")).unwrap();
        assert_eq!(rep.created_dirs.len(), 2, "both target dirs created");
        assert_eq!(rep.upserted, vec![SERGEANT_STATE, SERGEANT_BRIEFS], "both entries registered (loop order)");

        let reloaded = Registry::load(&reg_path).unwrap();
        for name in [SERGEANT_STATE, SERGEANT_BRIEFS] {
            let e = reloaded.get(name);
            assert_eq!(e.owner, SERGEANT_OWNER);
            assert!(!e.public, "Q6: write targets are private");
            let root = e.root.expect("target carries its sandbox root");
            assert!(root.is_absolute());
            assert!(root.is_dir(), "registered root exists on disk");
        }
        reloaded.validate_roots().expect("sibling roots do not overlap");
    }

    #[test]
    fn bootstrap_is_idempotent() {
        let home = tmp_home("idem");
        let mut reg = Registry::load(&home.join("collections.json")).unwrap();
        bootstrap(&mut reg, &home.join("state")).unwrap();
        let again = bootstrap(&mut reg, &home.join("state")).unwrap();
        assert!(again.created_dirs.is_empty(), "nothing created twice");
        assert!(again.upserted.is_empty(), "nothing upserted twice");
    }

    #[test]
    fn from_registry_picks_only_rooted_entries() {
        let home = tmp_home("roots");
        let mut reg = Registry::load(&home.join("collections.json")).unwrap();
        bootstrap(&mut reg, &home.join("state")).unwrap();
        // Foreign indexed ground: registered WITHOUT a root (Q6 shape).
        reg.upsert(CollectionEntry {
            name: "catchall".into(),
            public: false,
            owner: "qmd".into(),
            root: None,
        });

        let cfg = RememberConfig::from_registry(&reg, vec!["warden".into()], Duration::from_secs(5), Duration::from_secs(60));
        assert_eq!(cfg.roots.len(), 2, "only the rooted write targets");
        assert!(cfg.roots.iter().all(|r| r.is_absolute()));
    }
}
