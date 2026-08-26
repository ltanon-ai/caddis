//! writer.rs — the remember() WRITE FLOW (P3 slice (a) final increment,
//! quorum-ratified contract in state/briefs/caddis-remember-quorum/VERDICT.md).
//!
//! Composition of the tested substrate (remember.rs):
//! - **I2+ write lock**: one writer per collection root. The lock file
//!   carries `{pid, created_ts, monotonic seq}`. Stealing requires BOTH
//!   age > steal floor AND proven holder death (`winprobe` — exit code
//!   != 259, or pid-reuse by creation-time compare). Unprovable = busy.
//! - **I5+ sandbox**: the target root is canonicalized and prefix-matched
//!   against the REGISTERED collection roots, and every path component is
//!   asserted symlink-free, BEFORE any write. qmd trusts any path (F1) —
//!   containment is this organ's job, not the indexer's.
//! - **I3+ head-linearity**: a `supersedes`/`retracts` target must be a
//!   current leaf by a FRESH chain scan under the lock — the warden stays
//!   a generic law scanner; chain knowledge lives here.
//! - **I1 flow shape**: the warden judges the byte-exact draft (frame
//!   content); only `warden_seq` + `warden_tx_hash` derive post-verdict.
//!   The draft is never an on-disk artifact pre-verdict — there is no
//!   window in which an unjudged doc exists on disk.
//! - **I4 stamps**: `warden_tx_hash` = the warden ledger row's own id
//!   (`wardn{:016x}` of FNV-1a over `command\ncontent`) — the organ builds
//!   the exact frame the warden records, so the stamp is a row pointer,
//!   not a self-invented hash.
//!
//! Fail-closed everywhere: timeout, nonzero exit, or unparseable warden
//! output BLOCKS the write. `allow` with `seq == 0` = verdict ran but the
//! ledger row was NOT recorded (warden's documented fail-open record leg)
//! — no audit anchor exists, so the write is refused (`Unrecorded`).

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use crate::exec::{Job, Runner};
use crate::json::{self, Value};
use crate::remember::{self, MemoryDoc};
use crate::winprobe::{self, Probe};

/// The tool/command names the warden sees for a memory write.
pub const TOOL: &str = "caddis-memory";
pub const COMMAND: &str = "remember";

/// Lock file inside the collection root. No `.md` extension: qmd indexes
/// markdown patterns only, so the lock can never enter the index.
pub const LOCK_NAME: &str = ".remember-lock";

/// The organ-owned heads view: current chain tip per docid. A view, never
/// the decision source — leaf-ness is re-proven by fresh scan under the lock.
pub const HEADS_NAME: &str = "active_heads.json";

/// Temp artifact for the atomic final write. Extension-less so a crash
/// between write and rename can never leave an indexable corpse.
const TMP_PREFIX: &str = ".remember-tmp-";

// ---------------------------------------------------------------------------
// Config + errors
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct RememberConfig {
    /// Warden launcher (program + prefix args), stdin-frame mode.
    pub warden_launcher: Vec<String>,
    pub warden_timeout: Duration,
    /// REGISTERED collection roots (I5+). Writes are allowed only under one
    /// of these; the registry's pairwise non-overlap law governs the set.
    pub roots: Vec<PathBuf>,
    /// Steal age floor: max(2 × watchdog interval, 60 s) per amendment I2+.
    pub steal_age_floor: Duration,
}

impl RememberConfig {
    /// The verdict's steal floor from the refresh watchdog interval.
    pub fn steal_floor(watchdog_interval: Duration) -> Duration {
        Duration::from_secs(60).max(watchdog_interval.saturating_mul(2))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RememberError {
    /// Frontmatter carries no usable title — no slug, no filename.
    Slug(String),
    /// Target root is not inside a registered collection root (I5+), or a
    /// path component is a symlink, or a root is missing.
    Sandbox(String),
    /// Another live writer holds the lock (I2+).
    Busy { pid: Option<u32>, age_secs: Option<u64> },
    /// Supersede/retract target is not a current leaf (I3+) — or the chain
    /// scan hit a file it cannot honestly parse (fail-closed).
    HeadNotLeaf(String),
    /// The warden verdict exists but the ledger row does not (seq 0).
    Unrecorded,
    /// Warden said no.
    Denied { verdict: String, reason: String, law: String },
    /// Warden ran but its answer is unreadable (timeout / exit / parse) —
    /// BLOCK, never a silent allow.
    WardenUnreadable(String),
    /// Final filename already exists — timestamps collided; never overwrite.
    Conflict,
    Io(String),
}

impl std::fmt::Display for RememberError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RememberError::Slug(w) => write!(f, "slug: {w}"),
            RememberError::Sandbox(w) => write!(f, "sandbox (I5+): {w}"),
            RememberError::Busy { pid, age_secs } => {
                write!(f, "write lock busy (I2+): pid={pid:?} age={age_secs:?}s")
            }
            RememberError::HeadNotLeaf(w) => write!(f, "head-linearity (I3+): {w}"),
            RememberError::Unrecorded => write!(f, "warden allow carried seq 0 — ledger row missing, no audit anchor"),
            RememberError::Denied { verdict, reason, law } => {
                write!(f, "warden {verdict}: {reason} (law: {law})")
            }
            RememberError::WardenUnreadable(w) => write!(f, "warden unreadable (BLOCK): {w}"),
            RememberError::Conflict => write!(f, "target filename already exists — refusing to overwrite"),
            RememberError::Io(w) => write!(f, "io: {w}"),
        }
    }
}

/// What a successful remember() proved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remembered {
    pub path: PathBuf,
    pub seq: u64,
    pub tx_hash: String,
}

// ---------------------------------------------------------------------------
// I4 — the warden ledger-row id, computed organ-side from the exact frame
// ---------------------------------------------------------------------------

/// FNV-1a 64 (the warden's row-id hash). Vectors are unit-tested so the two
/// implementations can never drift silently.
pub fn fnv1a64(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in data {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// The ledger row id the warden will attach to this verdict:
/// `wardn{:016x}` of FNV-1a over `"{command}\n{content}"` (wire payload).
pub fn warden_tx_id(command: &str, draft: &[u8]) -> String {
    let mut payload = Vec::with_capacity(command.len() + 1 + draft.len());
    payload.extend_from_slice(command.as_bytes());
    payload.push(b'\n');
    payload.extend_from_slice(draft);
    format!("wardn{:016x}", fnv1a64(&payload))
}

// ---------------------------------------------------------------------------
// I2+ — the write lock
// ---------------------------------------------------------------------------

/// Lock file body: `pid\ncreated_unix\nseq\n` (seq increments per
/// acquisition; steals included — the counter is telemetry, the LAW is
/// age + proven death).
fn write_lock_file(path: &Path, pid: u32, created_unix: u64, seq: u64) -> io::Result<()> {
    let tmp = sibling_tmp(path);
    fs::write(&tmp, format!("{pid}\n{created_unix}\n{seq}\n"))?;
    fs::rename(&tmp, path)
}

fn read_lock_file(path: &Path) -> io::Result<Option<(u32, u64, u64)>> {
    match fs::read_to_string(path) {
        Ok(text) => {
            let mut it = text.lines();
            let pid = it.next().and_then(|l| l.trim().parse::<u32>().ok());
            let ts = it.next().and_then(|l| l.trim().parse::<u64>().ok());
            let seq = it.next().and_then(|l| l.trim().parse::<u64>().ok());
            Ok(match (pid, ts, seq) {
                (Some(p), Some(t), Some(s)) => Some((p, t, s)),
                // Unparseable lock: our writes are tmp+rename, so on-disk
                // corruption predates this organ — stealable garbage.
                _ => None,
            })
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Pure steal decision (unit-testable without FFI). `probe` is what
/// `winprobe::holder_state` proved (None = unprovable).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockDecision {
    /// No holder (or unparseable residue) — take it.
    Free,
    /// Holder proven dead/reused AND old enough — take it.
    Steal { pid: Option<u32> },
    /// Live or unprovable or too young — refuse.
    Busy { pid: Option<u32>, age_secs: Option<u64> },
}

pub fn decide_lock(
    holder: Option<(u32, u64, u64)>,
    now_unix: u64,
    steal_age_floor_secs: u64,
    probe: Option<Probe>,
) -> LockDecision {
    match holder {
        None => LockDecision::Free,
        Some((pid, created, _seq)) => {
            let age = now_unix.saturating_sub(created);
            let dead = matches!(probe, Some(Probe::Dead) | Some(Probe::Reused));
            if age > steal_age_floor_secs && dead {
                LockDecision::Steal { pid: Some(pid) }
            } else {
                LockDecision::Busy { pid: Some(pid), age_secs: Some(age) }
            }
        }
    }
}

/// Acquire the write lock or report Busy. Every non-Busy path rewrites the
/// lock with OUR pid and a bumped seq before returning.
fn acquire_lock(lock_path: &Path, now_unix: u64, cfg: &RememberConfig) -> Result<(), RememberError> {
    let holder = read_lock_file(lock_path).map_err(|e| RememberError::Io(e.to_string()))?;
    let decision = match holder {
        None => LockDecision::Free,
        Some((pid, created, seq)) => decide_lock(
            Some((pid, created, seq)),
            now_unix,
            cfg.steal_age_floor.as_secs(),
            winprobe::holder_state(pid, created),
        ),
    };
    match decision {
        LockDecision::Busy { pid, age_secs } => Err(RememberError::Busy { pid, age_secs }),
        LockDecision::Free => {
            write_lock_file(lock_path, std::process::id(), now_unix, 1)
                .map_err(|e| RememberError::Io(e.to_string()))
        }
        LockDecision::Steal { .. } => {
            let prev_seq = holder.map(|(_, _, s)| s).unwrap_or(0);
            write_lock_file(lock_path, std::process::id(), now_unix, prev_seq + 1)
                .map_err(|e| RememberError::Io(e.to_string()))
        }
    }
}

/// Release only a lock that still carries OUR pid — one stolen from us
/// mid-run stays with its new owner.
fn release_lock(lock_path: &Path) {
    if let Ok(Some((pid, _, _))) = read_lock_file(lock_path) {
        if pid == std::process::id() {
            let _ = fs::remove_file(lock_path);
        }
    }
}

fn sibling_tmp(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(".tmp");
    PathBuf::from(s)
}

// ---------------------------------------------------------------------------
// I5+ — the organ-owned sandbox
// ---------------------------------------------------------------------------

/// Prove `root` sits inside a REGISTERED collection root and that the GIVEN
/// path contains no symlink component. Returns the canonical registered
/// root that owns the target.
///
/// Two independent legs (I5+):
/// 1. **Lexical walk** — `std::path::absolute(root)` WITHOUT symlink
///    resolution; every component must be a real directory. Walking the
///    canonical form is useless there: `canonicalize` already resolved the
///    symlinks, so the inherited canon-walk could never fire (proven dead
///    by `sandbox_rejects_symlink_component`).
/// 2. **Canonical prefix-match** — the resolved target must sit under a
///    registered root (catches `..` escapes and redirections).
fn assert_sandbox(root: &Path, roots: &[PathBuf]) -> Result<PathBuf, RememberError> {
    let lexical = std::path::absolute(root)
        .map_err(|e| RememberError::Sandbox(format!("target root not absolutizable: {e}")))?;
    let mut cur = PathBuf::new();
    for comp in lexical.components() {
        cur.push(comp.as_os_str());
        match comp {
            Component::Prefix(_) | Component::RootDir => continue,
            Component::Normal(_) => {}
            _ => {
                return Err(RememberError::Sandbox(format!(
                    "unexpected path component in {lexical:?}"
                )))
            }
        }
        // nvidia's escape: a symlinked component would make qmd index
        // arbitrary directories — refuse anything not provably real.
        let md = fs::symlink_metadata(&cur).map_err(|e| {
            RememberError::Sandbox(format!("component {} unreadable: {e}", cur.display()))
        })?;
        if md.file_type().is_symlink() {
            return Err(RememberError::Sandbox(format!(
                "symlink component in write path: {}",
                cur.display()
            )));
        }
    }
    let canon_target = fs::canonicalize(root)
        .map_err(|e| RememberError::Sandbox(format!("target root uncanonicalizable: {e}")))?;
    for reg in roots {
        let canon_reg = match fs::canonicalize(reg) {
            Ok(p) => p,
            Err(_) => continue, // unregistered/missing root cannot own anything
        };
        if canon_target == canon_reg || canon_target.starts_with(&canon_reg) {
            return Ok(canon_reg);
        }
    }
    Err(RememberError::Sandbox(format!(
        "target root {} is not under any registered collection root",
        root.display()
    )))
}

// ---------------------------------------------------------------------------
// I3+ — head-linearity (fresh chain scan)
// ---------------------------------------------------------------------------

/// Minimal frontmatter reader for chain edges. Returns None for files with
/// no `---` fence (foreign format — they carry no edges). A fenced file
/// whose front block has an unparseable line is an ERROR: an unreadable
/// edge could hide a supersede, and leaf-ness must be PROVEN, not assumed.
fn read_front(path: &Path) -> io::Result<Option<BTreeMap<String, String>>> {
    let text = fs::read_to_string(path)?;
    if !text.starts_with("---\n") {
        return Ok(None);
    }
    let mut front = BTreeMap::new();
    for line in text[4..].lines() {
        if line.trim() == "---" {
            return Ok(Some(front));
        }
        if line.trim().is_empty() {
            continue;
        }
        let (k, v) = line
            .split_once(": ")
            .ok_or_else(|| io::Error::other(format!("unparseable frontmatter line in {}: {line:?}", path.display())))?;
        front.insert(k.trim().to_string(), v.trim().to_string());
    }
    Err(io::Error::other(format!("unterminated frontmatter fence: {}", path.display())))
}

/// Is `target_docid` a current leaf: its file exists AND no doc in the root
/// supersedes or retracts it (fresh scan — never the heads view).
fn is_leaf(root: &Path, target_docid: &str) -> Result<bool, RememberError> {
    if !root.join(format!("{target_docid}.md")).is_file() {
        return Ok(false);
    }
    for entry in fs::read_dir(root).map_err(|e| RememberError::Io(e.to_string()))? {
        let entry = entry.map_err(|e| RememberError::Io(e.to_string()))?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        if path.file_stem().and_then(|s| s.to_str()) == Some(target_docid) {
            continue;
        }
        let front = read_front(&path).map_err(|e| RememberError::HeadNotLeaf(e.to_string()))?;
        if let Some(front) = front {
            for edge in ["supersedes", "retracts"] {
                if front.get(edge).map(|v| v.trim()) == Some(target_docid) {
                    return Ok(false);
                }
            }
        }
    }
    Ok(true)
}

// ---------------------------------------------------------------------------
// active_heads.json — the asserted view
// ---------------------------------------------------------------------------

fn heads_path(root: &Path) -> PathBuf {
    root.join(HEADS_NAME)
}

fn load_heads(root: &Path) -> Result<BTreeMap<String, String>, RememberError> {
    let path = heads_path(root);
    match fs::read_to_string(&path) {
        Ok(text) => {
            let v = json::parse(&text)
                .map_err(|e| RememberError::Io(format!("active_heads unparseable: {e:?}")))?;
            let obj = v.as_obj().ok_or_else(|| {
                RememberError::Io("active_heads.json: top level must be an object".into())
            })?;
            let mut m = BTreeMap::new();
            for (k, val) in obj {
                let s = val
                    .as_str()
                    .ok_or_else(|| RememberError::Io(format!("active_heads.json: {k} must be a string")))?;
                m.insert(k.clone(), s.to_string());
            }
            Ok(m)
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(e) => Err(RememberError::Io(e.to_string())),
    }
}

fn save_heads(root: &Path, heads: &BTreeMap<String, String>) -> Result<(), RememberError> {
    let obj: Vec<(String, Value)> = heads
        .iter()
        .map(|(k, v)| (k.clone(), Value::Str(v.clone())))
        .collect();
    let text = json::to_string(&Value::Obj(obj));
    let tmp = sibling_tmp(&heads_path(root));
    fs::write(&tmp, text).map_err(|e| RememberError::Io(e.to_string()))?;
    fs::rename(&tmp, heads_path(root)).map_err(|e| RememberError::Io(e.to_string()))
}

// ---------------------------------------------------------------------------
// The flow
// ---------------------------------------------------------------------------

/// Canonical forward-slash form (registry precedent): the path string the
/// warden frame and the ledger row carry must be stable across platforms.
fn fwd(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// One warden-gated memory write. See the module docs for the law mapping.
pub fn remember(
    runner: &mut dyn Runner,
    cfg: &RememberConfig,
    doc: MemoryDoc,
    root: &Path,
    now_unix: u64,
) -> Result<Remembered, RememberError> {
    // Pre-lock gates (cheap, read-only): slug, sandbox.
    let title = doc
        .front
        .get("title")
        .ok_or_else(|| RememberError::Slug("frontmatter has no title".into()))?;
    let slug = remember::slugify(title).map_err(RememberError::Slug)?;
    let fname = remember::filename(now_unix, &slug);
    let final_path = root.join(&fname);
    assert_sandbox(root, &cfg.roots)?;

    if final_path.exists() {
        return Err(RememberError::Conflict);
    }

    let lock_path = root.join(LOCK_NAME);
    acquire_lock(&lock_path, now_unix, cfg)?;

    let outcome = locked_write(runner, cfg, doc, root, &fname, &final_path);
    release_lock(&lock_path);
    outcome
}

/// Everything that happens while holding the write lock. On ANY error path
/// the caller still releases the lock; no temp file may survive.
fn locked_write(
    runner: &mut dyn Runner,
    cfg: &RememberConfig,
    doc: MemoryDoc,
    root: &Path,
    fname: &str,
    final_path: &Path,
) -> Result<Remembered, RememberError> {
    // I3+: supersede/retract targets must be current leaves — fresh chain
    // read under the lock closes the concurrent-supersede fork race.
    let mut target_docid: Option<String> = None;
    for edge in ["supersedes", "retracts"] {
        if let Some(v) = doc.front.get(edge) {
            let v = v.trim();
            if v.is_empty() {
                continue;
            }
            if !is_leaf(root, v)? {
                return Err(RememberError::HeadNotLeaf(format!(
                    "target {v:?} is not a current leaf (missing, or already superseded/retracted)"
                )));
            }
            target_docid = Some(v.to_string());
        }
    }

    // I1: the warden judges the byte-exact draft.
    let draft = doc.draft_bytes();
    let tx = warden_tx_id(COMMAND, &draft);
    let frame = remember::encode_frame(TOOL, COMMAND, &fwd(final_path), &draft);

    let job = Job {
        launcher: cfg.warden_launcher.clone(),
        args: Vec::new(),
        workdir: None,
        timeout: cfg.warden_timeout,
        stdin_data: Some(frame),
    };
    let out = runner.run(&job);
    if out.timed_out {
        return Err(RememberError::WardenUnreadable("warden timed out".into()));
    }
    if out.code != Some(0) {
        return Err(RememberError::WardenUnreadable(format!(
            "warden exit {:?}: {}",
            out.code,
            out.stderr.trim()
        )));
    }
    let verdict =
        remember::parse_verdict(out.stdout.trim()).map_err(RememberError::WardenUnreadable)?;
    if !verdict.allow {
        return Err(RememberError::Denied {
            verdict: verdict.verdict,
            reason: verdict.reason,
            law: verdict.law,
        });
    }
    if verdict.seq == 0 {
        return Err(RememberError::Unrecorded);
    }

    // I4: the only post-verdict mutations. Write final atomically.
    let mut stamped = doc.clone();
    stamped.apply_stamps(verdict.seq, &tx);
    let tmp = root.join(format!("{}{}-{}", TMP_PREFIX, std::process::id(), verdict.seq));
    let write_result = (|| -> Result<(), RememberError> {
        let mut f = fs::File::create(&tmp).map_err(|e| RememberError::Io(e.to_string()))?;
        f.write_all(stamped.render().as_bytes())
            .and_then(|_| f.sync_all())
            .map_err(|e| RememberError::Io(e.to_string()))?;
        drop(f);
        if final_path.exists() {
            return Err(RememberError::Conflict);
        }
        fs::rename(&tmp, final_path).map_err(|e| RememberError::Io(e.to_string()))
    })();
    if let Err(e) = write_result {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }

    // Heads view: the new doc is the tip; a superseded/retracted docid stops
    // being one.
    let docid = fname.trim_end_matches(".md").to_string();
    let mut heads = load_heads(root)?;
    if let Some(t) = &target_docid {
        heads.remove(t);
    }
    heads.insert(docid, fname.to_string());
    save_heads(root, &heads)?;

    Ok(Remembered { path: final_path.to_path_buf(), seq: verdict.seq, tx_hash: tx })
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::testing::FakeRunner;
    use crate::exec::Outcome;
    use std::time::Duration;

    fn tmp_root(tag: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("caddis-remember-{}-{}", std::process::id(), tag));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn cfg_for(root: &Path) -> RememberConfig {
        RememberConfig {
            warden_launcher: vec!["caddis-warden".into()],
            warden_timeout: Duration::from_secs(30),
            roots: vec![root.to_path_buf()],
            steal_age_floor: Duration::from_secs(60),
        }
    }

    fn allow_json(seq: u64) -> String {
        json::to_string(&json::Value::Obj(vec![
            ("verdict".into(), Value::Str("allow".into())),
            ("reason".into(), Value::Str(String::new())),
            ("law".into(), Value::Str(String::new())),
            ("seq".into(), Value::Num(seq as f64)),
        ]))
    }

    fn deny_json() -> String {
        json::to_string(&json::Value::Obj(vec![
            ("verdict".into(), Value::Str("deny".into())),
            ("reason".into(), Value::Str("outside sandbox".into())),
            ("law".into(), Value::Str("L5".into())),
            ("seq".into(), Value::Num(7.0)),
        ]))
    }

    fn outcome(stdout: String) -> Outcome {
        Outcome {
            code: Some(0),
            timed_out: false,
            stdout,
            stderr: String::new(),
            duration: Duration::from_millis(5),
            stdout_truncated: false,
            stderr_truncated: false,
        }
    }

    fn doc(title: &str, body: &str) -> MemoryDoc {
        MemoryDoc {
            front: BTreeMap::from([
                ("title".to_string(), title.to_string()),
                ("kind".to_string(), "memory".to_string()),
            ]),
            body: body.to_string(),
        }
    }

    // -- fnv1a vectors (drift guard vs the warden's row ids) --

    #[test]
    fn fnv1a_vectors() {
        assert_eq!(fnv1a64(b""), 0xcbf29ce484222325);
        assert_eq!(fnv1a64(b"a"), 0xaf63dc4c8601ec8c);
        assert_eq!(fnv1a64(b"foobar"), 0x85944171f73967e8);
    }

    // -- I2+ decision matrix --

    #[test]
    fn lock_decision_matrix() {
        let now = 1_000_000_u64;
        // free
        assert_eq!(decide_lock(None, now, 60, None), LockDecision::Free);
        // young + alive → busy
        assert_eq!(
            decide_lock(Some((10, now - 5, 1)), now, 60, Some(Probe::Alive)),
            LockDecision::Busy { pid: Some(10), age_secs: Some(5) }
        );
        // old + alive (259) → busy: death NOT proven
        assert_eq!(
            decide_lock(Some((10, now - 500, 1)), now, 60, Some(Probe::Alive)),
            LockDecision::Busy { pid: Some(10), age_secs: Some(500) }
        );
        // old + dead → steal
        assert_eq!(
            decide_lock(Some((10, now - 500, 1)), now, 60, Some(Probe::Dead)),
            LockDecision::Steal { pid: Some(10) }
        );
        // old + reused pid → steal (the generational leg)
        assert_eq!(
            decide_lock(Some((10, now - 500, 1)), now, 60, Some(Probe::Reused)),
            LockDecision::Steal { pid: Some(10) }
        );
        // old + unprovable → busy (fail-closed: FFI refusal never steals)
        assert_eq!(
            decide_lock(Some((10, now - 500, 1)), now, 60, None),
            LockDecision::Busy { pid: Some(10), age_secs: Some(500) }
        );
        // young + dead → busy: age is not optional
        assert_eq!(
            decide_lock(Some((10, now - 5, 1)), now, 60, Some(Probe::Dead)),
            LockDecision::Busy { pid: Some(10), age_secs: Some(5) }
        );
    }

    // -- I5+ sandbox --

    #[test]
    fn sandbox_accepts_registered_root() {
        let root = tmp_root("sb-ok");
        assert!(assert_sandbox(&root, std::slice::from_ref(&root)).is_ok());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn sandbox_rejects_foreign_root() {
        let a = tmp_root("sb-a");
        let b = tmp_root("sb-b");
        let err = assert_sandbox(&b, std::slice::from_ref(&a)).unwrap_err();
        assert!(matches!(err, RememberError::Sandbox(_)), "{err}");
        let _ = fs::remove_dir_all(&a);
        let _ = fs::remove_dir_all(&b);
    }

    #[test]
    fn sandbox_rejects_missing_root() {
        let err = assert_sandbox(Path::new("Z:/definitely/not/here"), &[]).unwrap_err();
        assert!(matches!(err, RememberError::Sandbox(_)));
    }

    #[cfg(windows)]
    #[test]
    fn sandbox_rejects_symlink_component() {
        let root = tmp_root("sb-sym");
        let real = root.join("real");
        fs::create_dir_all(&real).unwrap();
        let link = root.join("link");
        if std::os::windows::fs::symlink_dir(&real, &link).is_err() {
            eprintln!("symlink creation refused (no dev-mode privilege) — case skipped honestly");
            let _ = fs::remove_dir_all(&root);
            return;
        }
        let target = link.join("mem");
        fs::create_dir_all(&target).unwrap();
        let err = assert_sandbox(&target, std::slice::from_ref(&root)).unwrap_err();
        assert!(
            matches!(&err, RememberError::Sandbox(w) if w.contains("symlink")),
            "{err}"
        );
        let _ = fs::remove_dir_all(&root);
    }

    // -- I3+ head-linearity --

    fn write_doc(root: &Path, stem: &str, front: &[(&str, &str)], body: &str) {
        let mut d = doc("x", body);
        for (k, v) in front {
            d.front.insert(k.to_string(), v.to_string());
        }
        fs::write(root.join(format!("{stem}.md")), d.render()).unwrap();
    }

    #[test]
    fn leaf_law() {
        let root = tmp_root("leaf");
        write_doc(&root, "a", &[], "first");
        write_doc(&root, "b", &[("supersedes", "a")], "second");
        assert!(is_leaf(&root, "b").unwrap());
        assert!(!is_leaf(&root, "a").unwrap()); // superseded by b
        assert!(!is_leaf(&root, "missing").unwrap()); // must exist
        write_doc(&root, "c", &[("retracts", "b")], "third");
        assert!(!is_leaf(&root, "b").unwrap()); // retracted by c
        assert!(is_leaf(&root, "c").unwrap());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn unreadable_front_breaks_leaf_proof() {
        let root = tmp_root("leaf-fail");
        // The target must EXIST: is_leaf short-circuits Ok(false) on a
        // missing file, before the chain scan that reads the corrupt doc.
        write_doc(&root, "target", &[], "real leaf");
        fs::write(root.join("weird.md"), "---\nnot a frontmatter line\n---\n\nbody").unwrap();
        let err = is_leaf(&root, "target").unwrap_err();
        assert!(matches!(err, RememberError::HeadNotLeaf(_)), "{err}");
        let _ = fs::remove_dir_all(&root);
    }

    // -- the flow --

    #[test]
    fn full_write_lands_stamped_doc_and_heads() {
        let root = tmp_root("full");
        let mut fake = FakeRunner::default();
        fake.then(outcome(allow_json(42)));
        let d = doc("Golden Needle", "the needle was found");
        let now = 1_777_777_777_u64;

        let r = remember(&mut fake, &cfg_for(&root), d.clone(), &root, now).unwrap();

        // I3 filename law
        let expected_name = format!("{}Z-golden-needle.md", remember::utc_compact(now));
        assert_eq!(r.path, root.join(&expected_name));
        assert!(r.path.is_file());
        // I4 stamps
        assert_eq!(r.seq, 42);
        assert_eq!(r.tx_hash, warden_tx_id(COMMAND, &d.draft_bytes()));
        // I1 audit leg: strip stamps → the draft the warden allowed
        let on_disk = fs::read_to_string(&r.path).unwrap();
        let reparsed = parse_roundtrip(&on_disk);
        assert!(reparsed.front.contains_key("warden_seq"));
        assert_eq!(reparsed.front.get("warden_seq").unwrap(), "42");
        let mut stripped = reparsed.clone();
        stripped.front.remove("warden_seq");
        stripped.front.remove("warden_tx_hash");
        assert_eq!(stripped.render(), d.render());
        // heads view
        let heads = load_heads(&root).unwrap();
        assert_eq!(
            heads.get(expected_name.trim_end_matches(".md")),
            Some(&expected_name)
        );
        // lock released, no temp corpses
        assert!(!root.join(LOCK_NAME).exists());
        assert!(fs::read_dir(&root).unwrap().filter_map(|e| e.ok()).all(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            !n.starts_with(TMP_PREFIX) && !n.ends_with(".tmp")
        }));
        // the warden spawn shape: launcher, no args, frame on stdin
        assert_eq!(fake.jobs.len(), 1);
        let job = &fake.jobs[0];
        assert_eq!(job.launcher, vec!["caddis-warden".to_string()]);
        assert!(job.args.is_empty());
        let frame = job.stdin_data.as_ref().unwrap();
        assert!(frame.starts_with(format!("tool {}\n{}\n", TOOL.len(), TOOL).as_bytes()));
        let _ = fs::remove_dir_all(&root);
    }

    /// Re-parse a rendered doc back to MemoryDoc (test-only inverse).
    fn parse_roundtrip(text: &str) -> MemoryDoc {
        assert!(text.starts_with("---\n"));
        let mut front = BTreeMap::new();
        let mut body = String::new();
        let mut in_front = true;
        let mut seen_close = false;
        let mut first_body_line = true;
        for line in text[4..].lines() {
            if in_front {
                if line.trim() == "---" {
                    in_front = false;
                    seen_close = true;
                    first_body_line = true;
                    continue;
                }
                let (k, v) = line.split_once(": ").unwrap();
                front.insert(k.to_string(), v.to_string());
            } else if seen_close {
                // render() separates fence and body with exactly one blank
                // line — it is structure, not content.
                if first_body_line && line.is_empty() {
                    first_body_line = false;
                    continue;
                }
                first_body_line = false;
                body.push_str(line);
                body.push('\n');
            }
        }
        MemoryDoc { front, body: body.trim_end().to_string() }
    }

    #[test]
    fn supersede_moves_the_head() {
        let root = tmp_root("supersede");
        // chain: a is a leaf
        write_doc(&root, "a", &[], "first");
        let mut fake = FakeRunner::default();
        fake.then(outcome(allow_json(10)));
        let mut d = doc("Second Take", "v2");
        d.front.insert("supersedes".into(), "a".into());

        let r = remember(&mut fake, &cfg_for(&root), d, &root, 1_777_777_000).unwrap();

        let heads = load_heads(&root).unwrap();
        assert!(!heads.contains_key("a"), "superseded doc leaves the heads view");
        let our_docid = r
            .path
            .file_stem()
            .unwrap()
            .to_string_lossy()
            .to_string();
        assert_eq!(heads.get(&our_docid).map(|s| s.as_str()), r.path.file_name().unwrap().to_str());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn superseding_non_leaf_is_denied_before_warden() {
        let root = tmp_root("nonleaf");
        write_doc(&root, "a", &[], "first");
        write_doc(&root, "b", &[("supersedes", "a")], "second");
        let mut fake = FakeRunner::default();
        let mut d = doc("Fork Attempt", "tries to supersede a");
        d.front.insert("supersedes".into(), "a".into());

        let err = remember(&mut fake, &cfg_for(&root), d, &root, 1_777_777_000).unwrap_err();
        assert!(matches!(err, RememberError::HeadNotLeaf(_)), "{err}");
        assert!(fake.jobs.is_empty(), "leaf check must precede the warden call");
        assert!(!root.join(LOCK_NAME).exists(), "lock released on error");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn deny_verdict_leaves_no_trace() {
        let root = tmp_root("deny");
        let mut fake = FakeRunner::default();
        fake.then(outcome(deny_json()));
        let before: Vec<String> = fs::read_dir(&root).unwrap().filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().to_string()).collect();

        let err = remember(&mut fake, &cfg_for(&root), doc("Refused", "no"), &root, 1_777_777_000).unwrap_err();
        assert!(
            matches!(&err, RememberError::Denied { verdict, .. } if verdict == "deny"),
            "{err}"
        );

        let after: Vec<String> = fs::read_dir(&root).unwrap().filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().to_string()).collect();
        assert_eq!(before, after, "deny must not change the root");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn seq_zero_allow_is_unrecorded_and_blocks() {
        let root = tmp_root("seq0");
        let mut fake = FakeRunner::default();
        fake.then(outcome(allow_json(0)));
        let err = remember(&mut fake, &cfg_for(&root), doc("Ghost", "no"), &root, 1_777_777_000).unwrap_err();
        assert_eq!(err, RememberError::Unrecorded);
        assert!(!root.join(LOCK_NAME).exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn garbage_verdict_blocks() {
        let root = tmp_root("garbage");
        let mut fake = FakeRunner::default();
        fake.then(outcome("not json at all".into()));
        let err = remember(&mut fake, &cfg_for(&root), doc("G", "no"), &root, 1_777_777_000).unwrap_err();
        assert!(matches!(err, RememberError::WardenUnreadable(_)), "{err}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn nonzero_exit_blocks() {
        let root = tmp_root("exit1");
        let mut fake = FakeRunner::default();
        fake.then(Outcome {
            code: Some(1),
            timed_out: false,
            stdout: String::new(),
            stderr: "warden exploded".into(),
            duration: Duration::from_millis(3),
            stdout_truncated: false,
            stderr_truncated: false,
        });
        let err = remember(&mut fake, &cfg_for(&root), doc("E", "no"), &root, 1_777_777_000).unwrap_err();
        assert!(matches!(&err, RememberError::WardenUnreadable(w) if w.contains("exit")), "{err}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn busy_lock_refuses_without_warden_call() {
        let root = tmp_root("busy");
        let now = 1_777_777_000_u64;
        // a young lock naming OUR pid: provably alive → busy
        write_lock_file(&root.join(LOCK_NAME), std::process::id(), now - 1, 1).unwrap();
        let mut fake = FakeRunner::default();
        let err = remember(&mut fake, &cfg_for(&root), doc("B", "no"), &root, now).unwrap_err();
        assert!(matches!(err, RememberError::Busy { pid: Some(p), .. } if p == std::process::id()), "{err}");
        assert!(fake.jobs.is_empty());
        // the foreign lock must survive (still theirs)
        assert!(root.join(LOCK_NAME).exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn filename_collision_refuses() {
        let root = tmp_root("collide");
        let now = 1_777_777_000_u64;
        let name = format!("{}Z-collide-now.md", remember::utc_compact(now));
        fs::write(root.join(&name), "pre-existing").unwrap();
        let mut fake = FakeRunner::default();
        let err = remember(&mut fake, &cfg_for(&root), doc("Collide Now", "x"), &root, now).unwrap_err();
        assert_eq!(err, RememberError::Conflict);
        assert_eq!(fs::read_to_string(root.join(&name)).unwrap(), "pre-existing");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn foreign_root_write_is_sandbox_denied() {
        let inside = tmp_root("fr-inside");
        let outside = tmp_root("fr-outside");
        let mut fake = FakeRunner::default();
        let err = remember(&mut fake, &cfg_for(&inside), doc("S", "no"), &outside, 1_777_777_000).unwrap_err();
        assert!(matches!(err, RememberError::Sandbox(_)), "{err}");
        assert!(fake.jobs.is_empty());
        let _ = fs::remove_dir_all(&inside);
        let _ = fs::remove_dir_all(&outside);
    }
}
