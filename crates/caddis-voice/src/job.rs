//! job.rs — the Job Objects harness (P1 hard requirement, QQ2+QQ3 ratified:
//! "children die with the Rust parent").
//!
//! Two distinct supervision tools, deliberately separated because their
//! Drop semantics are opposite:
//!
//! - [`DeadManSwitch`] — the organ BOOT path. Creates a kill-on-close job,
//!   assigns the ORGAN'S OWN process, and INTENTIONALLY LEAKS the job
//!   handle. From that moment every child the organ spawns lands in the job
//!   automatically (no per-child assignment needed), and when the organ
//!   process dies — crash, kill, shutdown — the kernel closes its handles,
//!   the job's last handle closes, and KILL_ON_JOB_CLOSE terminates every
//!   engine child still running. No orphaned whisper/piper processes, ever.
//!   Leaking the handle is not an accident to fix: the leak IS the switch.
//!
//! - [`ChildScope`] — the TEST/supervision path. A job whose handle IS
//!   closed on Drop, killing exactly the processes assigned to it. Used by
//!   the test that proves kill-on-close mechanics, and by P2's killable
//!   engine supervision.
//!
//! Non-Windows: honest unsupported errors. This organ's supervision law is a
//! Windows law (Job Objects); pretending on another platform would fake the
//! "children die" invariant that the whole design leans on.

#[cfg(windows)]
use crate::platform;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobErr(pub String);

impl fmt::Display for JobErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "job harness: {}", self.0)
    }
}

/// Proof that the dead-man switch is armed. Carries no handle — there is
/// nothing to close, by design.
#[derive(Debug, Clone)]
pub struct DeadManSwitch {
    /// Wall-clock proof line for health/telemetry.
    pub installed_utc_ms: u128,
}

impl DeadManSwitch {
    /// Arm the switch for the CURRENT process. Idempotent-by- stacking:
    /// each call creates a NEW anonymous job and assigns this process to it
    /// (nested jobs are legal on Windows 8+); the honest pattern is to call
    /// it exactly once at organ boot.
    #[cfg(windows)]
    pub fn install() -> Result<Self, JobErr> {
        // SAFETY: kernel handle creation with null/anonymous parameters; the
        // job handle is intentionally never closed (see module docs).
        let job = unsafe { platform::create_kill_on_close_job().map_err(JobErr)? };
        let me = unsafe { platform::current_process_handle() };
        // The pseudo-handle from GetCurrentProcess is valid for assignment;
        // AssignProcessToJobObject does not take ownership of it.
        if !unsafe { platform::assign_to_job(job, me) } {
            // Refused (e.g. already in a job that forbids nesting — pre-Win8
            // shape): the switch would be a LIE. Fail loudly, leak nothing.
            unsafe { platform::close_handle(job) };
            return Err(JobErr("AssignProcessToJobObject(self) refused".into()));
        }
        // No forget/cleanup of `job`: it is a raw handle, not RAII — going
        // out of scope keeps the kernel handle open forever, which IS the
        // dead-man contract (closed only by process death).
        Ok(DeadManSwitch { installed_utc_ms: unix_ms() })
    }

    #[cfg(not(windows))]
    pub fn install() -> Result<Self, JobErr> {
        Err(JobErr("Job Objects are Windows-only; refusing to fake the dead-man switch".into()))
    }
}

fn unix_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// A job whose handle closes on Drop — killing the assigned processes NOW.
/// Tests prove the kernel honors KILL_ON_JOB_CLOSE; P2 reuses it for
/// supervised engine kills.
#[cfg(windows)]
pub struct ChildScope {
    job: *mut std::os::raw::c_void,
}

#[cfg(windows)]
impl ChildScope {
    pub fn create() -> Result<Self, JobErr> {
        // SAFETY: anonymous job creation; handle stored, closed in Drop.
        let job = unsafe { platform::create_kill_on_close_job().map_err(JobErr)? };
        Ok(Self { job })
    }

    /// Assign a process by pid. The handle opened here is closed before
    /// returning — the job keeps its own reference.
    pub fn assign_pid(&self, pid: u32) -> Result<(), JobErr> {
        // SAFETY: OpenProcess on a raw pid with the two rights assignment
        // needs; null result is the caller's verdict to make.
        let proc = unsafe { platform::open_process_for_assignment(pid) };
        if proc.is_null() {
            return Err(JobErr(format!("OpenProcess({pid}) refused — gone or protected")));
        }
        // SAFETY: valid job + process handle pair.
        let ok = unsafe { platform::assign_to_job(self.job, proc) };
        // SAFETY: our own opened handle; the job holds its reference.
        unsafe { platform::close_handle(proc) };
        if !ok {
            return Err(JobErr(format!("AssignProcessToJobObject({pid}) refused")));
        }
        Ok(())
    }

    /// True while the handle is live (i.e. before Drop).
    pub fn is_armed(&self) -> bool {
        !self.job.is_null()
    }
}

#[cfg(windows)]
impl Drop for ChildScope {
    fn drop(&mut self) {
        // SAFETY: our own job handle; closing it is the kill signal.
        unsafe { platform::close_handle(self.job) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use std::time::{Duration, Instant};

    #[cfg(windows)]
    #[test]
    fn dead_man_switch_installs_for_this_process() {
        // install() must succeed in the test runner (no hostile parent job).
        // We do NOT test its kill semantics here: dropping requires a handle
        // we deliberately leaked — ChildScope below proves the kernel law.
        let sw = DeadManSwitch::install().expect("install in an un-nested process");
        assert!(sw.installed_utc_ms > 1_700_000_000_000);
    }

    #[cfg(windows)]
    #[test]
    fn closing_the_job_handle_kills_the_child() {
        let scope = ChildScope::create().expect("create kill-on-close job");
        // A child that would otherwise outlive this test by seconds.
        let mut child = Command::new("python")
            .args(["-c", "import time; time.sleep(8)"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn sleeper");
        scope.assign_pid(child.id()).expect("assign sleeper to job");
        assert!(scope.is_armed());

        // Drop closes the job handle → the kernel must kill the sleeper well
        // inside its 8s lifetime. 4s margin: generous for slow CI, still a
        // real kill proof (a broken harness leaves the child alive at 8s).
        drop(scope);
        let deadline = Instant::now() + Duration::from_secs(4);
        loop {
            match child.try_wait().expect("try_wait on live child") {
                Some(_) => return, // killed (or exited) — kernel honored it
                None if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
                None => panic!("child survived 4s after the job handle closed — KILL_ON_JOB_CLOSE not in effect"),
            }
        }
    }
}
