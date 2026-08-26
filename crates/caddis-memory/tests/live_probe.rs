//! live_probe.rs — P2 live golden probe against the REAL machine index.
//!
//! Ignored by default (hermetic-suite law): the unit suite proves logic with
//! FakeRunner; these tests prove the CONTRACT against today's real `qmd
//! status` output — the strict parser must accept reality, or the organ is
//! Red before it ships. Run explicitly:
//!
//! ```text
//! cargo test -p caddis-memory --test live_probe -- --ignored --nocapture
//! ```
//!
//! Test 1 is read-only against qmd. Test 2 exercises the organ's first-run
//! registry behavior at the Q6 default home (`~/.config/caddis/
//! collections.json`): load-missing → seed from the live snapshot → atomic
//! save → reload → every live collection present and private-by-default.
//! Re-runs are idempotent (seed adds only what is missing).

use caddis_memory::exec::RealRunner;
use caddis_memory::recall::MemoryConfig;
use caddis_memory::refresh::{probe, RefreshConfig};
use caddis_memory::registry::{Registry, QMD_OWNER};

fn live_cfg() -> RefreshConfig {
    RefreshConfig::new(MemoryConfig::detect())
}

#[test]
#[ignore = "live: spawns the real qmd CLI against the machine-global index"]
fn live_status_probe_parses_real_index() {
    let cfg = live_cfg();
    let mut runner = RealRunner;
    let snap = probe(&mut runner, &cfg)
        .expect("live golden probe: strict parser must accept today's real `qmd status`");
    println!("live snapshot: {snap:?}");
    assert!(
        !snap.collections.is_empty(),
        "machine-global index reports its collections"
    );
    assert!(snap.total_docs > 0, "machine-global index has docs");
}

#[test]
#[ignore = "live: reads the real index and writes the organ-owned registry file"]
fn live_registry_seed_roundtrip() {
    let cfg = live_cfg();
    let mut runner = RealRunner;
    let snap = probe(&mut runner, &cfg).expect("probe first");
    assert!(!snap.collections.is_empty());

    let path = Registry::default_path();
    let mut reg = Registry::load(&path).expect("registry loads or is first-run empty");
    let added = reg.seed_from_status(&snap.collections);
    reg.save().expect("atomic registry save");
    println!(
        "registry {} at {}: +{added} seeded, {} total",
        reg.entries().len(),
        path.display(),
        reg.entries().len()
    );

    let mut reloaded = Registry::load(&path).expect("registry reloads from disk");
    for c in &snap.collections {
        let entry = reloaded.get(&c.name);
        assert_eq!(entry.owner, QMD_OWNER, "seeded owner for {}", c.name);
        assert!(!entry.public, "Q6 law: seeded collections are private");
    }
    // Idempotence: seeding the same snapshot again adds nothing.
    let again = reloaded.seed_from_status(&snap.collections);
    assert_eq!(again, 0, "second seed of the same snapshot is a no-op");
}
