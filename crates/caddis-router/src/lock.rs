//! lock.rs — R6 mutual exclusion for the decision ledger's read-max/append
//! pair (quorum-caddis-router R6: "decision ledger append = SINGLE-WRITER
//! authority or OS file lock + fsync; concurrent appends forbidden by
//! construction").
//!
//! Same O_EXCL mechanism as caddis-core's `ledger_lock` (create_new maps to
//! `CREATE_NEW`, exactly one racer wins, zero dependencies), same Windows
//! trap handled (`CREATE_NEW` on a DELETE-PENDING name returns
//! `PermissionDenied`, not `AlreadyExists`), same token-guarded release.
//!
//! ONE LAW DIFFERS, ON PURPOSE. caddis-core's lock FAILS OPEN after its wait
//! ("a slightly-wrong seq is a smaller failure than a conscience that stops
//! recording") — there a lost row is a lost moral record. Here the append
//! FAILS CLOSED ([`LockErr::Busy`]): R6 forbids concurrent appends BY
//! CONSTRUCTION, and a routing decision row is RE-DERIVABLE (`route()` is a
//! pure function; the caller retries) while a forked ledger is not. The
//! conscience may record without exclusion; the router may not.

use std::fs;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// A lock older than this is nobody's — a process died holding it.
const STALE: Duration = Duration::from_secs(10);
#[derive(Debug)]
pub(crate) enum LockErr {
    /// Still held (and fresh) after waiting the whole budget. R6: the caller
    /// fails closed and retries; nobody appends without exclusion.
    Busy,
    Io(std::io::Error),
}

// io::Error is not PartialEq: compare by KIND — enough for the tests that
// assert WHICH law fired, never the OS message.
impl PartialEq for LockErr {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (LockErr::Busy, LockErr::Busy) => true,
            (LockErr::Io(a), LockErr::Io(b)) => a.kind() == b.kind(),
            _ => false,
        }
    }
}

#[derive(Debug)]
pub(crate) struct Lock {
    path: PathBuf,
    /// Did WE create the lock file? A lock we never took is one we must
    /// never release.
    owned: bool,
    /// What we wrote INTO the file when we created it. Ownership at
    /// acquire-time is not ownership at drop-time: a stale-breaker may have
    /// unlinked our file and created its own while we were slow — releasing
    /// THAT one would hand the ledger to a third racer.
    token: String,
}

impl Lock {
    pub(crate) fn acquire(ledger: &Path, wait: Duration) -> Result<Self, LockErr> {
        let path = ledger.with_extension("lock");
        // pid + nanoseconds: unique per acquisition without a dependency.
        // Pids recycle; the clock keeps two lifetimes of one pid apart.
        let token = format!(
            "{}:{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );
        let start = Instant::now();
        loop {
            match OpenOptions::new().create_new(true).write(true).open(&path) {
                Ok(mut f) => {
                    use std::io::Write;
                    // Best effort: an unwritten token only costs us the right
                    // to release our OWN lock; it can never release another's.
                    // swallow: best-effort-telemetry
                    let _ = f.write_all(token.as_bytes());
                    return Ok(Self {
                        path,
                        owned: true,
                        token,
                    });
                }
                // ⚠ TWO ERROR KINDS MEAN "HELD" (measured on Windows, see
                // caddis-core ledger_lock): unlinking a name another handle
                // has open leaves it DELETE-PENDING, and CREATE_NEW against a
                // delete-pending name returns ERROR_ACCESS_DENIED, not
                // ERROR_FILE_EXISTS. Missing the second kind fails every
                // append on Windows.
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::AlreadyExists | std::io::ErrorKind::PermissionDenied
                    ) =>
                {
                    if Self::is_stale(&path) {
                        // Nobody owns a stale lock; breaking it is correct.
                        // A racer may have removed it already — the outcome
                        // we wanted anyway. swallow: best-effort-cleanup
                        let _ = fs::remove_file(&path);
                        continue;
                    }
                    if start.elapsed() > wait {
                        // R6 fail-closed path. Contrast: caddis-core proceeds
                        // here because the conscience must keep recording.
                        return Err(LockErr::Busy);
                    }
                    std::thread::sleep(Duration::from_millis(2));
                }
                Err(e) => return Err(LockErr::Io(e)),
            }
        }
    }

    fn is_stale(path: &Path) -> bool {
        fs::metadata(path)
            .and_then(|m| m.modified())
            .and_then(|t| {
                SystemTime::now()
                    .duration_since(t)
                    .map_err(|_| std::io::Error::other("clock moved backwards"))
            })
            .map(|age| age > STALE)
            .unwrap_or(false)
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        if !self.owned {
            return;
        }
        // RELEASING FAILS CLOSED: an unreadable or changed file is treated as
        // NOT ours. The cost of declining is our lock file surviving until
        // the stale-breaker reclaims it one STALE later; the cost of guessing
        // wrong is releasing a lock somebody actively holds.
        if fs::read_to_string(&self.path)
            .map(|s| s != self.token)
            .unwrap_or(true)
        {
            return;
        }
        // Same honest scope as caddis-core: the confirm-to-unlink gap is not
        // atomically closable without FFI; reaching it requires our own lock
        // to have aged past STALE while we still held it, and the damage is
        // one spurious release recovered within one STALE. The stale-breaker
        // in `acquire` is the backstop. swallow: best-effort-cleanup
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn held_lock_is_busy_then_releasable() {
        let dir = std::env::temp_dir().join(format!("rtr-lock-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let ledger = dir.join("decisions.jsonl");
        let _ = fs::remove_file(ledger.with_extension("lock"));

        let a = Lock::acquire(&ledger, Duration::from_millis(20)).unwrap();
        // Fresh foreign lock: busy after the (short) budget — fail closed.
        assert_eq!(
            Lock::acquire(&ledger, Duration::from_millis(20)).unwrap_err(),
            LockErr::Busy
        );
        drop(a);
        // Released: immediately re-acquirable, and again busy while held.
        let b = Lock::acquire(&ledger, Duration::from_millis(20)).unwrap();
        assert_eq!(
            Lock::acquire(&ledger, Duration::from_millis(20)).unwrap_err(),
            LockErr::Busy
        );
        drop(b);
        assert!(Lock::acquire(&ledger, Duration::from_millis(20)).is_ok());
        fs::remove_dir_all(&dir).ok();
    }
}
