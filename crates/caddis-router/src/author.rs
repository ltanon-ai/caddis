//! author.rs — P5 phase (a): the AUTHOR family (brief
//! `state/briefs/p5-authoring-room-brief.md` §9, ladder complete 6/6:
//! council 6/8 unanimous + Q-A/Q-B splits, quorum 3 seats 2026-08-28T08:42Z).
//!
//! The organ was code-complete through R5 but UNAUTHORABLE: `lanes.jsonl`,
//! `policy.json` and `warden.key` are operator-ruled files the organ could
//! read but not write. This module is the ONE write path for the first two
//! (mint stays the terminal-only `warden mint` — Q-A ruled 2:1 the panel
//! never gets a mint button). Every law here is a folded ruling:
//!
//! - **Operator-authored only** (§4.1): the CLI proposes (`--dry-run`,
//!   verbatim candidate text) and commits on the SAME argv minus the flag;
//!   the panel in phase (c) only relays both halves — it never validates
//!   organ formats (parse law lives in exactly ONE Rust implementation,
//!   §4.2: [`crate::registry::parse_registry`] / [`crate::policy_file::parse_policy`]).
//! - **Missing file = EMPTY ruling** (file-is-whole-policy law): the first
//!   `policy-set` writes exactly the ruled keys — never builtin defaults;
//!   the first `lanes-upsert` writes exactly the one lane.
//! - **The organ never writes past a defect**: a current file that does not
//!   parse is a refusal (defect class) with the parser's own finding.
//! - **Self-validation**: the write path ENCODES, then RE-PARSES its own
//!   candidate through the same loader before the bytes may exist on disk.
//!   What [`author_commit`] emits must load or it does not land.
//! - **Optimistic concurrency**: `--expect-prior <hash16>` is compared to
//!   sha256(current)[0..16]; a mismatch is a STALE proposal (refusal 1 —
//!   the panel re-renders from a fresh dry-run, it never blind-writes).
//! - **No-op refusal**: a write whose candidate bytes equal the current
//!   bytes changes nothing and is refused (exit 1) — a ruling is a change.
//! - **Last-lane refusal**: removing the final lane would emit a registry
//!   no loader accepts (zero entries is malformed by law); the operator
//!   removes such a file by hand. Same law reaches policy through the
//!   re-parse gate (a file with no `tier.<data_class>` ruled is malformed).
//! - **Crash order** (fail-closed direction): immutable per-write `.bak`
//!   named by prior-content-hash FIRST → tmp+rename (std rename is
//!   MOVEFILE_REPLACE_EXISTING on Windows) → journal append. An orphan
//!   `.bak` without a journal row is detectable and safe (the old content
//!   it holds is still superseded by the renamed file); a journal row
//!   without the file write cannot happen — the row is appended LAST.
//! - **Journal** (`author.jsonl` beside the target, same home as the
//!   ledger by default): append-only flat rows, seq = max+1 over PARSED
//!   rows (model-voice lesson: never the line count), O_EXCL lock +
//!   single `write_all` + `sync_data` (R6 append law), `actor` /
//!   `actor_kind` additive vocabulary (`terminal` today; `ticketActor`
//!   when the phase (b) queue carries it; a future T-27 token slots in
//!   WITHOUT journal migration — quorum fold 5).
//!
//! Phase boundary: `PEPWORLD_ROUTER_HOME` (unset = real home, set =
//! sandbox, never the inverse — fold 6) is the BRIDGE law, phase (d); this
//! module takes explicit paths so the dev bridge can pin the sandbox.

use std::collections::BTreeMap;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::lane::{DataClass, LaneTier};
use crate::ledger::{as_str, as_u64, esc, fit, get, now_iso, parse_object, Val, LOCK_WAIT};
use crate::lock::Lock;
use crate::policy::{RoutePolicy, DEFAULT_MIN_SAMPLES};
use crate::policy_file::{encode_policy, parse_policy};
use crate::registry::{encode_registry, parse_registry, LaneEntry, LaneRegistry};
use crate::sha256::{hex as hex64, sha256};

/// Same row-size law as the ledger: one journal append is ONE `write_all`
/// syscall — above this the call loops and tearing returns.
const ROW_CAP: usize = 4096;

/// One authored operation, fully validated (shape-level; semantic gates run
/// at mutation + re-parse).
#[derive(Debug, Clone, PartialEq)]
pub enum AuthorOp {
    /// Insert-or-replace by id (identity is the id — a re-rule replaces
    /// family/tier/cost wholesale).
    LanesUpsert {
        id: String,
        family: String,
        tier: LaneTier,
        cost: f64,
    },
    LanesRemove {
        id: String,
    },
    PolicySet {
        key: String,
        value: PolicyValue,
    },
    PolicyUnset {
        key: String,
    },
}

impl AuthorOp {
    /// The journal/CLI word for this op.
    pub fn word(&self) -> &'static str {
        match self {
            AuthorOp::LanesUpsert { .. } => "lanes-upsert",
            AuthorOp::LanesRemove { .. } => "lanes-remove",
            AuthorOp::PolicySet { .. } => "policy-set",
            AuthorOp::PolicyUnset { .. } => "policy-unset",
        }
    }

    /// Which file family this op authors.
    pub fn targets_policy(&self) -> bool {
        matches!(
            self,
            AuthorOp::PolicySet { .. } | AuthorOp::PolicyUnset { .. }
        )
    }

    /// Construct a validated `policy-set` op from raw CLI strings. The key
    /// vocabulary and value grammar are checked HERE (one validation home);
    /// range laws (floor in (0..=1], ceiling positive) fire at
    /// [`RoutePolicy::validate`] and again at the re-parse gate.
    pub fn policy_set(key: &str, value: &str) -> Result<AuthorOp, String> {
        let v = PolicyValue::parse(key, value)?;
        Ok(AuthorOp::PolicySet {
            key: key.to_string(),
            value: v,
        })
    }

    /// Construct a validated `policy-unset` op (shape only — presence is a
    /// REFUSAL at prepare time, not a usage defect).
    pub fn policy_unset(key: &str) -> Result<AuthorOp, String> {
        validate_key_shape(key)?;
        Ok(AuthorOp::PolicyUnset {
            key: key.to_string(),
        })
    }
}

/// A typed policy value — the grammar behind `--value` per key kind.
#[derive(Debug, Clone, PartialEq)]
pub enum PolicyValue {
    Floor(f64),
    Ceiling(f64),
    Tiers(Vec<LaneTier>),
    MinSamples(u32),
}

impl PolicyValue {
    fn parse(key: &str, value: &str) -> Result<PolicyValue, String> {
        if let Some(class) = key.strip_prefix("floor.") {
            if class.is_empty() {
                return Err("key \"floor.\" has no task class".into());
            }
            let n: f64 = value
                .parse()
                .map_err(|_| format!("floor.{class}: value {value:?} is not a number"))?;
            if !n.is_finite() {
                return Err(format!("floor.{class}: value must be finite (got {n})"));
            }
            Ok(PolicyValue::Floor(n))
        } else if let Some(class) = key.strip_prefix("ceiling.") {
            if class.is_empty() {
                return Err("key \"ceiling.\" has no task class".into());
            }
            let n: f64 = value
                .parse()
                .map_err(|_| format!("ceiling.{class}: value {value:?} is not a number"))?;
            if !n.is_finite() {
                return Err(format!("ceiling.{class}: value must be finite (got {n})"));
            }
            Ok(PolicyValue::Ceiling(n))
        } else if let Some(word) = key.strip_prefix("tier.") {
            let dc = DataClass::parse(word).ok_or_else(|| {
                format!(
                    "key {key:?}: unknown data class {word:?} (vocabulary: secret|pii|internal|public)"
                )
            })?;
            let _ = dc; // shape check only; the value carries the tiers
            let mut tiers = Vec::new();
            for w in value.split(',') {
                let t = w.trim();
                if t.is_empty() {
                    return Err(format!("tier.{word}: empty tier word in {value:?}"));
                }
                let tier = LaneTier::parse(t).ok_or_else(|| {
                    format!(
                        "tier.{word}: unknown tier {t:?} (taxonomy: local|free|mid|premium; droid is refused — O2)"
                    )
                })?;
                tiers.push(tier);
            }
            if tiers.is_empty() {
                return Err(format!("tier.{word}: at least one tier required"));
            }
            Ok(PolicyValue::Tiers(tiers))
        } else if key == "min_samples" {
            let n: u32 = value
                .parse()
                .map_err(|_| format!("min_samples: value {value:?} is not an integer >= 1"))?;
            if n < 1 {
                return Err(format!("min_samples: must be >= 1 (got {n})"));
            }
            Ok(PolicyValue::MinSamples(n))
        } else {
            Err(format!(
                "unknown key {key:?} (vocabulary: floor.<class> | ceiling.<class> | tier.<data_class> | min_samples)"
            ))
        }
    }
}

/// Key-shape law shared by set and unset.
fn validate_key_shape(key: &str) -> Result<(), String> {
    if key == "min_samples" {
        return Ok(());
    }
    for (pfx, what) in [("floor.", "task class"), ("ceiling.", "task class")] {
        if let Some(class) = key.strip_prefix(pfx) {
            if class.is_empty() {
                return Err(format!("key {key:?} has no {what}"));
            }
            return Ok(());
        }
    }
    if let Some(word) = key.strip_prefix("tier.") {
        let dc = DataClass::parse(word).ok_or_else(|| {
            format!(
                "key {key:?}: unknown data class {word:?} (vocabulary: secret|pii|internal|public)"
            )
        })?;
        let _ = dc;
        return Ok(());
    }
    Err(format!(
        "unknown key {key:?} (vocabulary: floor.<class> | ceiling.<class> | tier.<data_class> | min_samples)"
    ))
}

/// Honest failure taxonomy. `Refusal` = exit 1 (nothing written; the
/// proposal was stale, a no-op, or removes the last of something); `Defect`
/// = exit 2 (usage, malformed current file, self-validation, environment).
#[derive(Debug, Clone, PartialEq)]
pub enum AuthorErr {
    Refusal(String),
    Defect(String),
}

impl AuthorErr {
    /// The exit-code-class word for CLI mapping.
    pub fn is_refusal(&self) -> bool {
        matches!(self, AuthorErr::Refusal(_))
    }
    pub fn message(&self) -> &str {
        match self {
            AuthorErr::Refusal(m) | AuthorErr::Defect(m) => m,
        }
    }
}

/// Everything a proposal learned in steps 1-6 — the dry-run verdict AND the
/// commit input. `candidate` is the VERBATIM text the write would land.
#[derive(Debug, Clone, PartialEq)]
pub struct AuthorPlan {
    pub op: AuthorOp,
    /// Human one-liner ("added lane gemini" / "set floor.chair=0.8").
    pub summary: String,
    /// The file the write targets.
    pub target: PathBuf,
    /// The journal this write appends to (beside the target — the same
    /// home as the ledger in the default flow).
    pub journal: PathBuf,
    /// sha256 of the CURRENT bytes; `None` = file absent (empty ruling).
    pub prior_hash: Option<String>,
    /// sha256 of the candidate bytes (always present — a write has content).
    pub next_hash: String,
    /// The immutable backup file name (`<file>.bak.<prior-hash16>`);
    /// `None` when there is no prior content to back up.
    pub bak: Option<String>,
    /// The verbatim candidate text (trailing newline included).
    pub candidate: String,
    /// The prior bytes (held for the .bak write; not part of any equality).
    pub prior: Option<Vec<u8>>,
}

/// Steps 1-6 of the brief's write algorithm: resolve, read, decode, mutate,
/// encode, self-validate. NO bytes are written — this IS the dry-run, and
/// the confirm is `author_commit(plan)` on the same plan.
///
/// `expect_prior`: the 16-hex prefix the proposal was rendered against;
/// compared case-insensitively against sha256(current)[0..16] (`"absent"`
/// when the file does not exist). A mismatch is a stale proposal.
pub fn author_prepare(
    op: AuthorOp,
    target: &Path,
    expect_prior: Option<&str>,
) -> Result<AuthorPlan, AuthorErr> {
    let fname = target
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| AuthorErr::Defect(format!("target has no file name: {}", target.display())))?
        .to_string();
    let journal = target
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("author.jsonl");

    // 1-2. Read current bytes (missing = empty ruling, NOT defaults).
    let prior: Option<Vec<u8>> = match fs::read(target) {
        Ok(b) => Some(b),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            return Err(AuthorErr::Defect(format!(
                "cannot read {}: {e}",
                target.display()
            )))
        }
    };
    let prior_hash = prior.as_ref().map(|b| hex64(&sha256(b)));
    let prior_hash16 = prior_hash
        .as_deref()
        .map(|h| h[..16].to_string())
        .unwrap_or_else(|| "absent".to_string());

    // Optimistic concurrency: the proposal may only commit against the
    // content it was rendered from.
    if let Some(expect) = expect_prior {
        if !expect.eq_ignore_ascii_case(&prior_hash16) {
            return Err(AuthorErr::Refusal(format!(
                "stale proposal: --expect-prior {expect} but current content hash is {prior_hash16} — re-propose from a fresh dry-run"
            )));
        }
    }

    let (candidate, summary) = if op.targets_policy() {
        prepare_policy(&op, prior.as_deref(), &fname)?
    } else {
        prepare_lanes(&op, prior.as_deref(), &fname)?
    };

    // No-op law: identical bytes is not a ruling.
    if prior.as_deref() == Some(candidate.as_bytes()) {
        return Err(AuthorErr::Refusal(format!(
            "no-op write refused — {fname} content is unchanged"
        )));
    }

    let next_hash = hex64(&sha256(candidate.as_bytes()));
    let bak = prior_hash
        .as_ref()
        .map(|h| format!("{fname}.bak.{}", &h[..16]));
    Ok(AuthorPlan {
        op,
        summary,
        target: target.to_path_buf(),
        journal,
        prior_hash,
        next_hash,
        bak,
        candidate,
        prior,
    })
}

/// Lanes mutation + encode + self-validation.
fn prepare_lanes(
    op: &AuthorOp,
    prior: Option<&[u8]>,
    fname: &str,
) -> Result<(String, String), AuthorErr> {
    let current: Option<LaneRegistry> = match prior {
        None => None,
        Some(b) => Some(parse_registry(&String::from_utf8_lossy(b)).map_err(registry_defect)?),
    };
    let mut entries: Vec<LaneEntry> = current
        .as_ref()
        .map(|r| r.entries().to_vec())
        .unwrap_or_default();
    let summary = match op {
        AuthorOp::LanesUpsert {
            id,
            family,
            tier,
            cost,
        } => {
            let entry = LaneEntry {
                id: id.clone(),
                family: family.clone(),
                tier: *tier,
                cost_per_task_usd: *cost,
            };
            let verb = if entries.iter().any(|l| l.id == *id) {
                entries.retain(|l| l.id != *id);
                entries.push(entry);
                "updated lane"
            } else {
                entries.push(entry);
                "added lane"
            };
            format!("{verb} {id}")
        }
        AuthorOp::LanesRemove { id } => {
            let before = entries.len();
            entries.retain(|l| l.id != *id);
            if entries.len() == before {
                return Err(AuthorErr::Refusal(format!(
                    "no lane id {id:?} in {fname} — nothing to remove"
                )));
            }
            if entries.is_empty() {
                return Err(AuthorErr::Refusal(format!(
                    "refusing to remove the LAST lane — a registry that exists must name at least one lane; remove {fname} by hand instead"
                )));
            }
            format!("removed lane {id}")
        }
        other => {
            return Err(AuthorErr::Defect(format!(
                "internal: lanes prepare got {}",
                other.word()
            )))
        }
    };
    let Some(reg) = LaneRegistry::from_entries(entries) else {
        return Err(AuthorErr::Defect(
            "internal: mutation left the registry empty".into(),
        ));
    };
    let candidate = encode_registry(&reg);
    // Self-validation: what the write path emits must load before it may
    // exist on disk.
    let reparsed = parse_registry(&candidate)
        .map_err(|e| AuthorErr::Defect(format!("self-validation refused the candidate: {e:?}")))?;
    if reparsed != reg {
        return Err(AuthorErr::Defect(
            "self-validation refused the candidate: re-parse is not the mutated registry".into(),
        ));
    }
    Ok((candidate, summary))
}

/// Policy mutation + encode + self-validation.
fn prepare_policy(
    op: &AuthorOp,
    prior: Option<&[u8]>,
    fname: &str,
) -> Result<(String, String), AuthorErr> {
    // An EMPTY policy (bootstrap): no floors, no ceilings, no tiers — the
    // file-is-whole-policy law; the first ruled keys are the whole file.
    let empty = || {
        RoutePolicy::from_parts(
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            DEFAULT_MIN_SAMPLES,
        )
    };
    let text = prior.map(|b| String::from_utf8_lossy(b).to_string());
    let current: RoutePolicy = match &text {
        None => empty(),
        Some(t) => parse_policy(t).map_err(|e| {
            AuthorErr::Defect(format!(
                "current {fname} is malformed — the organ never writes past a defect: {:?}",
                policy_msg(&e)
            ))
        })?,
    };
    // Raw-key presence: the decoded policy cannot distinguish an absent
    // `min_samples` from the default — the FILE's own keys can.
    let raw_keys: Vec<String> = match &text {
        None => Vec::new(),
        Some(t) => parse_object(t)
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default(),
    };

    let (next, summary) = match op {
        AuthorOp::PolicySet { key, value } => {
            let mut p = current.clone();
            let summary = apply_set(&mut p, key, value)?;
            (p, format!("set {summary}"))
        }
        AuthorOp::PolicyUnset { key } => {
            if !raw_keys.iter().any(|k| k == key) {
                return Err(AuthorErr::Refusal(format!(
                    "key {key:?} not present in {fname} — nothing to unset"
                )));
            }
            let mut floors = current.floors().clone();
            let mut ceilings = current.ceilings().clone();
            let mut tiers = current.tier_allow().clone();
            let min_samples = if key == "min_samples" {
                // Omitted-key semantics: an unset min_samples rules
                // nothing, and nothing-ruled keeps the DEFAULT — the
                // encoder writes the effective value either way.
                DEFAULT_MIN_SAMPLES
            } else {
                current.min_samples()
            };
            if let Some(class) = key.strip_prefix("floor.") {
                floors.remove(class);
            } else if let Some(class) = key.strip_prefix("ceiling.") {
                ceilings.remove(class);
            } else if let Some(word) = key.strip_prefix("tier.") {
                let dc = DataClass::parse(word).ok_or_else(|| {
                    AuthorErr::Defect(format!("internal: unset key {key:?} lost its data class"))
                })?;
                tiers.remove(&dc);
            }
            (
                RoutePolicy::from_parts(floors, ceilings, tiers, min_samples),
                format!("unset {key}"),
            )
        }
        other => {
            return Err(AuthorErr::Defect(format!(
                "internal: policy prepare got {}",
                other.word()
            )))
        }
    };
    next.validate()
        .map_err(|e| AuthorErr::Defect(format!("ruling invalid — nothing written: {e:?}")))?;
    let candidate = encode_policy(&next);
    let reparsed = parse_policy(&candidate).map_err(|e| {
        AuthorErr::Defect(format!(
            "self-validation refused the candidate: {}",
            policy_msg(&e)
        ))
    })?;
    if reparsed != next {
        return Err(AuthorErr::Defect(
            "self-validation refused the candidate: re-parse is not the mutated policy".into(),
        ));
    }
    Ok((candidate, summary))
}

fn apply_set(p: &mut RoutePolicy, key: &str, value: &PolicyValue) -> Result<String, AuthorErr> {
    match value {
        PolicyValue::Floor(x) => {
            p.set_floor(key.strip_prefix("floor.").unwrap_or(key), *x);
            Ok(format!("{key}={x}"))
        }
        PolicyValue::Ceiling(x) => {
            p.set_ceiling(key.strip_prefix("ceiling.").unwrap_or(key), *x);
            Ok(format!("{key}={x}"))
        }
        PolicyValue::Tiers(ts) => {
            let word = key.strip_prefix("tier.").unwrap_or(key);
            let dc = DataClass::parse(word).ok_or_else(|| {
                AuthorErr::Defect(format!("internal: tier key {key:?} lost its data class"))
            })?;
            p.set_tiers(dc, ts.clone());
            let words: Vec<&str> = ts.iter().map(|t| t.as_str()).collect();
            Ok(format!("{key}=\"{}\"", words.join(",")))
        }
        PolicyValue::MinSamples(n) => {
            p.set_min_samples(*n);
            Ok(format!("min_samples={n}"))
        }
    }
}

fn registry_defect(e: crate::registry::RegistryErr) -> AuthorErr {
    let msg = match e {
        crate::registry::RegistryErr::Read(m) => m,
        crate::registry::RegistryErr::Malformed(m) => m,
    };
    AuthorErr::Defect(format!(
        "current lanes file is malformed — the organ never writes past a defect: {msg}"
    ))
}

fn policy_msg(e: &crate::policy_file::PolicyFileErr) -> String {
    match e {
        crate::policy_file::PolicyFileErr::Read(m) => m.clone(),
        crate::policy_file::PolicyFileErr::Malformed(m) => m.clone(),
    }
}

/// Step 7: the crash-ordered write. `.bak` first (immutable, named by
/// prior-content-hash), then tmp+rename, then the journal row. Returns the
/// journal seq.
pub fn author_commit(plan: &AuthorPlan, actor: &str, actor_kind: &str) -> Result<u64, AuthorErr> {
    // 1. The backup BEFORE anything else: if everything after dies, the old
    //    content is preserved under its content-hash name (an orphan .bak
    //    is detectable; the renamed file below is the only writer).
    if let (Some(bak), Some(prior)) = (&plan.bak, &plan.prior) {
        let bak_path = plan.target.with_file_name(bak);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&bak_path)
        {
            Ok(mut f) => {
                f.write_all(prior)
                    .map_err(|e| io_err("write bak", &bak_path, e))?;
                f.sync_data()
                    .map_err(|e| io_err("sync bak", &bak_path, e))?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                // Immutable: never overwrite. Identical bytes are
                // guaranteed by the content-hash name.
            }
            Err(e) => return Err(io_err("create bak", &bak_path, e)),
        }
    }

    // 2. tmp + rename (std rename = MOVEFILE_REPLACE_EXISTING on Windows:
    //    the brief's Windows law is the std library's own behavior).
    let tmp = plan.target.with_file_name(format!(
        "{}.tmp",
        plan.target
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("target")
    ));
    if let Some(parent) = plan.target.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| io_err("create target dir", parent, e))?;
        }
    }
    {
        let mut f = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp)
            .map_err(|e| io_err("create tmp", &tmp, e))?;
        f.write_all(plan.candidate.as_bytes())
            .map_err(|e| io_err("write tmp", &tmp, e))?;
        f.sync_data().map_err(|e| io_err("sync tmp", &tmp, e))?;
    }
    fs::rename(&tmp, &plan.target).map_err(|e| io_err("rename over target", &plan.target, e))?;

    // 3. The journal row LAST: a row without its write cannot exist.
    let row = JournalRow {
        line: 0,
        seq: 0,
        ts: now_iso(),
        actor: fit(actor),
        actor_kind: fit(actor_kind),
        op: plan.op.word().to_string(),
        target: plan
            .target
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string(),
        prior_hash: plan.prior_hash.clone(),
        next_hash: plan.next_hash.clone(),
        bak: plan.bak.clone(),
    };
    journal_append(&plan.journal, &row).map_err(|e| match e {
        AuthorErr::Defect(m) => AuthorErr::Defect(format!(
            "WRITE LANDED at {} but the journal append failed — the write is real, its audit row is missing: {m}",
            plan.target.display()
        )),
        other => other,
    })
}

fn io_err(what: &str, p: &Path, e: std::io::Error) -> AuthorErr {
    AuthorErr::Defect(format!("cannot {what} {}: {e}", p.display()))
}

// --- the authoring journal ---------------------------------------------------

/// One parsed `author.jsonl` row (`line` = 1-based file line; 0 for rows
/// built in memory before append).
#[derive(Debug, Clone, PartialEq)]
pub struct JournalRow {
    pub line: u64,
    pub seq: u64,
    pub ts: String,
    pub actor: String,
    pub actor_kind: String,
    pub op: String,
    pub target: String,
    pub prior_hash: Option<String>,
    pub next_hash: String,
    pub bak: Option<String>,
}

/// The whole journal, honestly: parsed rows AND the lines that would not
/// parse (the read side never hides a defect).
#[derive(Debug, Default, PartialEq)]
pub struct Journal {
    pub rows: Vec<JournalRow>,
    pub bad: Vec<(u64, String)>,
}

/// Read the journal (lock-free — an append is one atomic syscall-sized
/// write; readers see the pre- or post-line, never half).
pub fn journal_load(path: &Path) -> Journal {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(_) => return Journal::default(), // missing = no writes yet
    };
    let text = String::from_utf8_lossy(&bytes);
    let mut j = Journal::default();
    for (idx, line) in text.split('\n').enumerate() {
        let line_no = (idx + 1) as u64;
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        match journal_row_from_line(t) {
            Ok(mut r) => {
                r.line = line_no;
                j.rows.push(r);
            }
            Err(why) => j.bad.push((line_no, why)),
        }
    }
    j
}

fn journal_row_from_line(line: &str) -> Result<JournalRow, String> {
    let m = parse_object(line)?;
    let seq = as_u64(get(&m, "seq")?, "seq")?;
    let ts = as_str(get(&m, "ts")?, "ts")?.to_string();
    let actor = as_str(get(&m, "actor")?, "actor")?.to_string();
    let actor_kind = as_str(get(&m, "actor_kind")?, "actor_kind")?.to_string();
    let op = as_str(get(&m, "op")?, "op")?.to_string();
    let target = as_str(get(&m, "target")?, "target")?.to_string();
    let prior_hash = opt_str(&m, "prior_hash")?;
    let next_hash = as_str(get(&m, "next_hash")?, "next_hash")?.to_string();
    let bak = opt_str(&m, "bak")?;
    Ok(JournalRow {
        line: 0,
        seq,
        ts,
        actor,
        actor_kind,
        op,
        target,
        prior_hash,
        next_hash,
        bak,
    })
}

fn opt_str(m: &BTreeMap<String, Val>, k: &str) -> Result<Option<String>, String> {
    match m.get(k) {
        None | Some(Val::Null) => Ok(None),
        Some(Val::Str(s)) => Ok(Some(s.clone())),
        Some(_) => Err(format!("field '{k}' not string|null")),
    }
}

/// Append one row under the R6 law: O_EXCL lock → seq = max+1 over PARSED
/// rows (never the line count — a hand-forked file must not re-fork the
/// next row) → ONE `write_all` under ROW_CAP → `sync_data` before the lock
/// releases.
fn journal_append(path: &Path, row: &JournalRow) -> Result<u64, AuthorErr> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| io_err("create journal dir", parent, e))?;
        }
    }
    let _guard = Lock::acquire(path, LOCK_WAIT).map_err(|e| {
        AuthorErr::Defect(format!(
            "journal lock busy/broken at {}: {e:?}",
            path.display()
        ))
    })?;
    let loaded = journal_load(path);
    let seq = loaded.rows.iter().map(|r| r.seq).max().unwrap_or(0) + 1;
    let line = journal_line(seq, row);
    if line.len() > ROW_CAP {
        return Err(AuthorErr::Defect(format!(
            "journal row would exceed the single-write cap ({}) — shorten actor/op",
            line.len()
        )));
    }
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| io_err("open journal", path, e))?;
    f.write_all(line.as_bytes())
        .map_err(|e| io_err("append journal", path, e))?;
    f.sync_data().map_err(|e| io_err("sync journal", path, e))?;
    Ok(seq)
}

/// Flat-object encode, fixed field order, the crate's two-character
/// escaping discipline (free text through `esc`; hashes are hex-safe but
/// go through `esc` anyway — quoting is structural, never sniffed).
fn journal_line(seq: u64, r: &JournalRow) -> String {
    let prior = match &r.prior_hash {
        Some(h) => format!("\"{}\"", esc(h)),
        None => "null".to_string(),
    };
    let bak = match &r.bak {
        Some(b) => format!("\"{}\"", esc(b)),
        None => "null".to_string(),
    };
    format!(
        "{{\"seq\":{seq},\"ts\":\"{}\",\"actor\":\"{}\",\"actor_kind\":\"{}\",\"op\":\"{}\",\"target\":\"{}\",\"prior_hash\":{prior},\"next_hash\":\"{}\",\"bak\":{bak}}}\n",
        esc(&r.ts),
        esc(&r.actor),
        esc(&r.actor_kind),
        esc(&r.op),
        esc(&r.target),
        esc(&r.next_hash),
    )
}

#[cfg(test)]
#[path = "author_tests.rs"]
mod tests;
