//! ledger_lock_tests.rs — the ownership properties of the ledger lock,
//! in their own file because ledger_lock.rs reached the 280-line cap.
//! Wired with #[path], the pattern caddis-warden already uses for
//! allowlist_tests.rs and attest_tests.rs.

use super::*;

/// A lock that gave up after `WAIT` must not delete the incumbent's file.
///
/// The old code fused "stale" and "timed out" into one branch and unlinked
/// on the way out of BOTH, then returned a `Lock` whose `Drop` unlinked
/// again. So a slow-but-alive holder had its lock removed twice by a racer
/// that never owned it — and the next acquirer would create a fresh lock
/// only for the timed-out racer's `Drop` to release THAT one too.
///
/// Deliberately spends `WAIT`: the property only exists on the timeout
/// path, and a test that dodged the wait would not be testing it.
#[test]
fn a_timed_out_lock_does_not_release_the_incumbents_file() {
    let dir = std::env::temp_dir().join(format!("caddis-lock-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let ledger = dir.join("t.jsonl");
    let lockfile = ledger.with_extension("lock");

    // An incumbent holds the lock, and it is FRESH — not stale, so the
    // stale-breaker must not touch it.
    std::fs::write(&lockfile, b"incumbent").unwrap();

    // ⛔ KEEP IT FRESH FOR THE WHOLE WAIT, or this test is FLAKY UNDER LOAD.
    // `acquire` checks staleness BEFORE the timeout, so a scheduling stall
    // that pushes the file's age past STALE (10s) during the WAIT (5s) loop
    // sends it down the stale-BREAK branch instead: it unlinks, retries,
    // succeeds, and returns `owned: true`. The assertion below then fails
    // for a reason that has nothing to do with the property under test.
    //
    // NOT hypothetical — a Zylė handoff audit ran the suite on a loaded
    // machine and got exactly that, one failure that stopped the whole
    // workspace run at 80 tests while an unloaded run of the same head gave
    // 492/0. A test whose verdict depends on machine load is not a test.
    let keep = lockfile.clone();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let halt = stop.clone();
    // ONE failed refresh is expected and harmless. ALL of them failing — an
    // inaccessible temp dir, a full disk — means the incumbent silently aged
    // past STALE and this run measured the flake again instead of the
    // property. Counting successes is what tells those two apart, because
    // both of them surface as the SAME assertion failure below.
    let refreshed = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let ticks = refreshed.clone();
    let toucher = std::thread::spawn(move || {
        while !halt.load(std::sync::atomic::Ordering::Relaxed) {
            // A refresh that loses a race with `acquire`'s own open is fine;
            // the next tick 400ms later re-freshens it well inside STALE.
            if std::fs::write(&keep, b"incumbent").is_ok() {
                ticks.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            std::thread::sleep(std::time::Duration::from_millis(400));
        }
    });

    let timed_out = Lock::acquire(&ledger).expect("acquire fails open, never errors");
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    // A panicked refresher voids this test's precondition, so it is a failure
    // with a name rather than a discarded Result.
    toucher.join().expect("the lock refresher thread panicked");
    // Checked BEFORE the property assertions: if the refresher never ran, the
    // cause of any failure below is this, not the lock semantics.
    assert!(
        refreshed.load(std::sync::atomic::Ordering::Relaxed) > 0,
        "the refresher never completed a single write, so the incumbent was \
         not kept fresh — this run measured the STALE race, not the timeout path"
    );
    assert!(
        !timed_out.owned,
        "acquire must report that it did NOT take the lock"
    );
    assert!(
        lockfile.exists(),
        "the incumbent's lock must survive another racer timing out"
    );

    drop(timed_out);
    assert!(
        lockfile.exists(),
        "dropping a lock we never held must not release the incumbent's"
    );

    // Cleanup stays best-effort ON PURPOSE, unlike the two swallows above:
    // on Windows a temp dir can refuse removal while a handle is still
    // closing, and failing the test on that would ADD the flakiness this
    // whole test exists to remove. A review flagged all three swallows
    // together; only these last ones are correct.
    std::fs::remove_dir_all(&dir).ok();
}

/// A stale-breaker took the lock from us; our `Drop` must leave THEIRS alone.
///
/// The cascade this closes is the one `owned` did not reach. We create the
/// file and genuinely own it, so `owned` is true — then a breaker decides we
/// are stale, unlinks ours and creates its own. Ownership at acquire-time is
/// not ownership at drop-time, and unlinking on the strength of the former
/// hands the ledger to a third racer while the breaker still believes it
/// holds the lock. Fast by construction: no wait is involved, only the
/// substitution.
#[test]
fn dropping_a_lock_a_breaker_replaced_does_not_release_theirs() {
    let dir = std::env::temp_dir().join(format!("caddis-lock-steal-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let ledger = dir.join("t.jsonl");
    let lockfile = ledger.with_extension("lock");

    let ours = Lock::acquire(&ledger).unwrap();
    assert!(ours.owned, "we created this one");

    // A stale-breaker unlinked ours and put its own lock in place.
    std::fs::write(&lockfile, b"another-holder").unwrap();

    drop(ours);
    assert!(
        lockfile.exists(),
        "our Drop must not release a lock that is no longer ours"
    );
    assert_eq!(
        std::fs::read_to_string(&lockfile).unwrap(),
        "another-holder",
        "and it must be untouched, not rewritten"
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// The ordinary path still takes and releases its own lock.
#[test]
fn an_owned_lock_is_created_and_released() {
    let dir = std::env::temp_dir().join(format!("caddis-lock-own-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let ledger = dir.join("t.jsonl");
    let lockfile = ledger.with_extension("lock");

    let held = Lock::acquire(&ledger).unwrap();
    assert!(held.owned, "a lock we created is ours");
    assert!(lockfile.exists(), "the lock file exists while held");

    drop(held);
    assert!(!lockfile.exists(), "dropping our own lock releases it");

    std::fs::remove_dir_all(&dir).ok();
}
