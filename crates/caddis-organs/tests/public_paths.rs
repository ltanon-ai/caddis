//! public_paths.rs — pins the crate's PUBLIC module paths from OUTSIDE the crate.
//!
//! RED-first for a real defect: the wave-1 split re-exported `hop::Hop` from
//! `canary` as `Hop as CanaryHop` to dodge a name collision the split itself
//! introduced, while the comment above it claimed `canary::Hop` still
//! resolved. Every in-crate test imports through `use super::*`, which picks
//! up the module's PRIVATE `use crate::hop::Hop` — so the whole suite stayed
//! green while the published path was gone. An integration test cannot see
//! those private imports, which is exactly why it catches this.
//!
//! Renaming a public symbol to resolve a split's naming conflict is a
//! forbidden move; this file is the mechanism that makes it fail loudly.

use caddis_organs::canary;
use caddis_organs::watchdog;

#[test]
fn canary_public_paths_survive_the_split() {
    // Each line fails to COMPILE if the path stops resolving.
    let _: fn(&[canary::Hop]) -> (canary::HopStatus, u32, u32) = canary::aggregate;
    let _: fn() -> canary::HostHooks = canary::HostHooks::none;
    let _ = canary::HopStatus::Ok;
    fn _takes_result(_: &canary::CanaryResult) {}
}

#[test]
fn watchdog_public_paths_survive_the_split() {
    let _: fn(&std::path::Path) -> Vec<watchdog::Blocker> = watchdog::list_open_blockers;
    let _: fn(&str, std::time::Duration) -> bool = watchdog::run_with_timeout;
}
