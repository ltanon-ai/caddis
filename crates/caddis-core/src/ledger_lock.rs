//! ledger_lock.rs — mutual exclusion for the ledger's read-max/append pair,
//! split from ledger.rs under the 280-line law (CARD-0108).
//!
//! The seam is real rather than cosmetic: ledger.rs is about the RECORD, this
//! file is about the EXCLUSION, and the exclusion is the half with the
//! platform-specific trap in it.

use std::fs;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

/// An advisory lock over one ledger, held for the read-max/append pair.
///
/// `create_new` is the whole mechanism: it maps to `O_EXCL` / `CREATE_NEW`, so
/// exactly one racer can create the file and the rest see `AlreadyExists`. That
/// keeps the crate's ZERO-DEPENDENCY property — a real `flock`/`LockFileEx`
/// would cost either a dependency or `unsafe` FFI, and the kernel's own lock
/// buys nothing here that this does not.
///
/// A HELD LOCK MUST NEVER BLOCK WORK FOREVER. The warden is spawned per tool
/// call, so a process killed mid-append would otherwise wedge every future call
/// behind a file nobody owns. A lock older than `STALE` is therefore broken and
/// the wait is bounded; if the wait expires the append proceeds ANYWAY, because
/// a slightly-wrong seq is a smaller failure than a conscience that stops
/// recording. Recording fails open; the JUDGEMENT still fails closed.
pub(crate) struct Lock {
    path: PathBuf,
    /// Did WE create the lock file? False when `acquire` gave up after `WAIT`
    /// and proceeded without exclusion. A lock we never took is a lock we must
    /// never release.
    owned: bool,
    /// What we wrote INTO the file when we created it.
    ///
    /// Ownership at acquire-time is not ownership at drop-time. A stale-breaker
    /// may have unlinked our file and created its own while we were slow — and
    /// then our `Drop` would unlink THEIRS, handing the ledger to a third racer
    /// while they still believed they held it. `owned` alone closed that only
    /// on the timeout path; the token closes it on the stale path too, which is
    /// the same cascade arriving through the other door.
    token: String,
}

impl Lock {
    const STALE: std::time::Duration = std::time::Duration::from_secs(10);
    const WAIT: std::time::Duration = std::time::Duration::from_secs(5);

    pub(crate) fn acquire(ledger: &Path) -> std::io::Result<Self> {
        let path = ledger.with_extension("lock");
        // pid + nanoseconds: unique per acquisition on this machine without a
        // dependency. pids recycle, so the clock is what keeps two lifetimes of
        // one pid apart.
        let token = format!(
            "{}:{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );
        let start = std::time::Instant::now();
        loop {
            match OpenOptions::new().create_new(true).write(true).open(&path) {
                Ok(mut f) => {
                    // Best effort: an unwritten token only costs us the right
                    // to release our OWN lock, which the stale-breaker then
                    // reclaims. It can never release someone else's.
                    use std::io::Write;
                    // swallow: best-effort-telemetry
                    let _ = f.write_all(token.as_bytes());
                    return Ok(Self {
                        path,
                        owned: true,
                        token,
                    });
                }
                // ⚠ TWO ERROR KINDS MEAN "HELD", AND MISSING THE SECOND FAILS
                // EVERY APPEND ON WINDOWS. Unlinking a name another handle has
                // open leaves it DELETE-PENDING, and `CREATE_NEW` against a
                // delete-pending name returns ERROR_ACCESS_DENIED (5), not
                // ERROR_FILE_EXISTS — so the releasing racer makes the acquiring
                // racer look permanently forbidden. Measured: treating only
                // AlreadyExists as held failed with `PermissionDenied` under 8
                // concurrent writers.
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::AlreadyExists | std::io::ErrorKind::PermissionDenied
                    ) =>
                {
                    // STALE and TIMED-OUT are different facts and were fused
                    // into one branch. Stale means nobody owns the file, so
                    // breaking it is correct. Timed out means someone may own
                    // it and simply be slow — breaking THAT file is not ours to
                    // do, and the old code did it anyway on the way out.
                    if Self::is_stale(&path) {
                        // A racer may have removed it already, which is the
                        // outcome we wanted anyway.
                        // swallow: best-effort-cleanup
                        let _ = fs::remove_file(&path);
                        continue;
                    }
                    if start.elapsed() > Self::WAIT {
                        // The documented fail-open: record without exclusion
                        // rather than stop recording. `owned: false` is the
                        // whole fix — we hold no lock, so we release none.
                        return Ok(Self {
                            path,
                            owned: false,
                            token,
                        });
                    }
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
                Err(e) => return Err(e),
            }
        }
    }

    fn is_stale(path: &Path) -> bool {
        fs::metadata(path)
            .and_then(|m| m.modified())
            .and_then(|t| {
                std::time::SystemTime::now()
                    .duration_since(t)
                    .map_err(|_| std::io::Error::other("clock moved backwards"))
            })
            .map(|age| age > Self::STALE)
            .unwrap_or(false)
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        if !self.owned {
            // We never created this file — `acquire` timed out and proceeded
            // without exclusion. Deleting it here would release a lock still
            // held by whoever DID create it.
            return;
        }
        // We DID create one, but is the file on disk still ours? If a
        // stale-breaker replaced it, the bytes belong to another holder and
        // unlinking them would release a lock we do not hold.
        //
        // RELEASING FAILS CLOSED. `unwrap_or(true)` means an unreadable file is
        // treated as NOT ours: the cost of declining is that our own lock file
        // survives until the stale-breaker reclaims it one STALE later, while
        // the cost of guessing wrong the other way is releasing a lock somebody
        // is actively holding. Recording fails open; releasing must not.
        if fs::read_to_string(&self.path)
            .map(|s| s != self.token)
            .unwrap_or(true)
        {
            return;
        }
        // ⚠ HONEST SCOPE — A RESIDUAL RACE REMAINS AND IS NOT CLOSED HERE.
        // The confirm above and the unlink below are two separate operations on
        // a path, so a stale-breaker that lands between them loses its file
        // exactly as it did before the token existed. Closing that needs an
        // atomic compare-and-delete, which no portable filesystem API offers —
        // it would cost a dependency or `unsafe` FFI, and this crate has
        // neither by design.
        //
        // What bounds it: the window is a single confirm-to-unlink gap, and
        // reaching it requires our own lock to have aged past STALE while we
        // still held it. The damage is one spurious release, which the
        // stale-breaker recovers from within one STALE — the same bounded cost
        // the documented fail-open already accepts. This comment exists because
        // a mechanism that states its limits is the entire point of this
        // project, and the previous version of this file over-claimed.
        // The stale-breaker in `acquire` is the backstop if this never runs, so
        // a failed unlink costs at most one STALE wait, never a wedged ledger.
        // swallow: best-effort-cleanup
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(test)]
#[path = "ledger_lock_tests.rs"]
mod tests;
