//! registry_drift.rs — THE RATCHET that keeps two registries from rotting apart.
//!
//! The estate's law corpus (`jit_checks/laws.json`) names checks by ID. The
//! callable behind each ID lives in Rust, here. That is deliberate — data must
//! never become executable — but it means the SAME FACT is bookkept in two
//! places, and two bookkeepings of one fact drift apart. This test is the only
//! reason that arrangement is survivable: it converts the drift from silent into
//! RED.
//!
//! WHAT IT DOES NOT PROVE, stated here so nobody reads more into a green than it
//! carries: it proves the Rust registry ANSWERS to every ID the corpus names. It
//! does NOT prove the Rust check and the Python check agree on any given command
//! — they are independent implementations, and the Rust ones match by hand what
//! the Python ones match by regex. ID coverage is what is ratcheted; semantic
//! parity is not, and claiming it would be the unearned "verified" this estate
//! treats as its costliest failure.
//!
//! THE CORPUS IS SHARED AND OTHER LANES WRITE TO IT. A law added elsewhere that
//! names a check this crate lacks turns this test red without anyone touching
//! this repo. That is the mechanism working, not a flake: the warden has just
//! been told, mechanically, that it is now out of date.

use std::path::PathBuf;

/// Where the shared law corpus lives, overridable so the ratchet can be pointed
/// at a fixture.
fn corpus_path() -> PathBuf {
    // Precedence: an explicit pointer, then the LIVE shared corpus on this
    // machine (other lanes write to it — that drift-turning-red is the
    // mechanism working), then the checked-in ids-only snapshot so the
    // ratchet runs — and can never silently skip — on any machine (CI, a
    // stranger's clone). The snapshot carries CHECK IDS ONLY: no law text,
    // no patterns, nothing private.
    if let Ok(p) = std::env::var("CADDIS_LAWS_JSON") {
        return PathBuf::from(p);
    }
    let live = {
        let home = std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home)
            .join(".claude")
            .join("hooks")
            .join("jit_checks")
            .join("laws.json")
    };
    if live.exists() {
        return live;
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
        .join("laws-corpus.json")
}

/// Every `"check": "<id>"` value in the corpus.
///
/// Scanned rather than parsed: this crate carries ZERO third-party dependencies
/// (a stated property of the repo, not an accident), so there is no serde here.
/// The corpus is machine-generated with stable formatting, and a scan that finds
/// FEWER ids than exist would weaken the ratchet silently — which is why the
/// caller asserts a non-zero count before trusting the result.
fn check_ids(corpus: &str) -> Vec<String> {
    let needle = "\"check\":";
    let mut out = Vec::new();
    for (idx, _) in corpus.match_indices(needle) {
        let rest = &corpus[idx + needle.len()..];
        let start = match rest.find('"') {
            Some(s) => s + 1,
            None => continue,
        };
        let tail = &rest[start..];
        match tail.find('"') {
            Some(end) => out.push(tail[..end].to_string()),
            None => continue,
        }
    }
    out.sort();
    out.dedup();
    out
}

#[test]
fn every_check_a_law_names_is_implemented_in_the_rust_registry() {
    let path = corpus_path();
    // THE PREMISE IS A CHECKED CLAIM, never an assumption (lesson #813). An
    // unreadable corpus means NOTHING WAS MEASURED, and a ratchet that goes
    // green when its input is missing is `assert(true == true)` wearing a
    // ratchet's clothes.
    let corpus = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "could not read the shared law corpus at {}: {e}. This test did not \
             pass and did not fail — it could not run. Point it at a corpus with \
             CADDIS_LAWS_JSON=<path>.",
            path.display()
        )
    });

    let ids = check_ids(&corpus);
    assert!(
        !ids.is_empty(),
        "the corpus at {} yielded ZERO check ids. Either the corpus changed shape \
         or the scanner is broken; both make this ratchet vacuous, which is worse \
         than absent.",
        path.display()
    );

    let missing: Vec<&String> = ids
        .iter()
        .filter(|id| !caddis_warden::checks::is_registered(id))
        .collect();

    assert!(
        missing.is_empty(),
        "{} law(s) in {} name a check the Rust registry does not implement: {:?}.\n\
         A law naming an unimplemented check is a law that silently does nothing. \
         Implement it in crates/caddis-warden/src/checks/, or the corpus is \
         promising an enforcement that does not exist.",
        missing.len(),
        path.display(),
        missing
    );
}
