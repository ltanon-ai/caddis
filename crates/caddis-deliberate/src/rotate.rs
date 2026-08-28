//! rotate.rs — ROTATION MACHINERY slice A: the ORCHESTRATION (brief §6).
//! One shot: lock → load → sweep → probe ReprobeDue seats in planned
//! waves → append result cards → streak bookkeeping → view re-sync →
//! report. The status map is the ruled table (brief §5.3 + Q6 amendment):
//!
//! | wire                     | class       | card                          |
//! |--------------------------|-------------|-------------------------------|
//! | 200                      | Live        | Live (since=now)              |
//! | 402                      | Expired     | Expired (quota calendar)      |
//! | 429                      | RateLimited | RateLimited (cooldown)        |
//! | 401/403 WITH auth        | Failed      | Failed (retry cadence)        |
//! | 401/403 WITHOUT auth     | Unprobeable | none; streak++; ×N ⇒ Unprobeable + ONE alert |
//! | blank base_url (no dial) | Unprobeable | same as above                 |
//! | 408/504/5xx/other/unlisted| Transient  | none (TTL backstop)           |
//! | network/TLS/timeout      | Transient   | none                          |
//!
//! Machine-written cards are TRANSPORT OBSERVATIONS (identity law): no
//! operator confirm per probe — the warden gates stay where F1/F2 put
//! them (convening + edits). Probing is not deliberation (council Q3,
//! 3:0). The `actor` is recorded in the run log (`deliberate-rotate`),
//! never inside card grammar (exact-field law).
//!
//! The core is PURE over an injected probe fn — tests fake it; the CLI
//! injects [`crate::prober::probe`]. Lock law (T-3): a held young lock is
//! a DEFECT; a lock older than [`LOCK_STALE_S`] is stolen (a wedged run
//! must not wedge rotation forever — worst-case run ≈ 120 s << 900 s).

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;

use crate::caps;
use crate::json::{self, Value};
use crate::prober::{self, ProbeCfg};
use crate::registry::{self, Card, Registry, SeatCard};
use crate::ttl::{self, Cadence};
use crate::LaneType;

/// A held young rotate lock is this many seconds old at most before it is
/// considered wedged (stolen). 900 s ≈ 7× the worst-case full rotation.
pub const LOCK_STALE_S: u64 = 900;

/// Q6 amendment default: this many CONSECUTIVE unprobeable rotations flip
/// the seat to `unprobeable` (census-visible) + ONE alert line.
pub const UNPROBEABLE_AFTER: u32 = 3;

/// Everything one rotation run is configured by (brief §6). Defaults =
/// compiled priors; the home's `rotation.json` overrides (validated,
/// exact-field; malformed = defect).
#[derive(Debug, Clone, PartialEq)]
pub struct RotateCfg {
    pub cadence: Cadence,
    pub probe: ProbeCfg,
    pub unprobeable_after: u32,
}

impl Default for RotateCfg {
    fn default() -> Self {
        RotateCfg {
            cadence: Cadence::default(),
            probe: ProbeCfg::default(),
            unprobeable_after: UNPROBEABLE_AFTER,
        }
    }
}

/// The ruled probe-result classification (pure).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeClass {
    Live,
    Expired,
    RateLimited,
    Failed,
    Unprobeable,
    Transient,
}

impl ProbeClass {
    pub fn word(self) -> &'static str {
        match self {
            ProbeClass::Live => "live",
            ProbeClass::Expired => "expired",
            ProbeClass::RateLimited => "rate-limited",
            ProbeClass::Failed => "failed",
            ProbeClass::Unprobeable => "unprobeable",
            ProbeClass::Transient => "transient",
        }
    }
}

/// The status map as ONE pure law: HTTP status (None = transport failed)
/// × whether auth is configured on the provider row → class.
pub fn map_status(status: Option<u16>, auth_configured: bool) -> ProbeClass {
    match status {
        Some(200) => ProbeClass::Live,
        Some(402) => ProbeClass::Expired,
        Some(429) => ProbeClass::RateLimited,
        Some(401) | Some(403) => {
            if auth_configured {
                ProbeClass::Failed
            } else {
                ProbeClass::Unprobeable
            }
        }
        // 408/504 explicitly transient (council Q2 amendment); every other
        // status is UNLISTED — honest transient-no-card, recorded in the
        // report. Never Live on a non-200 (money law).
        _ => ProbeClass::Transient,
    }
}

/// One rotation's honest report (the pure-JSON stdout surface).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RotateReport {
    pub now_epoch_s: u64,
    pub lock_stolen: bool,
    pub sweep_appended: usize,
    pub due: usize,
    pub skipped_non_http: Vec<String>,
    pub waves: usize,
    pub probed: usize,
    pub live: usize,
    pub expired: usize,
    pub rate_limited: usize,
    pub failed: usize,
    pub unprobeable: usize,
    pub transient: usize,
    /// (seat, reason) — bounded, credential-free.
    pub transient_reasons: Vec<(String, String)>,
    /// One line per seat per TRANSITION (Q6 amendment).
    pub alerts: Vec<String>,
    pub cards_appended: usize,
    pub view_synced: bool,
}

impl RotateReport {
    /// The machine surface (stdout law: pure JSON).
    pub fn to_json(&self) -> Value {
        let seats =
            |ids: &[String]| Value::Arr(ids.iter().map(|s| Value::Str(s.clone())).collect());
        Value::Obj(vec![
            ("verb".into(), Value::Str("rotate".into())),
            ("now_epoch_s".into(), Value::Num(self.now_epoch_s as f64)),
            ("lock_stolen".into(), Value::Bool(self.lock_stolen)),
            (
                "sweep_appended".into(),
                Value::Num(self.sweep_appended as f64),
            ),
            ("due".into(), Value::Num(self.due as f64)),
            ("skipped_non_http".into(), seats(&self.skipped_non_http)),
            ("waves".into(), Value::Num(self.waves as f64)),
            ("probed".into(), Value::Num(self.probed as f64)),
            ("live".into(), Value::Num(self.live as f64)),
            ("expired".into(), Value::Num(self.expired as f64)),
            ("rate_limited".into(), Value::Num(self.rate_limited as f64)),
            ("failed".into(), Value::Num(self.failed as f64)),
            ("unprobeable".into(), Value::Num(self.unprobeable as f64)),
            ("transient".into(), Value::Num(self.transient as f64)),
            (
                "transient_reasons".into(),
                Value::Arr(
                    self.transient_reasons
                        .iter()
                        .map(|(s, r)| {
                            Value::Arr(vec![Value::Str(s.clone()), Value::Str(r.clone())])
                        })
                        .collect(),
                ),
            ),
            (
                "alerts".into(),
                Value::Arr(self.alerts.iter().map(|a| Value::Str(a.clone())).collect()),
            ),
            (
                "cards_appended".into(),
                Value::Num(self.cards_appended as f64),
            ),
            ("view_synced".into(), Value::Bool(self.view_synced)),
        ])
    }
}

/// Rotation refusals. `Defect` = exit 2 (fail closed); a defect carries a
/// reason naming the broken law. `NothingDue` = exit 1 (honest quiet).
#[derive(Debug, Clone, PartialEq)]
pub enum RotateErr {
    /// Wiring/config/stream defect — nothing may proceed.
    Defect(String),
    /// Nothing was due and no sweep transitioned: rc 1.
    NothingDue(Box<RotateReport>),
}

impl fmt::Display for RotateErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RotateErr::Defect(m) => write!(f, "rotate defect: {m}"),
            RotateErr::NothingDue(_) => write!(f, "rotate: nothing due"),
        }
    }
}

impl std::error::Error for RotateErr {}

/// The injected transport: (base_url, auth_path, cfg) → outcome.
pub trait ProbeFn: Fn(&str, &str, &ProbeCfg) -> prober::ProbeOutcome + Sync {}
impl<T: Fn(&str, &str, &ProbeCfg) -> prober::ProbeOutcome + Sync> ProbeFn for T {}

/// Home file names.
pub fn stream_path(home: &Path) -> PathBuf {
    home.join("seats.jsonl")
}
pub fn view_path(home: &Path) -> PathBuf {
    home.join("seats-view.json")
}
fn lock_path(home: &Path) -> PathBuf {
    home.join("rotate.lock")
}
pub fn state_path(home: &Path) -> PathBuf {
    home.join("rotation-state.json")
}
pub fn config_path(home: &Path) -> PathBuf {
    home.join("rotation.json")
}
pub fn log_path(home: &Path) -> PathBuf {
    home.join("rotation.log")
}

// ---------------------------------------------------------------------------
// Lock (T-3 law)
// ---------------------------------------------------------------------------

/// RAII rotate lock: young held lock = defect; stale = stolen. Released
/// (file removed) on drop — including error paths.
struct RotateLock {
    path: PathBuf,
    active: bool,
}

impl RotateLock {
    fn acquire(home: &Path, now_epoch_s: u64) -> Result<(RotateLock, bool), String> {
        let path = lock_path(home);
        let body = format!(
            "{{\"pid\":{},\"started_epoch_s\":{}}}\n",
            std::process::id(),
            now_epoch_s
        );
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create_new(true);
        match opts.open(&path) {
            Ok(mut f) => {
                use std::io::Write;
                let _ = f.write_all(body.as_bytes());
                Ok((RotateLock { path, active: true }, false))
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                // Held: young = defect, stale = steal. Age comes from the
                // body when it parses; an unreadable body falls back to the
                // FILE MTIME (a crash mid-write must not wedge rotation,
                // and a corrupt YOUNG lock must not be stolen).
                let body_started = fs::read_to_string(&path)
                    .ok()
                    .and_then(|t| json::parse(&t).ok())
                    .and_then(|v| v.get("started_epoch_s").and_then(|n| n.as_f64()))
                    .map(|n| n as u64);
                let started = match body_started {
                    Some(s) => s,
                    None => fs::metadata(&path)
                        .and_then(|m| m.modified())
                        .map_err(|e| format!("lock {}: {e}", path.display()))?
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0),
                };
                let stale = now_epoch_s.saturating_sub(started.max(1)) >= LOCK_STALE_S;
                if !stale {
                    return Err(format!(
                        "rotate lock held ({}), younger than {LOCK_STALE_S}s",
                        path.display()
                    ));
                }
                let _ = fs::remove_file(&path);
                let mut opts = fs::OpenOptions::new();
                opts.write(true).create_new(true);
                match opts.open(&path) {
                    Ok(mut f) => {
                        use std::io::Write;
                        let _ = f.write_all(body.as_bytes());
                        Ok((RotateLock { path, active: true }, true))
                    }
                    Err(e2) => Err(format!("steal {} failed: {e2}", path.display())),
                }
            }
            Err(e) => Err(format!("open {}: {e}", path.display())),
        }
    }
}

impl Drop for RotateLock {
    fn drop(&mut self) {
        if self.active {
            let _ = fs::remove_file(&self.path);
        }
    }
}

// ---------------------------------------------------------------------------
// rotation-state.json (streak bookkeeping; fail-closed on malformed)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RotationState {
    /// seat id → consecutive unprobeable rotations.
    pub streaks: BTreeMap<String, u32>,
}

impl RotationState {
    fn load(home: &Path) -> Result<RotationState, String> {
        let p = state_path(home);
        if !p.exists() {
            return Ok(RotationState::default());
        }
        let text = fs::read_to_string(&p).map_err(|e| format!("read {}: {e}", p.display()))?;
        let v = json::parse(&text).map_err(|e| format!("parse {}: {e:?}", p.display()))?;
        let seats = v
            .get("seats")
            .ok_or_else(|| format!("{}: missing seats", p.display()))?;
        let obj = seats
            .as_obj()
            .ok_or_else(|| format!("{}: seats not an object", p.display()))?;
        let mut streaks = BTreeMap::new();
        for (id, sv) in obj {
            let st = sv
                .get("unprobeable_streak")
                .and_then(|n| n.as_f64())
                .ok_or_else(|| format!("{}: seat {id} missing unprobeable_streak", p.display()))?;
            if st < 0.0 || st.fract() != 0.0 || st > u32::MAX as f64 {
                return Err(format!("{}: seat {id} bad streak {st}", p.display()));
            }
            streaks.insert(id.clone(), st as u32);
        }
        Ok(RotationState { streaks })
    }

    fn save(&self, home: &Path, now_epoch_s: u64) -> Result<(), String> {
        let p = state_path(home);
        let mut seats = String::from("{");
        for (i, (id, st)) in self.streaks.iter().enumerate() {
            if i > 0 {
                seats.push(',');
            }
            seats.push_str(&json::to_string(&Value::Str(id.clone())));
            seats.push_str(&format!(":{{\"unprobeable_streak\":{st}}}"));
        }
        seats.push('}');
        let body = format!("{{\"updated_epoch_s\":{now_epoch_s},\"seats\":{seats}}}\n");
        atomic_write(&p, &body)
    }
}

/// Atomic file replace: sibling temp + rename (the view-write law; the
/// stream is append-only and never passes through here).
fn atomic_write(path: &Path, contents: &str) -> Result<(), String> {
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, contents).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    fs::rename(&tmp, path)
        .map_err(|e| format!("rename {} -> {}: {e}", tmp.display(), path.display()))
}

// ---------------------------------------------------------------------------
// rotation.json (cadence/probe overrides; exact-field law)
// ---------------------------------------------------------------------------

fn u64_of(v: &Value, key: &str) -> Result<u64, String> {
    v.get(key)
        .and_then(|n| n.as_f64())
        .ok_or_else(|| format!("rotation.json: {key} missing/not a number"))
        .and_then(|n| {
            if n >= 0.0 && n.fract() == 0.0 {
                Ok(n as u64)
            } else {
                Err(format!("rotation.json: {key} not a non-negative integer"))
            }
        })
}

fn fields_of(v: &Value) -> Vec<String> {
    v.as_obj()
        .map(|o| o.iter().map(|(k, _)| k.clone()).collect())
        .unwrap_or_default()
}

/// Load the home's rotation.json over the defaults. Absent = defaults;
/// present but malformed / unknown field = Err (exact-field law).
pub fn load_cfg(home: &Path) -> Result<RotateCfg, String> {
    let p = config_path(home);
    if !p.exists() {
        return Ok(RotateCfg::default());
    }
    let text = fs::read_to_string(&p).map_err(|e| format!("read {}: {e}", p.display()))?;
    let v = json::parse(&text).map_err(|e| format!("parse {}: {e:?}", p.display()))?;
    if !matches!(v, Value::Obj(_)) {
        return Err(format!("{}: not an object", p.display()));
    }
    let known = ["cadence", "probe", "unprobeable_after"];
    for k in fields_of(&v) {
        if !known.contains(&k.as_str()) {
            return Err(format!("{}: unknown field {k}", p.display()));
        }
    }
    let mut cfg = RotateCfg::default();
    if let Some(c) = v.get("cadence") {
        let fields = [
            "live_probe_every_s",
            "expired_ttl_s",
            "rate_limited_cooldown_s",
            "probing_timeout_s",
            "failed_retry_every_s",
            "unprobeable_retry_every_s",
        ];
        for k in fields_of(c) {
            if !fields.contains(&k.as_str()) {
                return Err(format!("{}: cadence unknown field {k}", p.display()));
            }
        }
        let mut cad = cfg.cadence;
        if c.get("live_probe_every_s").is_some() {
            cad.live_probe_every_s = u64_of(c, "live_probe_every_s")?;
        }
        if c.get("expired_ttl_s").is_some() {
            cad.expired_ttl_s = u64_of(c, "expired_ttl_s")?;
        }
        if c.get("rate_limited_cooldown_s").is_some() {
            cad.rate_limited_cooldown_s = u64_of(c, "rate_limited_cooldown_s")?;
        }
        if c.get("probing_timeout_s").is_some() {
            cad.probing_timeout_s = u64_of(c, "probing_timeout_s")?;
        }
        if c.get("failed_retry_every_s").is_some() {
            cad.failed_retry_every_s = u64_of(c, "failed_retry_every_s")?;
        }
        if c.get("unprobeable_retry_every_s").is_some() {
            cad.unprobeable_retry_every_s = u64_of(c, "unprobeable_retry_every_s")?;
        }
        cfg.cadence = cad;
    }
    if let Some(pr) = v.get("probe") {
        for k in fields_of(pr) {
            if k != "connect_timeout_s" && k != "total_timeout_s" {
                return Err(format!("{}: probe unknown field {k}", p.display()));
            }
        }
        let mut pc = cfg.probe;
        if pr.get("connect_timeout_s").is_some() {
            pc.connect_timeout = std::time::Duration::from_secs(u64_of(pr, "connect_timeout_s")?);
        }
        if pr.get("total_timeout_s").is_some() {
            pc.total_timeout = std::time::Duration::from_secs(u64_of(pr, "total_timeout_s")?);
        }
        cfg.probe = pc;
    }
    if v.get("unprobeable_after").is_some() {
        let n = v
            .get("unprobeable_after")
            .and_then(|x| x.as_f64())
            .ok_or("rotation.json: unprobeable_after not a number")?;
        if n < 1.0 || n.fract() != 0.0 || n > 100.0 {
            return Err("rotation.json: unprobeable_after out of range".into());
        }
        cfg.unprobeable_after = n as u32;
    }
    Ok(cfg)
}

// ---------------------------------------------------------------------------
// The rotation core
// ---------------------------------------------------------------------------

/// Run ONE rotation. `probe_fn` is the transport seam (tests fake it).
/// On Ok: the report of a RUN (rc 0). On [`RotateErr::NothingDue`]: the
/// report of a quiet pass (rc 1). On [`RotateErr::Defect`]: rc 2, and any
/// already-appended cards stay (the stream is truth; append-only law).
pub fn rotate<F>(
    home: &Path,
    now_epoch_s: u64,
    cfg: &RotateCfg,
    probe_fn: F,
) -> Result<RotateReport, RotateErr>
where
    F: ProbeFn,
{
    let (lock, stolen) = RotateLock::acquire(home, now_epoch_s).map_err(RotateErr::Defect)?;
    let mut report = RotateReport {
        now_epoch_s,
        lock_stolen: stolen,
        ..RotateReport::default()
    };
    let stream = stream_path(home);
    let view = view_path(home);
    if !stream.exists() {
        return Err(RotateErr::Defect(format!(
            "no stream at {} — seed the home first",
            stream.display()
        )));
    }
    let (mut reg, _) = registry::load_and_sync(&stream, &view)
        .map_err(|e| RotateErr::Defect(format!("load: {e}")))?;
    caps::validate_registry(&reg).map_err(|e| RotateErr::Defect(format!("caps law: {e}")))?;

    let sweep_cards = ttl::sweep(&reg, now_epoch_s, &cfg.cadence, ttl::quota_renewable);
    report.sweep_appended = sweep_cards.len();
    for card in sweep_cards {
        reg = append(&stream, &view, card, &mut report).map_err(RotateErr::Defect)?;
    }

    // 2. Due = step() says ReprobeDue and the lane is http (v1 transport).
    let mut due: Vec<SeatCard> = Vec::new();
    for seat in reg.seats.values() {
        if ttl::step(seat, now_epoch_s, &cfg.cadence, ttl::quota_renewable(seat))
            == ttl::Step::ReprobeDue
        {
            if seat.lane_type == LaneType::Http {
                due.push(seat.clone());
            } else {
                report.skipped_non_http.push(seat.id.clone());
            }
        }
    }
    due.sort_by(|a, b| a.id.cmp(&b.id)); // deterministic dispatch order
    report.due = due.len();
    if due.is_empty() && report.sweep_appended == 0 {
        return Err(RotateErr::NothingDue(Box::new(report)));
    }

    let wanted: Vec<&str> = due.iter().map(|s| s.id.as_str()).collect();
    let waves =
        caps::plan_batches(&wanted, &reg).map_err(|e| RotateErr::Defect(format!("plan: {e}")))?;
    report.waves = waves.len();
    let mut state = RotationState::load(home).map_err(RotateErr::Defect)?;
    let mut dirty_state = false;

    // 4. Wave legs concurrent, waves joined (executor family law).
    // plan_batches already refused unknown seats and missing providers,
    // so job assembly cannot fail — only drift mid-run would, and that is
    // a Defect below.
    let mut wave_jobs: Vec<Vec<(SeatCard, crate::registry::ProviderCard)>> = Vec::new();
    for wave in &waves {
        let mut jobs = Vec::new();
        for id in wave {
            let seat = &reg.seats[id];
            let provider = reg.providers.get(&seat.provider).ok_or_else(|| {
                RotateErr::Defect(format!(
                    "seat {} has no provider row {} (planner validated — drift?)",
                    seat.id, seat.provider
                ))
            })?;
            jobs.push((seat.clone(), provider.clone()));
        }
        wave_jobs.push(jobs);
    }
    let mut all_outcomes: Vec<(String, prober::ProbeOutcome)> = Vec::new();
    for wave in &wave_jobs {
        let mut wave_results: Vec<(String, prober::ProbeOutcome)> = Vec::new();
        thread::scope(|s| {
            let probe_fn = &probe_fn;
            let mut handles = Vec::new();
            for (seat, provider) in wave {
                let base_url = provider.base_url.clone();
                let auth_path = provider.auth_path.clone();
                let pcfg = cfg.probe;
                handles.push(s.spawn(move || {
                    if base_url.trim().is_empty() {
                        // Undialable-by-URL: the honest UNPROBEABLE class,
                        // never dialed (facts §1.5).
                        (
                            seat.id.clone(),
                            prober::ProbeOutcome {
                                status: None,
                                error: Some("blank base_url (undialable)".into()),
                            },
                        )
                    } else {
                        (seat.id.clone(), probe_fn(&base_url, &auth_path, &pcfg))
                    }
                }));
            }
            for h in handles {
                if let Ok(pair) = h.join() {
                    wave_results.push(pair);
                }
            }
        });
        wave_results.sort_by(|a, b| a.0.cmp(&b.0)); // deterministic fold-in
        all_outcomes.extend(wave_results);
    }

    // 5. Classify + land cards + streaks.
    for (seat_id, outcome) in all_outcomes {
        report.probed += 1;
        let seat = &reg.seats[&seat_id];
        let provider = reg
            .providers
            .get(&seat.provider)
            .ok_or_else(|| RotateErr::Defect(format!("provider {} vanished", seat.provider)))?;
        let auth_configured = !provider.auth_path.trim().is_empty();
        let undialable = outcome
            .error
            .as_deref()
            .map(|e| e.contains("blank base_url"))
            .unwrap_or(false);
        let class = if undialable {
            ProbeClass::Unprobeable
        } else {
            map_status(outcome.status, auth_configured)
        };
        match class {
            ProbeClass::Transient => {
                report.transient += 1;
                if report.transient_reasons.len() < 32 {
                    let reason = outcome
                        .error
                        .clone()
                        .unwrap_or_else(|| format!("unlisted status {:?}", outcome.status));
                    report.transient_reasons.push((seat_id.clone(), reason));
                }
                // Learned nothing: the streak chain is NOT consecutive
                // anymore — reset (documented reading of the amendment).
                if state.streaks.remove(&seat_id).is_some() {
                    dirty_state = true;
                }
            }
            ProbeClass::Unprobeable => {
                report.unprobeable += 1;
                let streak = state.streaks.entry(seat_id.clone()).or_insert(0);
                *streak += 1;
                dirty_state = true;
                if *streak >= cfg.unprobeable_after && seat.state != crate::SeatState::Unprobeable {
                    let mut card = seat.clone();
                    card.state = crate::SeatState::Unprobeable;
                    card.since_epoch_s = now_epoch_s;
                    reg = append(&stream, &view, Card::Seat(card), &mut report)
                        .map_err(RotateErr::Defect)?;
                    report.alerts.push(format!(
                        "seat {seat_id} unprobeable after {} consecutive rotations (no configured auth / undialable) — auth landing via the edits path lifts it",
                        *streak
                    ));
                }
            }
            mapped => {
                match mapped {
                    ProbeClass::Live => report.live += 1,
                    ProbeClass::Expired => report.expired += 1,
                    ProbeClass::RateLimited => report.rate_limited += 1,
                    ProbeClass::Failed => report.failed += 1,
                    ProbeClass::Unprobeable | ProbeClass::Transient => unreachable!(),
                }
                if state.streaks.remove(&seat_id).is_some() {
                    dirty_state = true;
                }
                // Mapped results ALWAYS stamp freshness (even same-state —
                // a fresh observation must restart the cadence clock).
                let mut card = seat.clone();
                card.state = match mapped {
                    ProbeClass::Live => crate::SeatState::Live,
                    ProbeClass::Expired => crate::SeatState::Expired,
                    ProbeClass::RateLimited => crate::SeatState::RateLimited,
                    ProbeClass::Failed => crate::SeatState::Failed,
                    ProbeClass::Unprobeable | ProbeClass::Transient => unreachable!(),
                };
                card.since_epoch_s = now_epoch_s;
                reg = append(&stream, &view, Card::Seat(card), &mut report)
                    .map_err(RotateErr::Defect)?;
            }
        }
    }

    // 6. Persist bookkeeping + view + log; release via Drop.
    if dirty_state {
        state.save(home, now_epoch_s).map_err(RotateErr::Defect)?;
    }
    let (_, synced) = registry::load_and_sync(&stream, &view)
        .map_err(|e| RotateErr::Defect(format!("final view sync: {e}")))?;
    report.view_synced = true;
    let _ = synced;
    append_log_line(home, &report);
    drop(lock);
    Ok(report)
}

fn append(
    stream: &Path,
    view: &Path,
    card: Card,
    report: &mut RotateReport,
) -> Result<Registry, String> {
    let (reg, _) =
        registry::append_card(stream, view, &card).map_err(|e| format!("append: {e}"))?;
    report.cards_appended += 1;
    Ok(reg)
}

/// One machine-parsable summary line per run (actor + honest counts).
fn append_log_line(home: &Path, r: &RotateReport) {
    let mut line = json::to_string(&r.to_json());
    line.push('\n');
    if let Ok(mut f) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path(home))
    {
        use std::io::Write;
        let _ = f.write_all(line.as_bytes());
    }
}

#[cfg(test)]
#[path = "rotate_tests.rs"]
mod tests;
