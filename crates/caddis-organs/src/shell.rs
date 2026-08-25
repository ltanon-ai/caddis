//! shell.rs — run one operator-configured command string under a hard
//! deadline, split out of watchdog.rs under the 280-line law.
//!
//! The seam is a different KIND of problem: everything else in the watchdog
//! is a state machine over probe results, while this is process handling —
//! spawn, poll, kill — with a platform fork that has nothing to do with
//! failure accounting. It is also the piece most likely to be reused by a
//! second organ that needs to run something with a timeout.
//!
//! SAFETY/TRUST: `cmd` is operator-configured (schedules/settings), never
//! model or channel output — the same contract the TinyAGI source carries.

use std::io;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// Run a shell command string under a hard deadline. Pure std: spawn via the
/// platform shell, poll `try_wait`, kill the child when the deadline passes.
/// Exit status 0 within the deadline = true.
pub fn run_with_timeout(cmd: &str, timeout: Duration) -> bool {
    let mut child = match spawn_shell(cmd) {
        Ok(c) => c,
        Err(_) => return false,
    };
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) => {
                if Instant::now() >= deadline {
                    // The deadline passed, so the probe has already FAILED.
                    // kill/wait only reap the child; if reaping fails there is
                    // no other outcome to report.
                    let _ = child.kill(); // swallow: best-effort-cleanup
                    let _ = child.wait(); // swallow: best-effort-cleanup
                    return false;
                }
                thread::sleep(Duration::from_millis(20));
            }
            Err(_) => {
                // try_wait already failed, so the probe counts as failed; this
                // kill is a reap attempt, not a check.
                let _ = child.kill(); // swallow: best-effort-cleanup
                return false;
            }
        }
    }
}

// ⛔ `#[cfg(windows)]`, NOT `if cfg!(windows)`. The first version of this
// function used the macro, which is a RUNTIME boolean: both arms still get
// type-checked on every target, so `use std::os::windows::process::CommandExt`
// was compiled on Linux too and failed with E0433 "could not find `windows` in
// `os`" plus E0599 on `raw_arg`. It passed here because this is a Windows box;
// the public CI builds on ubuntu and macos as well, so it would have red-ed two
// of three legs on the first tagged release. Attribute = conditional
// COMPILATION; macro = a bool. Only the attribute removes the other arm.
#[cfg(windows)]
fn spawn_shell(cmd: &str) -> io::Result<std::process::Child> {
    // raw_arg passes the command string VERBATIM on the Windows command
    // line. Plain `.arg(cmd)` would escape inner quotes as \" which
    // cmd.exe cannot parse — every quoted path inside an operator
    // command (echo x > "C: b\m.flag") would die with a syntax error.
    use std::os::windows::process::CommandExt;
    let mut c = Command::new("cmd");
    c.arg("/C");
    c.raw_arg(cmd);
    c.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
}

#[cfg(not(windows))]
fn spawn_shell(cmd: &str) -> io::Result<std::process::Child> {
    Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_kills_hung_probe() {
        // The hang must be spelled per-platform. The original fixture was
        // Windows-only `ping -n 10 127.0.0.1`, which was harmless while the
        // crate did not compile off Windows — and the cfg fix above made this
        // test START RUNNING on ubuntu and macos, where GNU ping reads `-n` as
        // "numeric" and then chokes on two destinations. It would have exited
        // immediately, the assertion would still have passed, and the test
        // would have proved a malformed command fails fast rather than that a
        // HUNG one is killed at the deadline: green, for the wrong reason, on
        // two of three CI platforms.
        //
        // `cfg!` is correct HERE (unlike in spawn_shell): both arms are just
        // strings, so nothing platform-specific is type-checked on the wrong
        // target.
        let hang = if cfg!(windows) {
            "ping -n 10 127.0.0.1"
        } else {
            "sleep 10"
        };
        let ok = run_with_timeout(hang, Duration::from_millis(150));
        assert!(!ok, "hung command must count as a failed probe");
    }

    #[test]
    fn command_zero_exit_is_healthy() {
        assert!(run_with_timeout("exit 0", Duration::from_secs(5)));
        assert!(!run_with_timeout("exit 3", Duration::from_secs(5)));
    }
}
