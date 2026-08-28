//! winprobe.rs — I2+ holder-liveness probe (quorum 2026-08-26, amendment I2+).
//!
//! F2 (live-proven 2026-08-26): a naive `OpenProcess` SUCCESS proves NOTHING —
//! a terminated process still opens. Death is only proven by
//! `GetExitCodeProcess != STILL_ACTIVE (259)`; liveness is 259. The
//! generational leg (gemini's amendment) catches PID reuse: if the holder
//! process was CREATED after the lock file was written, the pid now belongs
//! to a different process — the lock's pid is not the holder anymore.
//!
//! Failure doctrine is FAIL-CLOSED for stealing: anything this module cannot
//! prove (API failure, unsupported platform) reports `Alive`-shaped `None`,
//! so the caller's steal rule (age AND proven-dead) refuses to fire. The
//! verdict's age cap alone bounds how long an unprovable holder can block.
//!
//! Std-only law: raw `extern "system"` declarations, no windows-sys dependency.

/// What the probe proved about the pid named in a lock file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Probe {
    /// Exit code != 259 — the process object is a corpse (F2).
    Dead,
    /// Exit code == 259 and the process predates the lock: a live holder.
    Alive,
    /// Exit code == 259 but the process was created AFTER the lock was
    /// written — the pid was reused; the original holder is gone.
    Reused,
}

/// Slack (seconds) comparing process creation time to the lock's `created_ts`:
/// both clocks are wall-clock seconds from different sources; two seconds of
/// tolerance keeps a boundary write from reading as a reuse.
const REUSE_SLACK_SECS: u64 = 2;

/// Windows FILETIME epoch offset to Unix seconds (100 ns ticks between
/// 1601-01-01 and 1970-01-01).
const FILETIME_UNIX_EPOCH: i64 = 116_444_736_000_000_000;

#[cfg(windows)]
mod ffi {
    use std::os::raw::{c_int, c_ulong, c_void};

    pub const PROCESS_QUERY_LIMITED_INFORMATION: c_ulong = 0x1000;
    pub const STILL_ACTIVE: c_ulong = 259;

    #[link(name = "kernel32")]
    extern "system" {
        pub fn OpenProcess(
            dwDesiredAccess: c_ulong,
            bInheritHandle: c_int,
            dwProcessId: c_ulong,
        ) -> *mut c_void;
        pub fn GetExitCodeProcess(hProcess: *mut c_void, lpExitCode: *mut c_ulong) -> c_int;
        pub fn GetProcessTimes(
            hProcess: *mut c_void,
            lpCreationTime: *mut i64,
            lpExitTime: *mut i64,
            lpKernelTime: *mut i64,
            lpUserTime: *mut i64,
        ) -> c_int;
        pub fn CloseHandle(hObject: *mut c_void) -> c_int;
    }
}

/// Probe the pid a lock file names. `None` = could not prove anything
/// (unsupported platform, OpenProcess failure, API error) — callers MUST
/// treat `None` as "cannot steal" (fail-closed), never as dead.
///
/// Known residual (documented, accepted): a process that genuinely exited
/// WITH exit code 259 reads as live; the steal rule's age cap bounds the
/// block. Everything provable is proven here.
#[cfg(windows)]
pub fn holder_state(pid: u32, lock_created_unix: u64) -> Option<Probe> {
    use std::os::raw::c_ulong;

    use ffi::*;

    if pid == 0 {
        return None; // pid 0 is the idle process / never a lock holder
    }
    // SAFETY: plain Win32 handle query on a raw pid; no borrows cross the
    // boundary. Handle is closed on every path that opens it.
    let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if h.is_null() {
        // OpenProcess refused: access-denied means the process exists;
        // invalid-parameter means it is gone. We do not call GetLastError
        // (extra FFI surface) — refusing to prove is the safe answer.
        return None;
    }
    let mut code: c_ulong = 0;
    let ok = unsafe { GetExitCodeProcess(h, &mut code) } != 0;
    let probe = if !ok {
        None
    } else if code != STILL_ACTIVE {
        Some(Probe::Dead)
    } else {
        // 259: live or a corpse that exited with 259. The generational leg
        // settles pid reuse; a real live holder predates the lock.
        let mut creation: i64 = 0;
        let (mut exit_t, mut kernel, mut user): (i64, i64, i64) = (0, 0, 0);
        let times_ok =
            unsafe { GetProcessTimes(h, &mut creation, &mut exit_t, &mut kernel, &mut user) } != 0;
        if !times_ok {
            None
        } else {
            let created_unix = (creation - FILETIME_UNIX_EPOCH) / 10_000_000;
            if created_unix > (lock_created_unix as i64) + (REUSE_SLACK_SECS as i64) {
                Some(Probe::Reused)
            } else {
                Some(Probe::Alive)
            }
        }
    };
    unsafe { CloseHandle(h) };
    probe
}

/// Non-Windows: the F2 probe is Windows law; without it death is unprovable,
/// so the answer is `None` and the steal rule stays closed.
#[cfg(not(windows))]
pub fn holder_state(_pid: u32, _lock_created_unix: u64) -> Option<Probe> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn own_pid_is_alive() {
        // Our own process: 259 and created long before "now" — a lock we
        // hold reads back as a live holder, not a reuse.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert_eq!(holder_state(std::process::id(), now), Some(Probe::Alive));
    }

    #[cfg(windows)]
    #[test]
    fn own_pid_with_ancient_lock_reads_reused() {
        // The generational leg: this process was created AFTER the lock
        // claims to have been written (epoch 0) → the pid is not the holder.
        assert_eq!(holder_state(std::process::id(), 0), Some(Probe::Reused));
    }

    #[cfg(windows)]
    #[test]
    fn reaped_child_reads_dead() {
        use std::process::Command;
        // A child that ran and exited: exit code 0 != 259 → Dead (F2 shape).
        let mut child = Command::new("cmd")
            .args(["/c", "exit 0"])
            .spawn()
            .expect("spawn cmd");
        let pid = child.id();
        child.wait().expect("wait child");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert_eq!(holder_state(pid, now), Some(Probe::Dead));
    }

    #[test]
    fn pid_zero_is_unprovable() {
        assert_eq!(holder_state(0, 0), None);
    }
}
