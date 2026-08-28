//! recall.rs — the read-only Recall API (P1 surface).
//!
//! Three calls, per the CONVENING ruling Q1: `search` (fast lane), `query`
//! (deep lane), `get` (fast lane). Every call goes through the Runner seam
//! with a sanitized environment and a hard lane budget, and returns its
//! telemetry alongside the data — the organ reports, the host decides.
//!
//! Lane budgets (ruling, from live probes 2026-08-26):
//! - FAST 5 s — `search` measured 0.2 s e2e (BM25, no LLM); `get` is a
//!   sqlite read, far under budget.
//! - DEEP 60 s — `query` measured 15.0 s warm and 21.2 s cold (hybrid +
//!   local GGUF expansion + rerank); the budget absorbs cold-start variance.
//!
//! Fail-closed law: any nonzero exit, timeout, spawn failure, or unparseable
//! output is a `RecallError` — never an empty-result success. An empty array
//! is returned ONLY when qmd itself exited 0 with `[]`.

use std::path::PathBuf;
use std::time::Duration;

use crate::exec::{Job, Outcome, RealRunner, Runner};
use crate::parse::{parse_get, parse_hits, GetDoc, Hit, ParseErr};

pub const FAST_LANE: Duration = Duration::from_secs(5);
pub const DEEP_LANE: Duration = Duration::from_secs(60);

/// Where qmd lives and how long lanes may run.
///
/// `launcher` is `program + prefix args`, e.g.
/// `["node", "C:/Users/…/npm/node_modules/@tobilu/qmd/bin/qmd"]`. A Vec keeps
/// this crate free of shell-quoting law (raw args, no cmd.exe round trip —
/// the raw_arg lesson from caddis-organs shell.rs).
#[derive(Debug, Clone)]
pub struct MemoryConfig {
    pub launcher: Vec<String>,
    /// Working dir for every call: qmd resolves a project-local `.qmd` index
    /// from cwd. `None` = the machine-global index (the live memory).
    pub workdir: Option<PathBuf>,
    pub fast_timeout: Duration,
    pub deep_timeout: Duration,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        MemoryConfig {
            launcher: default_launcher(),
            workdir: None,
            fast_timeout: FAST_LANE,
            deep_timeout: DEEP_LANE,
        }
    }
}

impl MemoryConfig {
    /// Detect from the environment: `QMD_LAUNCHER` (full command, split on
    /// whitespace — paths with spaces must come via workdir instead) or the
    /// npm-global default probed live in the convening.
    pub fn detect() -> Self {
        let launcher = std::env::var("QMD_LAUNCHER")
            .ok()
            .map(|s| s.split_whitespace().map(str::to_string).collect())
            .unwrap_or_else(default_launcher);
        MemoryConfig {
            launcher,
            ..MemoryConfig::default()
        }
    }
}

fn default_launcher() -> Vec<String> {
    vec![
        "node".into(),
        "C:/Users/ashpac/AppData/Roaming/npm/node_modules/@tobilu/qmd/bin/qmd".into(),
    ]
}

/// What one recall call provably did — the organ's telemetry (ruling Q1:
/// latency telemetry ships with v1).
#[derive(Debug, Clone)]
pub struct Report {
    pub subcommand: &'static str,
    pub query: String,
    pub duration: Duration,
    pub timed_out: bool,
    pub code: Option<i32>,
    pub stdout_bytes: usize,
    pub stderr_bytes: usize,
    pub stdout_truncated: bool,
    pub result_count: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RecallError {
    Spawn(String),
    /// Killed at the lane deadline — includes which lane and its budget.
    Timeout {
        subcommand: &'static str,
        budget: Duration,
        after: Duration,
    },
    NonZero {
        code: i32,
        stderr_head: String,
    },
    Parse {
        why: ParseErr,
        stdout_head: String,
    },
}

impl std::fmt::Display for RecallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RecallError::Spawn(m) => write!(f, "spawn failed: {m}"),
            RecallError::Timeout {
                subcommand,
                budget,
                after,
            } => write!(
                f,
                "{subcommand} killed at lane budget {budget:?} (elapsed {after:?})"
            ),
            RecallError::NonZero { code, stderr_head } => {
                write!(f, "qmd exited {code}: {}", head(stderr_head, 200))
            }
            RecallError::Parse { why, stdout_head } => {
                write!(
                    f,
                    "unparseable qmd output ({why:?}): {}",
                    head(stdout_head, 200)
                )
            }
        }
    }
}

fn head(s: &str, n: usize) -> &str {
    match s.char_indices().nth(n) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

/// The recall organ handle. Construct with `Recall::new(config)` (real
/// subprocess lane) or `Recall::with_runner` (any Runner — the test seam).
pub struct Recall<R: Runner = RealRunner> {
    config: MemoryConfig,
    runner: R,
}

impl Recall<RealRunner> {
    pub fn new(config: MemoryConfig) -> Self {
        Recall {
            config,
            runner: RealRunner,
        }
    }
}

impl<R: Runner> Recall<R> {
    pub fn with_runner(config: MemoryConfig, runner: R) -> Self {
        Recall { config, runner }
    }

    pub fn config(&self) -> &MemoryConfig {
        &self.config
    }

    /// Fast lane: BM25 full-text search, no LLM. Measured ~0.2 s.
    pub fn search(&mut self, query: &str) -> Result<(Vec<Hit>, Report), RecallError> {
        let (out, report) = self.call(
            "search",
            query,
            vec!["search".into(), query.into(), "--json".into()],
        )?;
        let hits = parse_hits(&out.stdout).map_err(|why| to_parse_err(why, &out))?;
        let mut report = report;
        report.result_count = hits.len();
        Ok((hits, report))
    }

    /// Deep lane: hybrid + local GGUF expansion + rerank. Measured 15–21 s;
    /// background/panel use only. Requires the sanitized env (CI stripped) —
    /// the Runner owns that law.
    pub fn query(&mut self, query: &str) -> Result<(Vec<Hit>, Report), RecallError> {
        let (out, report) = self.call(
            "query",
            query,
            vec!["query".into(), query.into(), "--json".into()],
        )?;
        let hits = parse_hits(&out.stdout).map_err(|why| to_parse_err(why, &out))?;
        let mut report = report;
        report.result_count = hits.len();
        Ok((hits, report))
    }

    /// Fast lane: fetch one document by path (optionally `file:from:count`).
    /// Plain-text output (`get` ignores `--json`), parsed into numbered lines.
    pub fn get(&mut self, file: &str) -> Result<(GetDoc, Report), RecallError> {
        let (out, report) = self.call("get", file, vec!["get".into(), file.into()])?;
        let doc = parse_get(&out.stdout).map_err(|why| to_parse_err(why, &out))?;
        let mut report = report;
        report.result_count = doc.lines.len();
        Ok((doc, report))
    }

    fn call(
        &mut self,
        subcommand: &'static str,
        query: &str,
        args: Vec<String>,
    ) -> Result<(Outcome, Report), RecallError> {
        let budget = match subcommand {
            "query" => self.config.deep_timeout,
            _ => self.config.fast_timeout,
        };
        let job = Job {
            launcher: self.config.launcher.clone(),
            args,
            workdir: self.config.workdir.clone(),
            timeout: budget,
            stdin_data: None,
        };
        let out = self.runner.run(&job);
        let report = Report {
            subcommand,
            query: query.to_string(),
            duration: out.duration,
            timed_out: out.timed_out,
            code: out.code,
            stdout_bytes: out.stdout.len(),
            stderr_bytes: out.stderr.len(),
            stdout_truncated: out.stdout_truncated,
            result_count: 0,
        };
        // Fail-closed gate BEFORE any parsing: a killed or failed run never
        // falls through to "empty results" or a misleading parse error.
        if let Some(err) = fail_closed(&out, &report) {
            return Err(err);
        }
        Ok((out, report))
    }
}

fn to_parse_err(why: ParseErr, out: &Outcome) -> RecallError {
    RecallError::Parse {
        why,
        stdout_head: head(&out.stdout, 400).to_string(),
    }
}

fn fail_closed(outcome: &Outcome, report: &Report) -> Option<RecallError> {
    if outcome.timed_out {
        return Some(RecallError::Timeout {
            subcommand: report.subcommand,
            budget: if report.subcommand == "query" {
                DEEP_LANE
            } else {
                FAST_LANE
            },
            after: outcome.duration,
        });
    }
    match outcome.code {
        None => Some(RecallError::Spawn(outcome.stderr.clone())),
        Some(c) if c != 0 => Some(RecallError::NonZero {
            code: c,
            stderr_head: outcome.stderr.clone(),
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::testing::FakeRunner;
    use std::time::Duration;

    fn cfg() -> MemoryConfig {
        MemoryConfig {
            launcher: vec!["node".into(), "qmd".into()],
            workdir: None,
            fast_timeout: Duration::from_secs(5),
            deep_timeout: Duration::from_secs(60),
        }
    }

    #[test]
    fn search_parses_and_telemeters() {
        let mut fake = FakeRunner::default();
        fake.on(
            "search",
            FakeRunner::ok_json("search", r##"[{"docid":"#1","file":"qmd://a.md"}]"##),
        );
        let mut recall = Recall::with_runner(cfg(), fake);
        let (hits, report) = recall.search("needle").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(report.subcommand, "search");
        assert_eq!(report.result_count, 1);
        assert_eq!(report.code, Some(0));
        // the args carried --json and the query verbatim
        let calls = &recall.runner.calls;
        assert_eq!(calls[0], vec!["search", "needle", "--json"]);
    }

    #[test]
    fn deep_lane_uses_deep_budget() {
        let mut fake = FakeRunner::default();
        let mut slow = FakeRunner::ok_json("query", "[]");
        slow.duration = Duration::from_secs(61);
        slow.timed_out = true;
        slow.code = None;
        fake.on("query", slow);
        let mut recall = Recall::with_runner(cfg(), fake);
        let err = recall.query("deep thing").unwrap_err();
        assert!(matches!(
            err,
            RecallError::Timeout {
                subcommand: "query",
                ..
            }
        ));
        assert!(err.to_string().contains("60s"));
    }

    #[test]
    fn nonzero_exit_fails_closed() {
        let mut fake = FakeRunner::default();
        let mut bad = FakeRunner::ok_json("search", "[]");
        bad.code = Some(1);
        bad.stderr = "LLM operations are disabled in CI (set CI=true)".into();
        fake.on("search", bad);
        let mut recall = Recall::with_runner(cfg(), fake);
        let err = recall.search("x").unwrap_err();
        match err {
            RecallError::NonZero { code, stderr_head } => {
                assert_eq!(code, 1);
                assert!(stderr_head.contains("disabled in CI"));
            }
            other => panic!("expected NonZero, got {other:?}"),
        }
    }

    #[test]
    fn unparseable_output_fails_closed_with_head() {
        let mut fake = FakeRunner::default();
        fake.on(
            "search",
            FakeRunner::ok_json("search", "Warning: junk\nno json"),
        );
        let mut recall = Recall::with_runner(cfg(), fake);
        let err = recall.search("x").unwrap_err();
        assert!(matches!(err, RecallError::Parse { .. }));
        assert!(err.to_string().contains("Warning: junk"));
    }

    #[test]
    fn empty_result_is_success_not_error() {
        let mut fake = FakeRunner::default();
        fake.on("search", FakeRunner::ok_json("search", "[]"));
        let mut recall = Recall::with_runner(cfg(), fake);
        let (hits, report) = recall.search("nothing matches").unwrap();
        assert!(hits.is_empty());
        assert_eq!(report.result_count, 0);
        assert_eq!(report.code, Some(0));
    }

    #[test]
    fn get_uses_fast_lane_and_no_json_flag() {
        let mut fake = FakeRunner::default();
        let mut out = FakeRunner::ok_json("get", "");
        out.stdout = "qmd://docs/g.md  #9\n1: hello\n2: world\n".into();
        fake.on("get", out);
        let mut recall = Recall::with_runner(cfg(), fake);
        let (doc, report) = recall.get("docs/g.md").unwrap();
        assert_eq!(doc.docid, "#9");
        assert_eq!(report.result_count, 2);
        assert_eq!(recall.runner.calls[0], vec!["get", "docs/g.md"]);
    }

    #[test]
    fn spawn_failure_surfaces() {
        let mut fake = FakeRunner::default();
        let mut dead = FakeRunner::ok_json("search", "[]");
        dead.code = None;
        dead.stderr = "spawn failed: no such program".into();
        fake.on("search", dead);
        let mut recall = Recall::with_runner(cfg(), fake);
        assert!(matches!(recall.search("x"), Err(RecallError::Spawn(_))));
    }

    #[test]
    fn lane_budgets_match_ruling() {
        assert_eq!(FAST_LANE, Duration::from_secs(5));
        assert_eq!(DEEP_LANE, Duration::from_secs(60));
    }
}
