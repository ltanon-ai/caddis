//! exec.rs — run one qmd CLI invocation under a hard deadline, with the
//! CI-scrubbed environment.
//!
//! Env sanitization is a HARD SPEC item (CONVENING.md fact-check row 3): the
//! harness environment carries `CI=true`, which makes qmd REFUSE LLM
//! operations ("LLM operations are disabled in CI", llm.js:1058) — the deep
//! `query` lane dies in 0.2s. The ruling: sanitize unconditionally on EVERY
//! call, so lane choice never breaks. We strip the inherited env of `CI`
//! (removal, not clearing — a full `env_clear` would drop Windows system vars
//! like SystemRoot and break the spawn itself).
//!
//! Timeout mechanics follow the shell.rs precedent (spawn → poll `try_wait` →
//! kill at the deadline), extended to CAPTURE output: two reader threads drain
//! stdout/stderr (each capped) so a chatty child can never deadlock on a full
//! pipe. Killing the child closes its pipe handles; the readers see EOF and
//! finish, so joining after the wait is always safe.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// Cap per output stream (8 MiB). qmd result payloads are kilobytes; anything
/// bigger than this is not a recall answer and gets truncated honestly.
const STREAM_CAP: usize = 8 * 1024 * 1024;

/// Keys stripped from the inherited environment before every qmd spawn.
/// `CI` is the proven offender; it is removed even when set to a falsy value
/// so lane behavior can never depend on harness shape.
pub const STRIPPED_ENV_KEYS: &[&str] = &["CI"];

/// Pure decision function: does this inherited env key get stripped?
/// Exposed so the sanitize law has a directly testable seam.
pub fn should_strip(key: &str) -> bool {
    STRIPPED_ENV_KEYS.contains(&key)
}

/// One subprocess job: how to launch qmd, where to run it, how long it may run.
#[derive(Debug, Clone)]
pub struct Job {
    /// Launcher = program + prefix args (e.g. `["node", "C:/…/bin/qmd"]`).
    /// A Vec keeps the crate free of shell-quoting law entirely (raw args,
    /// no cmd.exe round trip).
    pub launcher: Vec<String>,
    /// qmd subcommand args, e.g. `["search", "golden needle", "--json"]`.
    pub args: Vec<String>,
    /// Working directory: qmd resolves a project-local `.qmd` index from cwd,
    /// which is how sandbox tests avoid the live 131 MB index.
    pub workdir: Option<PathBuf>,
    pub timeout: Duration,
    /// Stdin payload, written in full before the deadline is judged. `None`
    /// keeps stdin null (the qmd surface). The warden frame path is the
    /// first user: its request is a length-prefixed byte frame, not argv —
    /// argv would round-trip every byte through shell-quoting law this
    /// crate refuses to own.
    pub stdin_data: Option<Vec<u8>>,
}

/// What one run provably did. Duration is wall time; `timed_out` true means
/// the child was killed at the deadline (fail-closed upstream).
#[derive(Debug, Clone)]
pub struct Outcome {
    pub code: Option<i32>,
    pub timed_out: bool,
    pub stdout: String,
    pub stderr: String,
    pub duration: Duration,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

/// The exec seam. Real = process spawn; Fake (test-only) = canned behavior.
pub trait Runner {
    fn run(&mut self, job: &Job) -> Outcome;
}

#[derive(Default)]
pub struct RealRunner;

impl Runner for RealRunner {
    fn run(&mut self, job: &Job) -> Outcome {
        let started = Instant::now();
        let (program, prefix) = match job.launcher.split_first() {
            Some(x) => x,
            None => {
                return Outcome {
                    code: None,
                    timed_out: false,
                    stdout: String::new(),
                    stderr: "empty launcher".into(),
                    duration: started.elapsed(),
                    stdout_truncated: false,
                    stderr_truncated: false,
                }
            }
        };
        let mut cmd = Command::new(program);
        cmd.args(prefix).args(&job.args);
        if let Some(dir) = &job.workdir {
            cmd.current_dir(dir);
        }
        for key in STRIPPED_ENV_KEYS {
            cmd.env_remove(key);
        }
        if job.stdin_data.is_some() {
            cmd.stdin(Stdio::piped());
        } else {
            cmd.stdin(Stdio::null());
        }
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                return Outcome {
                    code: None,
                    timed_out: false,
                    stdout: String::new(),
                    stderr: format!("spawn failed: {e}"),
                    duration: started.elapsed(),
                    stdout_truncated: false,
                    stderr_truncated: false,
                }
            }
        };

        let out_handle = child.stdout.take();
        let err_handle = child.stderr.take();
        let stdin_sink = child.stdin.take();
        let t_out = thread::spawn(move || read_capped(out_handle));
        let t_err = thread::spawn(move || read_capped(err_handle));
        // Stdin writer: its own thread, exactly like the readers, so a child
        // that exits without draining stdin (warden fail-closed paths) can
        // never deadlock the pump — a broken pipe here is the child's verdict,
        // not ours to judge.
        let t_in = job.stdin_data.clone().map(|data| {
            thread::spawn(move || {
                if let Some(mut w) = stdin_sink {
                    let _ = w.write_all(&data);
                    // Drop closes the pipe; the child sees EOF and replies.
                }
            })
        });

        let deadline = started + job.timeout;
        let mut timed_out = false;
        let code = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status.code(),
                Ok(None) => {
                    if Instant::now() >= deadline {
                        timed_out = true;
                        let _ = child.kill();
                        let _ = child.wait(); // reap; closes the pipes for the readers
                        break None;
                    }
                    thread::sleep(Duration::from_millis(5));
                }
                Err(_) => break None,
            }
        };

        if let Some(t_in) = t_in {
            let _ = t_in.join();
        }
        // Dropping a still-undrained stdin handle (writer thread raced the
        // child's exit) is fine: the pipe is already closed on our side.
        let (stdout, stdout_truncated) = t_out.join().unwrap_or_default();
        let (stderr, stderr_truncated) = t_err.join().unwrap_or_default();
        Outcome {
            code,
            timed_out,
            stdout,
            stderr,
            duration: started.elapsed(),
            stdout_truncated,
            stderr_truncated,
        }
    }
}

fn read_capped<R: Read>(mut r: Option<R>) -> (String, bool) {
    let mut buf = Vec::new();
    let mut truncated = false;
    if let Some(r) = r.as_mut() {
        let mut chunk = [0u8; 16384];
        loop {
            match r.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    if buf.len() + n > STREAM_CAP {
                        let room = STREAM_CAP.saturating_sub(buf.len());
                        buf.extend_from_slice(&chunk[..room]);
                        truncated = true;
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                }
                Err(_) => break,
            }
        }
    }
    (String::from_utf8_lossy(&buf).into_owned(), truncated)
}

#[cfg(test)]
pub mod testing {
    //! FakeRunner — canned Outcomes keyed by a matcher on the qmd subcommand.
    //! Lives under cfg(test) so it never ships in the public surface.
    use super::{Job, Outcome, Runner};
    use std::collections::VecDeque;
    use std::time::Duration;

    #[derive(Default)]
    pub struct FakeRunner {
        pub calls: Vec<Vec<String>>,
        /// Full jobs in call order (launcher + args + stdin). Remember
        /// tests must tell a warden spawn (launcher=[bin], args=[],
        /// stdin=frame) from a qmd spawn (args=["update"]) — args alone
        /// cannot express that.
        pub jobs: Vec<Job>,
        canned: Vec<(String, Outcome)>,
        pub default: Option<Outcome>,
        seq: VecDeque<Outcome>,
    }

    impl FakeRunner {
        /// Match by first qmd arg ("search" / "query" / "get").
        pub fn on(&mut self, subcommand: &str, out: Outcome) -> &mut Self {
            self.canned.push((subcommand.to_string(), out));
            self
        }

        /// Queue one Outcome for the NEXT run call regardless of
        /// subcommand — refresh scripts its steps in order
        /// (status → update → embed → status), which first-arg matching
        /// cannot express. Consumed before canned/default matching.
        pub fn then(&mut self, out: Outcome) -> &mut Self {
            self.seq.push_back(out);
            self
        }

        pub fn ok_json(_sub: &str, json: &str) -> Outcome {
            Outcome {
                code: Some(0),
                timed_out: false,
                stdout: json.to_string(),
                stderr: String::new(),
                duration: Duration::from_millis(200),
                stdout_truncated: false,
                stderr_truncated: false,
            }
        }
    }

    impl Runner for FakeRunner {
        fn run(&mut self, job: &Job) -> Outcome {
            self.calls.push(job.args.clone());
            self.jobs.push(job.clone());
            if let Some(out) = self.seq.pop_front() {
                return out;
            }
            if let Some(first) = job.args.first() {
                for (sub, out) in &self.canned {
                    if sub == first {
                        return out.clone();
                    }
                }
            }
            self.default.clone().unwrap_or(Outcome {
                code: Some(1),
                timed_out: false,
                stdout: String::new(),
                stderr: "no canned outcome".into(),
                duration: Duration::from_millis(1),
                stdout_truncated: false,
                stderr_truncated: false,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job_for(program: &str, args: &[&str], ms: u64) -> Job {
        Job {
            launcher: vec![program.to_string()],
            args: args.iter().map(|s| s.to_string()).collect(),
            workdir: None,
            timeout: Duration::from_millis(ms),
            stdin_data: None,
        }
    }

    #[test]
    fn sanitize_law_strips_ci_only() {
        assert!(should_strip("CI"));
        assert!(!should_strip("PATH"));
        assert!(!should_strip("SystemRoot"));
        assert!(!should_strip("ci"), "case-sensitive: only CI is the gate");
    }

    #[test]
    fn child_env_has_ci_removed() {
        // `node -e` prints whether CI is defined, proving env_remove reached
        // the child. Node is required for this crate's real lane anyway; if
        // node is absent the run reports spawn failure and the assert catches
        // the missing fixture honestly.
        let out = RealRunner.run(&job_for(
            "node",
            &["-e", "process.stdout.write(process.env.CI === undefined ? \"CLEAN\" : \"DIRTY\")"],
            15_000,
        ));
        assert_eq!(out.code, Some(0), "node probe must run: {}", out.stderr);
        assert_eq!(out.stdout.trim(), "CLEAN");
    }

    #[test]
    fn timeout_kills_and_reports() {
        // Cross-platform hang via node (same binary the real lane needs).
        let out = RealRunner.run(&job_for("node", &["-e", "setTimeout(()=>{}, 30000)"], 300));
        assert!(out.timed_out);
        assert_eq!(out.code, None);
        assert!(out.duration < Duration::from_secs(5), "must die near the 300ms deadline");
    }

    #[test]
    fn nonzero_exit_is_reported_not_panicked() {
        let out = RealRunner.run(&job_for("node", &["-e", "process.exit(3)"], 15_000));
        assert_eq!(out.code, Some(3));
        assert!(!out.timed_out);
    }

    #[test]
    fn spawn_failure_is_an_outcome() {
        let out = RealRunner.run(&job_for("definitely-not-a-program-4c7d1", &["x"], 5_000));
        assert_eq!(out.code, None);
        assert!(!out.timed_out);
        assert!(out.stderr.starts_with("spawn failed:"));
    }

    #[test]
    fn stdout_stream_cap_truncates_honestly() {
        // 9 MiB of output against the 8 MiB cap: truncated=true, len capped.
        let out = RealRunner.run(&job_for(
            "node",
            &["-e", "process.stdout.write('x'.repeat(9*1024*1024))"],
            30_000,
        ));
        assert!(out.stdout_truncated);
        assert_eq!(out.stdout.len(), 8 * 1024 * 1024);
    }

    #[test]
    fn workdir_is_honored() {
        let dir = std::env::temp_dir();
        let out = RealRunner.run(&Job {
            launcher: vec!["node".into()],
            args: vec!["-e".into(), "process.stdout.write(process.cwd())".into()],
            workdir: Some(dir.clone()),
            stdin_data: None,
            timeout: Duration::from_secs(15),
        });
        assert_eq!(out.code, Some(0));
        let seen = std::path::Path::new(out.stdout.trim());
        assert_eq!(seen, dir.as_path());
    }
}
