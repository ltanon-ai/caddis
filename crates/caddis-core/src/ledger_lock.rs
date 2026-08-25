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
}

impl Lock {
    const STALE: std::time::Duration = std::time::Duration::from_secs(10);
    const WAIT: std::time::Duration = std::time::Duration::from_secs(5);

    pub(crate) fn acquire(ledger: &Path) -> std::io::Result<Self> {
        let path = ledger.with_extension("lock");
        let start = std::time::Instant::now();
        loop {
            match OpenOptions::new().create_new(true).write(true).open(&path) {
                Ok(_) => return Ok(Self { path }),
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
                    if Self::is_stale(&path) || start.elapsed() > Self::WAIT {
                        // A racer may have removed it already, which is the
                        // outcome we wanted anyway.
                        // swallow: best-effort-cleanup
                        let _ = fs::remove_file(&path);
                        if start.elapsed() > Self::WAIT {
                            return Ok(Self { path });
                        }
                        continue;
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
        // The stale-breaker in `acquire` is the backstop if this never runs, so
        // a failed unlink costs at most one STALE wait, never a wedged ledger.
        // swallow: best-effort-cleanup
        let _ = fs::remove_file(&self.path);
    }
}
