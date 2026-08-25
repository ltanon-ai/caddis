//! watchdog.rs — the self-heal organ (wave 1).
//! Port of the TinyAGI watchdog job (adoption A3), harness-agnostic:
//! probe -> restart -> backoff -> blocker, std only.
//!
//! Law carried over from the source:
//! - after `max_failures` consecutive failed probes (post-restart) a BLOCKER
//!   is filed and the watchdog stops hammering until the blocker is resolved;
//! - a healthy probe resets the failure counter;
//! - never pointed at a shared daemon by default — only at subordinate
//!   services the operator explicitly lists (unset health_cmd -> report only);
//! - command strings are OPERATOR-CONFIGURED (same trust model as the source;
//!   they never originate from model/channel output).
//!
//! Blockers persist as one JSONL line per blocker in a host-owned file:
//! `{"source":"watchdog:<label>","reason":"...","ts":"..."}` — resolving is
//! deleting the line (the operator's act, or an automation with sanction).

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::blocker::{file_blocker, resolve_source};
use crate::util::iso8601_now;

// The blocker record and the shell runner moved out under the 280-line law
// (blocker.rs, shell.rs). They are re-exported HERE so every existing path —
// `watchdog::Blocker`, `watchdog::list_open_blockers`,
// `watchdog::run_with_timeout` — still resolves. A split that silently moves
// a public symbol is an API break wearing a refactor's clothes.
pub use crate::blocker::{list_open_blockers, Blocker};
pub use crate::shell::run_with_timeout;

pub const DEFAULT_MAX_FAILURES: u32 = 3;
pub const DEFAULT_PROBE_TIMEOUT_MS: u64 = 10_000;
pub const DEFAULT_RESTART_TIMEOUT_MS: u64 = 30_000;

/// What one probe cycle did.
#[derive(Debug, Clone, PartialEq)]
pub enum ProbeAction {
    /// Healthy — counter reset.
    Healthy,
    /// Probe failed, restart command attempted (flag = restart exit).
    RestartAttempted { restart_ok: bool },
    /// Probe failed, no restart command configured.
    FailedNoRestart,
    /// No health_cmd configured — reporting only.
    ReportOnly,
    /// An open blocker for this source exists — hammering is suspended.
    SkippedOpenBlocker,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProbeOutcome {
    pub action: ProbeAction,
    /// Set on the failure that crosses `max_failures`.
    pub blocker: Option<Blocker>,
    /// Consecutive failures after this cycle.
    pub failures: u32,
}

/// Watchdog for one subordinate service.
pub struct Watchdog {
    label: String,
    health_cmd: Option<String>,
    restart_cmd: Option<String>,
    probe_timeout: Duration,
    restart_timeout: Duration,
    max_failures: u32,
    failures: u32,
    blockers_path: PathBuf,
}

impl Watchdog {
    pub fn new(label: &str, blockers_path: &Path) -> Self {
        Watchdog {
            label: label.to_string(),
            health_cmd: None,
            restart_cmd: None,
            probe_timeout: Duration::from_millis(DEFAULT_PROBE_TIMEOUT_MS),
            restart_timeout: Duration::from_millis(DEFAULT_RESTART_TIMEOUT_MS),
            max_failures: DEFAULT_MAX_FAILURES,
            failures: 0,
            blockers_path: blockers_path.to_path_buf(),
        }
    }

    pub fn health_cmd(mut self, cmd: &str) -> Self {
        self.health_cmd = Some(cmd.to_string());
        self
    }

    pub fn restart_cmd(mut self, cmd: &str) -> Self {
        self.restart_cmd = Some(cmd.to_string());
        self
    }

    pub fn timeouts(mut self, probe_ms: u64, restart_ms: u64) -> Self {
        self.probe_timeout = Duration::from_millis(probe_ms);
        self.restart_timeout = Duration::from_millis(restart_ms);
        self
    }

    pub fn max_failures(mut self, n: u32) -> Self {
        self.max_failures = n.max(1);
        self
    }

    pub fn failures(&self) -> u32 {
        self.failures
    }

    /// One probe cycle (the scheduler/heartbeat calls this per tick).
    pub fn run_probe(&mut self) -> ProbeOutcome {
        let source = format!("watchdog:{}", self.label);
        if list_open_blockers(&self.blockers_path)
            .iter()
            .any(|b| b.source == source)
        {
            return ProbeOutcome {
                action: ProbeAction::SkippedOpenBlocker,
                blocker: None,
                failures: self.failures,
            };
        }
        let Some(health) = self.health_cmd.as_ref() else {
            return ProbeOutcome {
                action: ProbeAction::ReportOnly,
                blocker: None,
                failures: self.failures,
            };
        };
        if run_with_timeout(health, self.probe_timeout) {
            self.failures = 0;
            return ProbeOutcome {
                action: ProbeAction::Healthy,
                blocker: None,
                failures: 0,
            };
        }
        let restart_ok = self
            .restart_cmd
            .as_ref()
            .map(|c| run_with_timeout(c, self.restart_timeout))
            .unwrap_or(false);
        self.failures += 1;
        let crossed = self.failures >= self.max_failures;
        let blocker = if crossed {
            let b = Blocker {
                source: source.clone(),
                reason: format!(
                    "Watchdog '{}' failed health probe {}x after restart attempts",
                    self.label, self.failures
                ),
                ts: iso8601_now(),
            };
            match file_blocker(&self.blockers_path, &b) {
                Ok(()) => {
                    self.failures = 0; // stop hammering; the blocker owns the pause
                    Some(b)
                }
                Err(e) => {
                    // The blocker is the safety net: if IT cannot persist, keep
                    // the in-memory failure count so the next probe re-tries.
                    let shouting = Blocker {
                        reason: format!("{} (FILING FAILED: {})", b.reason, e),
                        ..b
                    };
                    self.failures = self.max_failures;
                    Some(shouting)
                }
            }
        } else {
            None
        };
        ProbeOutcome {
            action: if self.restart_cmd.is_some() {
                ProbeAction::RestartAttempted { restart_ok }
            } else {
                ProbeAction::FailedNoRestart
            },
            blocker,
            failures: self.failures,
        }
    }

    /// Resolve (delete) this watchdog's blockers — the operator's act.
    pub fn resolve_blockers(&self) -> io::Result<usize> {
        resolve_source(&self.blockers_path, &format!("watchdog:{}", self.label))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("caddis-wd-{name}-{}", std::process::id()));
        // swallow: best-effort-cleanup — stale temp dir from a previous run.
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn healthy_probe_resets_counter() {
        let dir = tmp("healthy");
        std::fs::create_dir_all(&dir).unwrap();
        let mut wd = Watchdog::new("svc", &dir.join("blockers.jsonl"))
            .health_cmd("exit 0")
            .max_failures(2);
        wd.failures = 1; // simulate a prior failure
        let out = wd.run_probe();
        assert_eq!(out.action, ProbeAction::Healthy);
        assert_eq!(out.failures, 0);
    }

    #[test]
    fn three_failures_file_blocker_then_skip_then_resolve() {
        let dir = tmp("blocker");
        std::fs::create_dir_all(&dir).unwrap();
        let blockers = dir.join("blockers.jsonl");
        let marker = dir.join("restarted.flag");
        let restart = format!("echo x > \"{}\"", marker.to_string_lossy());
        let mut wd = Watchdog::new("svc", &blockers)
            .health_cmd("exit 1")
            .restart_cmd(&restart);
        for i in 1..=2 {
            let out = wd.run_probe();
            assert!(
                matches!(out.action, ProbeAction::RestartAttempted { .. }),
                "iter {i}"
            );
            assert!(
                out.blocker.is_none(),
                "no blocker before max_failures (iter {i})"
            );
        }
        assert!(marker.is_file(), "restart command ran");
        let out = wd.run_probe();
        assert!(out.blocker.is_some(), "blocker filed at max_failures");
        let open = list_open_blockers(&blockers);
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].source, "watchdog:svc");
        // Hammering suspended while the blocker is open.
        let out = wd.run_probe();
        assert_eq!(out.action, ProbeAction::SkippedOpenBlocker);
        // Operator resolves; probing resumes and now the service is healthy.
        assert_eq!(wd.resolve_blockers().unwrap(), 1);
        let mut wd = wd.health_cmd("exit 0");
        let out = wd.run_probe();
        assert_eq!(out.action, ProbeAction::Healthy);
    }

    #[test]
    fn no_health_cmd_reports_only() {
        let dir = tmp("report");
        std::fs::create_dir_all(&dir).unwrap();
        let mut wd = Watchdog::new("svc", &dir.join("b.jsonl"));
        assert_eq!(wd.run_probe().action, ProbeAction::ReportOnly);
    }
}
