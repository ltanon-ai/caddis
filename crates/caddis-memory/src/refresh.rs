//! refresh.rs — P2: the refresh sub-organ (watchdog pattern, Q3 ratified 3/3).
//!
//! Council ruling (CONVENING.md): the ORGAN owns index refresh — staleness
//! probe → `qmd update` → `qmd embed`, serialized by a file lock with
//! steal-on-stale (adopted amendment 1; loop-runner T-3 lock precedent);
//! the post-refresh canary asserts pending-embed == 0 (amendment 2) and RED
//! halts per the caddis-organs law. The organ REPORTS and PROVES — the host
//! decides and halts. `refresh()` kills nothing but its own subprocesses at
//! their deadlines.
//!
//! Verdict classes follow canary.rs:
//! - **RED** — the chain provably failed: nonzero exit, timeout,
//!   unparseable `status`, or the canary still seeing pending embeds.
//! - **DEGRADED** — qmd is unusable on this machine (spawn failure only);
//!   never halts, exactly like an absent model lane.
//! - **BUSY** — a live sibling refresh holds the lock (host retries later).
//! - **FRESH / REFRESHED** — provably good; REFRESHED carries step traces.
//!
//! Status parsing is strict (fail-closed): `Total`/`Vectors` lines are
//! mandatory. A missing `Pending:` line reads as 0 pending — qmd prints it
//! as a call-to-action, and the post-embed recheck decides the canary, not
//! this assumption. A missing `Updated:` line reads as unknown age (never
//! stale by age alone).

use crate::exec::{Job, Outcome, Runner};
use crate::recall::MemoryConfig;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// `qmd status` budget. Measured ~1 s live; generous for cold node starts.
pub const PROBE_BUDGET: Duration = Duration::from_secs(30);
/// `qmd update` budget (rescan of ~3.2k docs + sqlite writes).
pub const UPDATE_BUDGET: Duration = Duration::from_secs(120);
/// `qmd embed` budget — local GGUF embedding of a pending backlog.
pub const EMBED_BUDGET: Duration = Duration::from_secs(600);
/// Lock staleness before steal — the loop-runner T-3 precedent (900 s).
pub const LOCK_STALE_AFTER: Duration = Duration::from_secs(900);
/// An index at least this old is stale even with zero pending embeds
/// (files may have changed without a rescan).
pub const STALE_INDEX_AFTER: Duration = Duration::from_secs(6 * 3600);

/// What one refresh step provably did (telemetry; heads only — payloads
/// belong to the organ, not the log).
#[derive(Debug, Clone, PartialEq)]
pub struct StepTrace {
    pub phase: &'static str,
    pub duration: Duration,
    pub timed_out: bool,
    pub code: Option<i32>,
    pub stdout_bytes: usize,
    pub stderr_bytes: usize,
}

impl StepTrace {
    fn of(phase: &'static str, out: &Outcome) -> Self {
        StepTrace {
            phase,
            duration: out.duration,
            timed_out: out.timed_out,
            code: out.code,
            stdout_bytes: out.stdout.len(),
            stderr_bytes: out.stderr.len(),
        }
    }
}

/// One parsed `qmd status` report — the staleness probe's ground truth.
#[derive(Debug, Clone, PartialEq)]
pub struct StatusSnapshot {
    pub total_docs: u64,
    pub vectors: u64,
    pub pending_embed: u64,
    /// `None` = qmd did not state an index age.
    pub index_age_secs: Option<u64>,
    pub collections: Vec<CollectionStatus>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CollectionStatus {
    pub name: String,
    pub files: u64,
    pub updated_ago_secs: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RefreshError {
    Spawn(String),
    Timeout {
        phase: &'static str,
        budget: Duration,
        after: Duration,
    },
    NonZero {
        phase: &'static str,
        code: i32,
        stderr_head: String,
    },
    Parse {
        why: String,
        stdout_head: String,
    },
}

impl std::fmt::Display for RefreshError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RefreshError::Spawn(why) => write!(f, "spawn failed: {why}"),
            RefreshError::Timeout {
                phase,
                budget,
                after,
            } => {
                write!(f, "{phase} killed at {budget:?} deadline (after {after:?})")
            }
            RefreshError::NonZero {
                phase,
                code,
                stderr_head,
            } => {
                write!(f, "{phase} exited {code}: {stderr_head}")
            }
            RefreshError::Parse { why, stdout_head } => {
                write!(f, "status unparseable ({why}): {stdout_head}")
            }
        }
    }
}

/// The refresh verdict. The organ reports; the HOST halts on Red.
#[derive(Debug, Clone, PartialEq)]
pub enum RefreshVerdict {
    /// Nothing to do: zero pending embeds and the index is younger than the
    /// staleness threshold.
    Fresh { snapshot: StatusSnapshot },
    /// Update + embed ran under the lock and the canary saw pending == 0.
    Refreshed {
        before: StatusSnapshot,
        after: StatusSnapshot,
        steps: Vec<StepTrace>,
        lock_stolen: bool,
    },
    /// A live sibling refresh owns the lock. Not an error — the host retries.
    Busy {
        pid: Option<u32>,
        age_secs: Option<u64>,
    },
    /// Provably failed. RED halts (canary law).
    Red(String),
    /// qmd unusable right now (spawn failure only). Never halts.
    Degraded(String),
}

/// Where qmd lives, where the refresh lock lives, how long steps may run.
#[derive(Debug, Clone)]
pub struct RefreshConfig {
    pub memory: MemoryConfig,
    pub lock_path: PathBuf,
    pub lock_stale_after: Duration,
    /// Index age at/above which a refresh runs even with zero pending embeds.
    pub stale_after: Duration,
    pub probe_timeout: Duration,
    pub update_timeout: Duration,
    pub embed_timeout: Duration,
}

impl RefreshConfig {
    pub fn new(memory: MemoryConfig) -> Self {
        RefreshConfig {
            lock_path: default_lock_path(),
            memory,
            lock_stale_after: LOCK_STALE_AFTER,
            stale_after: STALE_INDEX_AFTER,
            probe_timeout: PROBE_BUDGET,
            update_timeout: UPDATE_BUDGET,
            embed_timeout: EMBED_BUDGET,
        }
    }
}

pub(crate) fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

/// The refresh lock lives beside the resource it serializes (the qmd cache).
pub fn default_lock_path() -> PathBuf {
    home_dir()
        .map(|h| h.join(".cache").join("qmd").join("caddis-refresh.lock"))
        .unwrap_or_else(|| std::env::temp_dir().join("caddis-refresh.lock"))
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Status parsing (strict, fail-closed)
// ---------------------------------------------------------------------------

fn first_num(s: &str) -> Option<u64> {
    s.split_whitespace().next()?.parse().ok()
}

/// "21h" / "12d" / "30m" / "45s" / "2w" (bare digits = seconds).
fn parse_age(tok: &str) -> Option<u64> {
    let mult = match tok.as_bytes().last()? {
        b's' => 1,
        b'm' => 60,
        b'h' => 3600,
        b'd' => 86_400,
        b'w' => 604_800,
        _ => return tok.parse::<u64>().ok(),
    };
    tok[..tok.len() - 1].parse::<u64>().ok().map(|n| n * mult)
}

/// `Files:    49 (updated 12d ago)` → (49, Some(age)); age absent → None.
fn parse_files_value(v: &str) -> Option<(u64, Option<u64>)> {
    let files = first_num(v)?;
    let age = v.find("(updated ").and_then(|i| {
        let rest = &v[i + "(updated ".len()..];
        let rest = rest.strip_suffix(')').unwrap_or(rest);
        let rest = rest.strip_suffix(" ago").unwrap_or(rest);
        parse_age(rest.trim())
    });
    Some((files, age))
}

/// Parse the text report of `qmd status` (live shape probed 2026-08-26; the
/// `--json` flag is accepted but ignored by qmd — text IS the contract).
pub fn parse_status(text: &str) -> Result<StatusSnapshot, RefreshError> {
    let head = |n: usize| text.get(..n).unwrap_or(text).replace('\n', "\\n");
    let bad = |why: &str| RefreshError::Parse {
        why: why.to_string(),
        stdout_head: head(200),
    };

    #[derive(PartialEq)]
    enum Sec {
        None,
        Documents,
        Collections,
    }

    let mut sec = Sec::None;
    let mut total: Option<u64> = None;
    let mut vectors: Option<u64> = None;
    let mut pending: u64 = 0;
    let mut updated: Option<u64> = None;
    let mut collections: Vec<CollectionStatus> = Vec::new();
    let mut current: Option<String> = None;

    for raw in text.lines() {
        let line = raw.trim_end_matches('\r');
        let trimmed = line.trim();
        if !line.starts_with(' ') {
            // Section headers are flush-left single words.
            sec = match trimmed {
                "Documents" => Sec::Documents,
                "Collections" => Sec::Collections,
                _ => Sec::None,
            };
            current = None;
            continue;
        }
        if trimmed.is_empty() {
            continue;
        }
        match sec {
            Sec::Documents => {
                if let Some(v) = trimmed.strip_prefix("Total:") {
                    total = Some(first_num(v).ok_or_else(|| bad("Total line unreadable"))?);
                } else if let Some(v) = trimmed.strip_prefix("Vectors:") {
                    vectors = Some(first_num(v).ok_or_else(|| bad("Vectors line unreadable"))?);
                } else if let Some(v) = trimmed.strip_prefix("Pending:") {
                    pending = first_num(v).ok_or_else(|| bad("Pending line unreadable"))?;
                } else if let Some(v) = trimmed.strip_prefix("Updated:") {
                    let mut toks = v.split_whitespace();
                    let age = match toks.next() {
                        Some("just") | Some("now") => Some(0),
                        Some(t) => parse_age(t),
                        None => None,
                    };
                    updated = age; // unparseable token → None → unknown age
                }
            }
            Sec::Collections => {
                let toks: Vec<&str> = trimmed.split_whitespace().collect();
                if toks.len() == 2 && toks[1].starts_with("(qmd://") && toks[1].ends_with(')') {
                    current = Some(toks[0].to_string());
                } else if let (Some(name), Some(v)) = (&current, trimmed.strip_prefix("Files:")) {
                    let (files, updated_ago_secs) =
                        parse_files_value(v).ok_or_else(|| bad("Files line unreadable"))?;
                    collections.push(CollectionStatus {
                        name: name.clone(),
                        files,
                        updated_ago_secs,
                    });
                }
            }
            Sec::None => {}
        }
    }

    let total_docs = total.ok_or_else(|| bad("Documents section missing Total"))?;
    let vectors = vectors.ok_or_else(|| bad("Documents section missing Vectors"))?;
    Ok(StatusSnapshot {
        total_docs,
        vectors,
        pending_embed: pending,
        index_age_secs: updated,
        collections,
    })
}

// ---------------------------------------------------------------------------
// The refresh lock (steal-on-stale; loop-runner T-3 precedent)
// ---------------------------------------------------------------------------

/// Result of trying to take the refresh lock.
#[derive(Debug, Clone, PartialEq)]
pub enum LockState {
    /// The lock file did not exist.
    Acquired,
    /// A stale (or pre-organ, unparseable) lock was overwritten.
    Stolen {
        pid: Option<u32>,
        age_secs: Option<u64>,
    },
    /// A live holder owns it — back off.
    Busy {
        pid: Option<u32>,
        age_secs: Option<u64>,
    },
}

/// Lock file body: `pid\ntimestamp\n`.
pub fn parse_lock(text: &str) -> Option<(u32, u64)> {
    let mut it = text.lines();
    let pid = it.next()?.trim().parse::<u32>().ok()?;
    let ts = it.next()?.trim().parse::<u64>().ok()?;
    Some((pid, ts))
}

/// Pure decision. `holder = None` means the file existed but was unparseable:
/// our writes are tmp+rename (atomic), so on-disk corruption predates this
/// organ and is stealable garbage.
pub fn decide_lock(holder: Option<(u32, u64)>, now_secs: u64, stale_after_secs: u64) -> LockState {
    match holder {
        None => LockState::Stolen {
            pid: None,
            age_secs: None,
        },
        Some((pid, ts)) => {
            let age = now_secs.saturating_sub(ts);
            if age >= stale_after_secs {
                LockState::Stolen {
                    pid: Some(pid),
                    age_secs: Some(age),
                }
            } else {
                LockState::Busy {
                    pid: Some(pid),
                    age_secs: Some(age),
                }
            }
        }
    }
}

fn sibling_tmp(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(".tmp");
    PathBuf::from(s)
}

fn write_lock(path: &Path) -> io::Result<()> {
    let body = format!("{}\n{}\n", std::process::id(), now_secs());
    let tmp = sibling_tmp(path);
    fs::write(&tmp, body)?;
    fs::rename(&tmp, path) // replaces the target (MOVEFILE_REPLACE_EXISTING)
}

/// Take the lock if free or stale; report Busy otherwise. An unreadable
/// (permissions) lock is NEVER stolen blind — that reports Busy{None,None}.
pub fn acquire_lock(path: &Path, stale_after: Duration) -> LockState {
    let now = now_secs();
    let state = match fs::read_to_string(path) {
        Ok(text) => decide_lock(parse_lock(&text), now, stale_after.as_secs()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => LockState::Acquired,
        Err(_) => {
            return LockState::Busy {
                pid: None,
                age_secs: None,
            }
        }
    };
    if !matches!(state, LockState::Busy { .. }) && write_lock(path).is_err() {
        return LockState::Busy {
            pid: None,
            age_secs: None,
        };
    }
    state
}

/// Release only a lock that still carries OUR pid — a lock stolen from us
/// mid-run by a newer refresh stays theirs (we lost the race; our embeds
/// finished anyway, theirs serializes on top).
pub fn release_lock(path: &Path) {
    let pid = std::process::id();
    if let Ok(text) = fs::read_to_string(path) {
        if parse_lock(&text).is_some_and(|(p, _)| p == pid) {
            let _ = fs::remove_file(path);
        }
    }
}

// ---------------------------------------------------------------------------
// The watchdog flow
// ---------------------------------------------------------------------------

/// Read-only staleness probe: run `qmd status` and parse it.
pub fn probe<R: Runner>(
    runner: &mut R,
    cfg: &RefreshConfig,
) -> Result<StatusSnapshot, RefreshError> {
    let job = Job {
        launcher: cfg.memory.launcher.clone(),
        args: vec!["status".into()],
        workdir: cfg.memory.workdir.clone(),
        timeout: cfg.probe_timeout,
        stdin_data: None,
    };
    let out = runner.run(&job);
    if let Some(err) = fail_closed("status", cfg.probe_timeout, &out) {
        return Err(err);
    }
    parse_status(&out.stdout)
}

fn fail_closed(phase: &'static str, budget: Duration, out: &Outcome) -> Option<RefreshError> {
    if out.timed_out {
        return Some(RefreshError::Timeout {
            phase,
            budget,
            after: out.duration,
        });
    }
    match out.code {
        None => Some(RefreshError::Spawn(out.stderr.clone())),
        Some(c) if c != 0 => Some(RefreshError::NonZero {
            phase,
            code: c,
            stderr_head: out.stderr.chars().take(400).collect(),
        }),
        _ => None,
    }
}

fn err_verdict(e: RefreshError) -> RefreshVerdict {
    match e {
        RefreshError::Spawn(why) => RefreshVerdict::Degraded(why),
        other => RefreshVerdict::Red(other.to_string()),
    }
}

/// The watchdog: probe → (if stale) update → embed under the lock →
/// canary pending-embed == 0. The lock is ALWAYS released on every exit
/// path after it is taken.
pub fn refresh<R: Runner>(runner: &mut R, cfg: &RefreshConfig) -> RefreshVerdict {
    let before = match probe(runner, cfg) {
        Ok(s) => s,
        Err(e) => return err_verdict(e),
    };
    let stale = before.pending_embed > 0
        || before
            .index_age_secs
            .is_some_and(|a| a >= cfg.stale_after.as_secs());
    if !stale {
        return RefreshVerdict::Fresh { snapshot: before };
    }
    let (lock_stolen, verdict) = match acquire_lock(&cfg.lock_path, cfg.lock_stale_after) {
        LockState::Busy { pid, age_secs } => return RefreshVerdict::Busy { pid, age_secs },
        LockState::Acquired => (false, refresh_locked(runner, cfg, before)),
        LockState::Stolen { .. } => (true, refresh_locked(runner, cfg, before)),
    };
    release_lock(&cfg.lock_path);
    match verdict {
        RefreshVerdict::Refreshed {
            before,
            after,
            steps,
            ..
        } => RefreshVerdict::Refreshed {
            before,
            after,
            steps,
            lock_stolen,
        },
        other => other,
    }
}

fn refresh_locked<R: Runner>(
    runner: &mut R,
    cfg: &RefreshConfig,
    before: StatusSnapshot,
) -> RefreshVerdict {
    let plan: [(&'static str, Vec<String>, Duration); 2] = [
        ("update", vec!["update".into()], cfg.update_timeout),
        ("embed", vec!["embed".into()], cfg.embed_timeout),
    ];
    let mut steps = Vec::new();
    for (phase, args, budget) in plan {
        let job = Job {
            launcher: cfg.memory.launcher.clone(),
            args,
            workdir: cfg.memory.workdir.clone(),
            timeout: budget,
            stdin_data: None,
        };
        let out = runner.run(&job);
        steps.push(StepTrace::of(phase, &out));
        if let Some(err) = fail_closed(phase, budget, &out) {
            return err_verdict(err);
        }
    }
    // The canary (amendment 2): pending-embed must return to 0. RED halts.
    match probe(runner, cfg) {
        Ok(after) if after.pending_embed == 0 => RefreshVerdict::Refreshed {
            before,
            after,
            steps,
            lock_stolen: false,
        },
        Ok(after) => RefreshVerdict::Red(format!(
            "canary RED: pending-embed still {} after refresh",
            after.pending_embed
        )),
        Err(e) => err_verdict(e),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::testing::FakeRunner;
    use std::fs;

    fn out(stdout: &str) -> Outcome {
        Outcome {
            code: Some(0),
            timed_out: false,
            stdout: stdout.to_string(),
            stderr: String::new(),
            duration: Duration::from_millis(100),
            stdout_truncated: false,
            stderr_truncated: false,
        }
    }

    fn ok() -> Outcome {
        out("")
    }

    /// The live `qmd status` shape (probed 2026-08-26), parameterized.
    fn status_text(pending_line: Option<&str>, updated: &str) -> String {
        let pending = pending_line
            .map(|p| format!("  Pending:  {p} need embedding (run 'qmd embed')\n"))
            .unwrap_or_default();
        format!(
            "QMD Status\r\n\
             \r\n\
             Index: C:/Users/ashpac/.cache/qmd/index.sqlite\r\n\
             Size:  131.3 MB\r\n\
             MCP:   running (PID 20476)\r\n\
             \r\n\
             Documents\r\n\
             \x20 Total:    3196 files indexed\r\n\
             \x20 Vectors:  20797 embedded\r\n\
             {pending}\
             \x20 Updated:  {updated} ago\r\n\
             \r\n\
             AST Chunking\r\n\
             \x20 Status:   active\r\n\
             \r\n\
             Collections\r\n\
             \x20 showr (qmd://showr/)\r\n\
             \x20   Pattern:  **/*.md\r\n\
             \x20   Files:    49 (updated 12d ago)\r\n\
             \x20   Contexts: 1\r\n\
             \x20 memory (qmd://memory/)\r\n\
             \x20   Pattern:  **/*.md\r\n\
             \x20   Files:    777 (updated 21h ago)\r\n\
             \x20   Contexts: 1\r\n"
        )
    }

    fn status_out(pending: u64) -> Outcome {
        out(&status_text(Some(&pending.to_string()), "21h"))
    }

    fn zero_pending_out() -> Outcome {
        out(&status_text(Some("0"), "5m"))
    }

    fn cfg_in(dir: &Path) -> RefreshConfig {
        RefreshConfig {
            lock_path: dir.join("refresh.lock"),
            memory: MemoryConfig {
                launcher: vec!["qmd".into()],
                workdir: None,
                fast_timeout: FAST,
                deep_timeout: DEEP,
            },
            lock_stale_after: Duration::from_secs(900),
            stale_after: Duration::from_secs(6 * 3600),
            probe_timeout: PROBE_BUDGET,
            update_timeout: UPDATE_BUDGET,
            embed_timeout: EMBED_BUDGET,
        }
    }

    const FAST: Duration = Duration::from_secs(5);
    const DEEP: Duration = Duration::from_secs(60);

    fn tmp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("caddis-refresh-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    // ---- parse_status ----

    #[test]
    fn parses_live_shape() {
        let snap = parse_status(&status_text(Some("7"), "21h")).unwrap();
        assert_eq!(snap.total_docs, 3196);
        assert_eq!(snap.vectors, 20797);
        assert_eq!(snap.pending_embed, 7);
        assert_eq!(snap.index_age_secs, Some(21 * 3600));
        assert_eq!(snap.collections.len(), 2);
        assert_eq!(snap.collections[0].name, "showr");
        assert_eq!(snap.collections[0].files, 49);
        assert_eq!(snap.collections[0].updated_ago_secs, Some(12 * 86_400));
        assert_eq!(snap.collections[1].name, "memory");
        assert_eq!(snap.collections[1].files, 777);
    }

    #[test]
    fn missing_pending_line_reads_zero() {
        let snap = parse_status(&status_text(None, "1h")).unwrap();
        assert_eq!(snap.pending_embed, 0);
    }

    #[test]
    fn missing_updated_line_reads_unknown_age() {
        let text = status_text(Some("0"), "21h").replace("  Updated:  21h ago\r\n", "");
        let snap = parse_status(&text).unwrap();
        assert_eq!(snap.index_age_secs, None);
    }

    #[test]
    fn updated_just_now_is_zero() {
        let snap = parse_status(&status_text(Some("0"), "just now")).unwrap();
        assert_eq!(snap.index_age_secs, Some(0));
    }

    #[test]
    fn missing_total_is_parse_error() {
        let text = status_text(Some("7"), "21h").replace("  Total:    3196 files indexed\r\n", "");
        assert!(matches!(
            parse_status(&text),
            Err(RefreshError::Parse { .. })
        ));
    }

    #[test]
    fn missing_vectors_is_parse_error() {
        let text = status_text(Some("7"), "21h").replace("  Vectors:  20797 embedded\r\n", "");
        assert!(matches!(
            parse_status(&text),
            Err(RefreshError::Parse { .. })
        ));
    }

    #[test]
    fn garbage_pending_token_is_parse_error() {
        let text = status_text(Some("soon"), "21h");
        assert!(matches!(
            parse_status(&text),
            Err(RefreshError::Parse { .. })
        ));
    }

    #[test]
    fn age_units() {
        assert_eq!(parse_age("45s"), Some(45));
        assert_eq!(parse_age("30m"), Some(1800));
        assert_eq!(parse_age("21h"), Some(75_600));
        assert_eq!(parse_age("12d"), Some(1_036_800));
        assert_eq!(parse_age("2w"), Some(1_209_600));
        assert_eq!(parse_age("90"), Some(90));
        assert_eq!(parse_age("soon"), None);
        assert_eq!(parse_age(""), None);
    }

    // ---- lock ----

    #[test]
    fn lock_parse_roundtrip() {
        assert_eq!(parse_lock("1234\n5678\n"), Some((1234, 5678)));
        assert_eq!(parse_lock(""), None);
        assert_eq!(parse_lock("nan\n5678\n"), None);
        assert_eq!(parse_lock("1234\nnan\n"), None);
        assert_eq!(parse_lock("1234\n"), None);
    }

    #[test]
    fn lock_decision_matrix() {
        // fresh holder → busy
        assert_eq!(
            decide_lock(Some((10, 1_000)), 1_500, 900),
            LockState::Busy {
                pid: Some(10),
                age_secs: Some(500)
            }
        );
        // stale holder → steal
        assert_eq!(
            decide_lock(Some((10, 1_000)), 2_000, 900),
            LockState::Stolen {
                pid: Some(10),
                age_secs: Some(1_000)
            }
        );
        // exactly at the boundary → steal (>= is stale)
        assert_eq!(
            decide_lock(Some((10, 1_000)), 1_900, 900),
            LockState::Stolen {
                pid: Some(10),
                age_secs: Some(900)
            }
        );
        // corrupt body → steal, unknown holder
        assert_eq!(
            decide_lock(None, 1_900, 900),
            LockState::Stolen {
                pid: None,
                age_secs: None
            }
        );
        // clock behind holder (skew) → age clamps to 0 → busy
        assert_eq!(
            decide_lock(Some((10, 5_000)), 1_000, 900),
            LockState::Busy {
                pid: Some(10),
                age_secs: Some(0)
            }
        );
    }

    #[test]
    fn lock_acquire_free_then_release() {
        let dir = tmp_dir("free");
        let path = dir.join("refresh.lock");
        assert_eq!(
            acquire_lock(&path, Duration::from_secs(900)),
            LockState::Acquired
        );
        assert!(path.exists());
        release_lock(&path); // our pid is in the file → removed
        assert!(!path.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn lock_acquire_busy_on_fresh_foreign_holder() {
        let dir = tmp_dir("busy");
        let path = dir.join("refresh.lock");
        // A foreign pid holding a just-written lock.
        let tmp = sibling_tmp(&path);
        fs::write(&tmp, format!("999999\n{}\n", now_secs())).unwrap();
        fs::rename(&tmp, &path).unwrap();
        match acquire_lock(&path, Duration::from_secs(900)) {
            LockState::Busy {
                pid: Some(999_999), ..
            } => {}
            other => panic!("expected busy, got {other:?}"),
        }
        // release_lock must NOT remove a foreign pid's lock.
        release_lock(&path);
        assert!(path.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn lock_acquire_steals_ancient_holder() {
        let dir = tmp_dir("steal");
        let path = dir.join("refresh.lock");
        fs::write(&path, "1\n0\n").unwrap(); // pid 1, epoch timestamp
        match acquire_lock(&path, Duration::from_secs(900)) {
            LockState::Stolen { pid: Some(1), .. } => {}
            other => panic!("expected steal, got {other:?}"),
        }
        release_lock(&path); // now OUR pid is stamped → removed
        assert!(!path.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    // ---- refresh flow (FakeRunner, scripted with then()) ----

    #[test]
    fn fresh_index_is_a_no_op() {
        let mut runner = FakeRunner::default();
        runner.on("status", zero_pending_out());
        let dir = tmp_dir("fresh");
        let verdict = refresh(&mut runner, &cfg_in(&dir));
        assert!(matches!(verdict, RefreshVerdict::Fresh { .. }));
        assert_eq!(runner.calls, vec![vec!["status".to_string()]]);
        assert!(
            !dir.join("refresh.lock").exists(),
            "fresh no-op must not take the lock"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn refresh_runs_update_embed_and_cansaries_green() {
        let mut runner = FakeRunner::default();
        runner.then(status_out(7)); // probe
        runner.then(ok()); // update
        runner.then(ok()); // embed
        runner.then(zero_pending_out()); // recheck
        let dir = tmp_dir("green");
        let verdict = refresh(&mut runner, &cfg_in(&dir));
        match &verdict {
            RefreshVerdict::Refreshed {
                before,
                after,
                steps,
                lock_stolen,
            } => {
                assert_eq!(before.pending_embed, 7);
                assert_eq!(after.pending_embed, 0);
                assert_eq!(steps.len(), 2);
                assert_eq!(steps[0].phase, "update");
                assert_eq!(steps[1].phase, "embed");
                assert!(!lock_stolen);
            }
            other => panic!("expected Refreshed, got {other:?}"),
        }
        assert_eq!(
            runner.calls,
            vec![
                vec!["status".to_string()],
                vec!["update".to_string()],
                vec!["embed".to_string()],
                vec!["status".to_string()],
            ]
        );
        assert!(
            !dir.join("refresh.lock").exists(),
            "lock must be released on Green"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn canary_red_when_pending_survives_refresh() {
        let mut runner = FakeRunner::default();
        runner.then(status_out(7));
        runner.then(ok());
        runner.then(ok());
        runner.then(status_out(3)); // embed "succeeded" but pending remains
        let dir = tmp_dir("red");
        let verdict = refresh(&mut runner, &cfg_in(&dir));
        match &verdict {
            RefreshVerdict::Red(why) => assert!(why.contains("pending-embed still 3")),
            other => panic!("expected Red, got {other:?}"),
        }
        assert!(
            !dir.join("refresh.lock").exists(),
            "lock must be released on Red"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn busy_lock_short_circuits_before_any_mutation() {
        let mut runner = FakeRunner::default();
        runner.on("status", status_out(7)); // stale → wants refresh
        let dir = tmp_dir("busylock");
        let path = dir.join("refresh.lock");
        let tmp = sibling_tmp(&path);
        fs::write(&tmp, format!("999999\n{}\n", now_secs())).unwrap();
        fs::rename(&tmp, &path).unwrap();
        let verdict = refresh(&mut runner, &cfg_in(&dir));
        match verdict {
            RefreshVerdict::Busy {
                pid: Some(999_999), ..
            } => {}
            other => panic!("expected Busy, got {other:?}"),
        }
        assert_eq!(
            runner.calls,
            vec![vec!["status".to_string()]],
            "no update/embed while busy"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn stale_age_alone_triggers_refresh() {
        let mut runner = FakeRunner::default();
        runner.then(out(&status_text(Some("0"), "2d"))); // pending 0, index 2 days old
        runner.then(ok());
        runner.then(ok());
        runner.then(zero_pending_out());
        let dir = tmp_dir("age");
        let verdict = refresh(&mut runner, &cfg_in(&dir));
        assert!(matches!(verdict, RefreshVerdict::Refreshed { .. }));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn update_timeout_is_red() {
        let mut runner = FakeRunner::default();
        runner.then(status_out(7));
        runner.then(Outcome {
            code: None,
            timed_out: true,
            ..ok()
        });
        let dir = tmp_dir("timeout");
        let verdict = refresh(&mut runner, &cfg_in(&dir));
        match &verdict {
            RefreshVerdict::Red(why) => assert!(why.contains("update killed at")),
            other => panic!("expected Red, got {other:?}"),
        }
        assert!(
            !dir.join("refresh.lock").exists(),
            "lock released after timeout"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn embed_nonzero_is_red() {
        let mut runner = FakeRunner::default();
        runner.then(status_out(7));
        runner.then(ok()); // update fine
        runner.then(Outcome {
            code: Some(1),
            stderr: "gguf oom".into(),
            ..ok()
        });
        let dir = tmp_dir("nonzero");
        let verdict = refresh(&mut runner, &cfg_in(&dir));
        match &verdict {
            RefreshVerdict::Red(why) => assert!(why.contains("embed exited 1")),
            other => panic!("expected Red, got {other:?}"),
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn spawn_failure_is_degraded_never_red() {
        let mut runner = FakeRunner::default();
        runner.then(Outcome {
            code: None,
            stderr: "spawn failed: node not found".into(),
            ..ok()
        });
        let dir = tmp_dir("degraded");
        let verdict = refresh(&mut runner, &cfg_in(&dir));
        match &verdict {
            RefreshVerdict::Degraded(why) => assert!(why.contains("node not found")),
            other => panic!("expected Degraded, got {other:?}"),
        }
        assert!(!dir.join("refresh.lock").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unparseable_probe_is_red() {
        let mut runner = FakeRunner::default();
        runner.then(out("qmd: unknown command 'status'"));
        let dir = tmp_dir("badparse");
        let verdict = refresh(&mut runner, &cfg_in(&dir));
        assert!(matches!(verdict, RefreshVerdict::Red(_)));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn seq_fake_runner_orders_by_call_not_subcommand() {
        // The whole point of `then()`: two `status` calls with different
        // bodies in one script.
        let mut runner = FakeRunner::default();
        runner.then(status_out(9));
        runner.then(status_out(1));
        let cfg = cfg_in(&tmp_dir("seq"));
        assert_eq!(probe(&mut runner, &cfg).unwrap().pending_embed, 9);
        assert_eq!(probe(&mut runner, &cfg).unwrap().pending_embed, 1);
    }
}
