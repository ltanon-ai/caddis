//! P4 slice 1 — the ALERT organ + the R2/R4 transition scan.
//!
//! Two laws land here:
//! - **R2 (transient -> persistent promotion):** hysteresis decay that
//!   PERSISTS must become a LEDGER FACT, not just fold state. The scan
//!   replays outcome rows in seq order, derives the demote/heal transition
//!   history per (lane, class), and appends `promotion` marker rows for every
//!   transition not yet recorded — idempotent by PREFIX: the recorded markers
//!   of a healthy pair are exactly a prefix of the derived history, so a
//!   re-scan appends nothing. Markers that are NOT a prefix (hand edit,
//!   forked file) are counted as `marker_mismatch` and the pair is LEFT
//!   ALONE — the scan is a writer, never a silent repairman (model-voice
//!   law; `verify` is the tool that reports ledger damage).
//! - **R4 (degraded alert):** the operator must KNOW, not discover. Every
//!   appended promotion also lands one row in the append-only alert stream
//!   `<home>/alerts.jsonl` — same append law as the ledger (O_EXCL lock,
//!   seq = max+1 over PARSED rows, one `write_all`, `sync_data` before the
//!   lock releases, fail closed on a busy lock).
//!
//! Escalation stops (P3's fail-safe halts) become alerts through
//! [`Alert::from_escalation_stop`] — the LIBRARY surface the P4 dispatch
//! adapters call when `escalate()` refuses a climb. This crate never
//! dispatches (F1), so nothing here emits stop alerts on its own.
//!
//! Purity split (F1): [`transitions`] and [`plan_scan`] are pure functions
//! of `Loaded`; only [`run_scan`] touches files, through [`Ledger`] and
//! [`Alerts`].

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::escalation::{EscalationCtx, EscalationErr};
use crate::lane::DataClass;
use crate::ledger::{
    as_str, as_u64, esc, fit, get, now_iso, parse_object, Ledger, LedgerErr, Loaded, Outcome,
    ParsedRow, PromotionRow, Row, LOCK_WAIT,
};
use crate::lock::{Lock, LockErr};
use crate::route::RouteErr;
use crate::stats::HYSTERESIS_FAILS;

/// Same single-`write_all` atomicity budget as the ledger row cap.
const ALERT_ROW_CAP: usize = 4096;

// ---------------------------------------------------------------------------
// Alert record + stream
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertKind {
    /// R2/R4: trailing RED-TEST fails reached hysteresis — lane demoted
    /// persistent. Operator must know.
    Degraded,
    /// R2/QQ2: a pass cleared persistent decay — lane healed.
    Healed,
    /// P3 fail-safe halt from [`crate::escalation`] — emitted by the P4
    /// dispatch adapters, never by the scan.
    EscalationStop,
    /// P1/P4: [`crate::route::route`] refused (fail closed) — emitted by
    /// the dispatch adapters on every routing halt, never by the crate.
    RouteStop,
}

impl AlertKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            AlertKind::Degraded => "degraded",
            AlertKind::Healed => "healed",
            AlertKind::EscalationStop => "escalation_stop",
            AlertKind::RouteStop => "route_stop",
        }
    }
    fn parse(s: &str) -> Option<Self> {
        match s {
            "degraded" => Some(AlertKind::Degraded),
            "healed" => Some(AlertKind::Healed),
            "escalation_stop" => Some(AlertKind::EscalationStop),
            "route_stop" => Some(AlertKind::RouteStop),
            _ => None,
        }
    }
}

/// One operator alert. Free text lives in `detail` only; `lane_id`/`class`
/// are routing coordinates so feeds can group without parsing prose.
#[derive(Debug, Clone, PartialEq)]
pub struct Alert {
    pub kind: AlertKind,
    pub lane_id: String,
    pub class: String,
    pub detail: String,
}

impl Alert {
    /// The P3 stop surface as an alert — the derivation the dispatch
    /// adapters (P4) persist when `escalate()` fails safe. Every variant
    /// carries its own honest reason; none is guessed.
    pub fn from_escalation_stop(ctx: &EscalationCtx, err: &EscalationErr) -> Self {
        let detail: &'static str = match err {
            EscalationErr::MaxHops => "chain spent MAX_HOPS decisions — fail-safe halt",
            EscalationErr::NoFloorForClass => "class has no quality floor (routing defect) — halt",
            EscalationErr::NoCeilingForClass => {
                "no operator budget ceiling ruled — escalation closed (R1)"
            }
            EscalationErr::UnknownFailedLane => {
                "failed lane has no capability row for the class (caller defect)"
            }
            EscalationErr::TopOfLadder => {
                "no measured lane strictly above the failed rung — fail-safe halt"
            }
            EscalationErr::OverCeiling => {
                "every climb would exceed the class ceiling — fail-safe halt"
            }
        };
        Alert {
            kind: AlertKind::EscalationStop,
            lane_id: ctx.failed_lane_id.clone(),
            class: ctx.task_class.clone(),
            detail: detail.to_string(),
        }
    }

    /// P4/F5: the route() halt surface as an alert — the derivation the
    /// dispatch adapters persist when [`crate::route::route`] refuses. No
    /// lane was chosen, so `lane_id` is empty; the data class lives in the
    /// detail because it names WHICH filter closed. Every variant carries
    /// its own honest reason; none is guessed.
    pub fn from_route_stop(data_class: DataClass, task_class: &str, err: &RouteErr) -> Self {
        let detail = match err {
            RouteErr::NoAliveLane => "no lane alive — routing refused (fail closed)".to_string(),
            RouteErr::NoPermittedLane => format!(
                "F5: no permitted lane for data class {} — fail closed",
                data_class.as_str()
            ),
            RouteErr::NoMeasuredLane => {
                "F2: no permitted lane measured for the class (cold start) — refuse to guess"
                    .to_string()
            }
            RouteErr::NoFloorForClass => {
                "F6: class has no quality floor — thresholds are never guessed".to_string()
            }
            RouteErr::BelowFloorFailClosed => format!(
                "R4: every measured lane below floor on {} — fail closed",
                data_class.as_str()
            ),
        };
        Alert {
            kind: AlertKind::RouteStop,
            lane_id: String::new(),
            class: task_class.to_string(),
            detail,
        }
    }
}

#[derive(Debug)]
pub enum AlertErr {
    Io(std::io::Error),
    /// Same R6 posture as the ledger: concurrent appends are forbidden by
    /// construction — refuse, retry.
    LockBusy,
}

// io::Error is not PartialEq: compare by KIND (same law as ledger/lock).
impl PartialEq for AlertErr {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (AlertErr::Io(a), AlertErr::Io(b)) => a.kind() == b.kind(),
            (AlertErr::LockBusy, AlertErr::LockBusy) => true,
            _ => false,
        }
    }
}
impl From<std::io::Error> for AlertErr {
    fn from(e: std::io::Error) -> Self {
        AlertErr::Io(e)
    }
}
impl From<LockErr> for AlertErr {
    fn from(e: LockErr) -> Self {
        match e {
            LockErr::Busy => AlertErr::LockBusy,
            LockErr::Io(e) => AlertErr::Io(e),
        }
    }
}
impl std::fmt::Display for AlertErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AlertErr::Io(e) => write!(f, "alerts io: {e}"),
            AlertErr::LockBusy => write!(f, "alerts lock busy (R6 fail-closed)"),
        }
    }
}

/// One parsed alert row with its file coordinates.
#[derive(Debug, Clone, PartialEq)]
pub struct AlertRow {
    pub seq: u64,
    pub ts: String,
    pub alert: Alert,
}

/// The alert stream, honestly: parsed rows + unparseable lines kept.
#[derive(Debug, Default, PartialEq)]
pub struct LoadedAlerts {
    pub rows: Vec<AlertRow>,
    pub bad: Vec<(u64, String)>,
}

impl LoadedAlerts {
    pub fn max_seq(&self) -> u64 {
        self.rows.iter().map(|r| r.seq).max().unwrap_or(0)
    }
}

/// The alert stream writer — the ledger's append law, verbatim (R6).
pub struct Alerts {
    path: PathBuf,
}

impl Alerts {
    pub fn new<P: AsRef<Path>>(path: P) -> Self {
        Alerts {
            path: path.as_ref().to_path_buf(),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Append one alert (clock: real time). See [`Alerts::append_ts`] for
    /// the deterministic entry point tests use.
    pub fn append(&self, a: &Alert) -> Result<u64, AlertErr> {
        self.append_ts(a, &now_iso(), LOCK_WAIT)
    }

    pub(crate) fn append_ts(&self, a: &Alert, ts: &str, wait: Duration) -> Result<u64, AlertErr> {
        // A missing parent dir is birth, not corruption (os error 3 lesson).
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let _guard = Lock::acquire(&self.path, wait)?;
        let loaded = self.load_unlocked()?;
        let seq = loaded.max_seq() + 1;
        let line = encode_alert(seq, ts, a);
        debug_assert!(
            line.len() <= ALERT_ROW_CAP,
            "alert exceeds ALERT_ROW_CAP by construction"
        );
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        f.write_all(line.as_bytes())?;
        f.sync_data()?;
        Ok(seq)
    }

    /// Lock-free read (an append is one atomic syscall-sized write).
    pub fn load(&self) -> Result<LoadedAlerts, AlertErr> {
        self.load_unlocked()
    }

    fn load_unlocked(&self) -> Result<LoadedAlerts, AlertErr> {
        let bytes = match fs::read(&self.path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(LoadedAlerts::default());
            }
            Err(e) => return Err(AlertErr::Io(e)),
        };
        let text = String::from_utf8_lossy(&bytes);
        Ok(parse_alert_stream(&text))
    }
}

/// Encode one alert row. Every string is `fit`-bounded, so the line cannot
/// exceed ALERT_ROW_CAP by construction — no failure mode, no `Result`.
fn encode_alert(seq: u64, ts: &str, a: &Alert) -> String {
    let fields: Vec<String> = vec![
        format!("\"seq\":{seq}"),
        format!("\"ts\":\"{}\"", esc(&fit(ts))),
        format!("\"kind\":\"{}\"", esc(a.kind.as_str())),
        format!("\"lane_id\":\"{}\"", esc(&fit(&a.lane_id))),
        format!("\"class\":\"{}\"", esc(&fit(&a.class))),
        format!("\"detail\":\"{}\"", esc(&fit(&a.detail))),
    ];
    format!("{{{}}}\n", fields.join(","))
}

/// Parse a whole alert stream into rows + honest bad-line list (the ledger's
/// `parse_stream` discipline over the alert shape).
pub fn parse_alert_stream(text: &str) -> LoadedAlerts {
    let mut loaded = LoadedAlerts::default();
    for (idx, line) in text.split('\n').enumerate() {
        let line_no = (idx + 1) as u64;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match parse_alert_line(trimmed) {
            Ok(row) => loaded.rows.push(AlertRow {
                seq: row.0,
                ts: row.1,
                alert: row.2,
            }),
            Err(why) => loaded.bad.push((line_no, why)),
        }
    }
    loaded
}

fn parse_alert_line(line: &str) -> Result<(u64, String, Alert), String> {
    let m = parse_object(line)?;
    let seq = as_u64(get(&m, "seq")?, "seq")?;
    if seq == 0 {
        return Err("seq must start at 1".into());
    }
    let ts = as_str(get(&m, "ts")?, "ts")?.to_string();
    let kind = AlertKind::parse(as_str(get(&m, "kind")?, "kind")?)
        .ok_or("kind must be degraded|healed|escalation_stop")?;
    Ok((
        seq,
        ts,
        Alert {
            kind,
            lane_id: as_str(get(&m, "lane_id")?, "lane_id")?.to_string(),
            class: as_str(get(&m, "class")?, "class")?.to_string(),
            detail: as_str(get(&m, "detail")?, "detail")?.to_string(),
        },
    ))
}

// ---------------------------------------------------------------------------
// R2 transition derivation (pure)
// ---------------------------------------------------------------------------

/// One derived demote/heal transition. `after_seq` is the outcome row that
/// CAUSED it — the promotion marker's natural anchor in the stream.
#[derive(Debug, Clone, PartialEq)]
pub struct Transition {
    pub lane_id: String,
    pub class: String,
    pub demoted: bool,
    pub after_seq: u64,
    pub trailing_fails: u32,
}

/// Replay OUTCOME rows in seq order and derive the full demote/heal history
/// per (lane, class). Promotion markers are deliberately ignored here:
/// derivation is a pure function of outcome evidence (QQ1a spirit — a
/// marker is a CLAIM about decay, not decay itself). The first transition of
/// any pair is always a demotion; the sequence alternates.
pub fn transitions(loaded: &Loaded) -> Vec<Transition> {
    #[derive(Default)]
    struct St {
        trailing: u32,
        demoted: bool,
    }
    let mut sorted: Vec<&ParsedRow> = loaded.rows.iter().collect();
    sorted.sort_by_key(|p| p.seq);
    let mut st: BTreeMap<(String, String), St> = BTreeMap::new();
    let mut out = Vec::new();
    for p in sorted {
        let Row::Outcome(o) = &p.row else {
            continue;
        };
        let key = (o.lane_id.clone(), o.task_class.clone());
        let e = st.entry(key.clone()).or_default();
        match o.outcome {
            Outcome::Fail => {
                e.trailing += 1;
                if e.trailing >= HYSTERESIS_FAILS && !e.demoted {
                    e.demoted = true;
                    out.push(Transition {
                        lane_id: key.0,
                        class: key.1,
                        demoted: true,
                        after_seq: p.seq,
                        trailing_fails: e.trailing,
                    });
                }
            }
            Outcome::Pass => {
                if e.demoted {
                    out.push(Transition {
                        lane_id: key.0,
                        class: key.1,
                        demoted: false,
                        after_seq: p.seq,
                        trailing_fails: 0,
                    });
                }
                e.trailing = 0;
                e.demoted = false;
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// The scan (pure plan + file executor)
// ---------------------------------------------------------------------------

/// What the scan would do — pure, testable, never touches a file.
#[derive(Debug, PartialEq)]
pub struct ScanPlan {
    pub report: ScanReport,
    /// Transitions not yet recorded, in derivation order.
    pub append: Vec<Transition>,
}

#[derive(Debug, Default, PartialEq)]
pub struct ScanReport {
    /// Outcome rows scanned (evidence base).
    pub outcomes_scanned: u64,
    /// Full derived transition history length.
    pub transitions_total: u32,
    /// Promotion markers already in the ledger.
    pub promotions_recorded: u32,
    /// Promotions this scan appends (in a dry run: would append).
    pub promotions_appended: u32,
    /// Alerts this scan appends (one per promotion; in a dry run: would).
    pub alerts_appended: u32,
    /// (lane, class) pairs whose recorded markers are NOT a prefix of the
    /// derived history — left untouched, reported honestly.
    pub marker_mismatch: u32,
    pub dry_run: bool,
}

#[derive(Debug)]
pub enum ScanErr {
    Ledger(LedgerErr),
    Alerts(AlertErr),
}

impl PartialEq for ScanErr {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (ScanErr::Ledger(a), ScanErr::Ledger(b)) => a == b,
            (ScanErr::Alerts(a), ScanErr::Alerts(b)) => a == b,
            _ => false,
        }
    }
}
impl From<LedgerErr> for ScanErr {
    fn from(e: LedgerErr) -> Self {
        ScanErr::Ledger(e)
    }
}
impl From<AlertErr> for ScanErr {
    fn from(e: AlertErr) -> Self {
        ScanErr::Alerts(e)
    }
}
impl std::fmt::Display for ScanErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScanErr::Ledger(e) => write!(f, "scan ledger: {e}"),
            ScanErr::Alerts(e) => write!(f, "scan alerts: {e}"),
        }
    }
}

/// Compare derived history against recorded markers; plan the suffix to
/// append. Prefix law: healthy recorded markers are exactly the first k
/// derived transitions (demoted flags equal); anything else is a mismatch
/// counted, never repaired.
pub fn plan_scan(loaded: &Loaded) -> ScanPlan {
    let derived = transitions(loaded);
    let mut sorted: Vec<&ParsedRow> = loaded.rows.iter().collect();
    sorted.sort_by_key(|p| p.seq);
    let mut recorded: BTreeMap<(String, String), Vec<bool>> = BTreeMap::new();
    let (mut outcomes_scanned, mut promotions_recorded) = (0u64, 0u32);
    for p in &sorted {
        match &p.row {
            Row::Outcome(_) => outcomes_scanned += 1,
            Row::Promotion(pr) => {
                recorded
                    .entry((pr.lane_id.clone(), pr.task_class.clone()))
                    .or_default()
                    .push(pr.demoted);
                promotions_recorded += 1;
            }
            Row::Decision(_) => {}
        }
    }
    let mut derived_groups: BTreeMap<(String, String), Vec<&Transition>> = BTreeMap::new();
    for t in &derived {
        derived_groups
            .entry((t.lane_id.clone(), t.class.clone()))
            .or_default()
            .push(t);
    }
    let keys: BTreeSet<&(String, String)> = recorded.keys().chain(derived_groups.keys()).collect();
    let mut append: Vec<Transition> = Vec::new();
    let mut mismatch = 0u32;
    for k in keys {
        let rec: &[bool] = recorded.get(k).map(|v| v.as_slice()).unwrap_or(&[]);
        let der: &[&Transition] = derived_groups.get(k).map(|v| v.as_slice()).unwrap_or(&[]);
        let prefix_ok =
            rec.len() <= der.len() && rec.iter().zip(der.iter()).all(|(r, d)| *r == d.demoted);
        if !prefix_ok {
            mismatch += 1;
            continue;
        }
        append.extend(der[rec.len()..].iter().map(|t| (*t).clone()));
    }
    let n = append.len() as u32;
    ScanPlan {
        report: ScanReport {
            outcomes_scanned,
            transitions_total: derived.len() as u32,
            promotions_recorded,
            promotions_appended: n,
            alerts_appended: n,
            marker_mismatch: mismatch,
            dry_run: false,
        },
        append,
    }
}

/// Load, plan, and (unless dry) append: one promotion row + one alert per
/// unrecorded transition. Promotion first, alert second — the alert's detail
/// carries the promotion's seq, so the operator can jump from alert to
/// ledger fact. A mid-loop append failure returns the error with everything
/// already written left standing (append-only law: nothing is undone).
pub fn run_scan(ledger: &Ledger, alerts: &Alerts, dry_run: bool) -> Result<ScanReport, ScanErr> {
    let loaded = ledger.load()?;
    let mut plan = plan_scan(&loaded);
    plan.report.dry_run = dry_run;
    if dry_run {
        return Ok(plan.report);
    }
    for t in &plan.append {
        let pseq = ledger.append(&Row::Promotion(PromotionRow {
            lane_id: t.lane_id.clone(),
            task_class: t.class.clone(),
            demoted: t.demoted,
            trailing_fails: t.trailing_fails,
        }))?;
        alerts.append(&Alert {
            kind: if t.demoted {
                AlertKind::Degraded
            } else {
                AlertKind::Healed
            },
            lane_id: t.lane_id.clone(),
            class: t.class.clone(),
            detail: format!(
                "promotion seq {pseq}: trailing RED-TEST fails {} — lane {}",
                t.trailing_fails,
                if t.demoted {
                    "demoted persistent (R2)"
                } else {
                    "healed, one pass (QQ2)"
                }
            ),
        })?;
    }
    Ok(plan.report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::parse_stream;
    use std::fs;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rtr-alerts-{}-{}", tag, std::process::id()));
        fs::create_dir_all(&d).unwrap();
        d
    }

    /// Hand-written wire lines (cli.rs convention): exact `ledger::encode`
    /// output shape, so derivation is tested against the REAL stream format.
    fn out(seq: u64, lane: &str, class: &str, pass: bool) -> String {
        format!(
            "{{\"seq\":{seq},\"ts\":\"2026-08-28T00:00:00Z\",\"kind\":\"outcome\",\"card_id\":\"C\",\"task_class\":\"{class}\",\"lane_id\":\"{lane}\",\"model\":\"m\",\"cost_tokens\":1,\"cost_usd_est\":0.001,\"latency_ms\":10,\"verify_outcome\":\"{}\",\"escalated_to\":null}}",
            if pass { "pass" } else { "fail" }
        )
    }
    fn prom(seq: u64, lane: &str, class: &str, demoted: bool) -> String {
        format!(
            "{{\"seq\":{seq},\"ts\":\"2026-08-28T00:00:00Z\",\"kind\":\"promotion\",\"lane_id\":\"{lane}\",\"task_class\":\"{class}\",\"demoted\":{demoted},\"trailing_fails\":2}}"
        )
    }
    fn loaded(lines: &[String]) -> Loaded {
        let text = lines.join("\n");
        let l = parse_stream(&text);
        assert_eq!(l.bad.len(), 0, "fixture must be clean wire format");
        l
    }

    #[test]
    fn demote_at_hysteresis_exactly_once() {
        let l = loaded(&[
            out(1, "groq", "coding", false),
            out(2, "groq", "coding", false),
        ]);
        let t = transitions(&l);
        assert_eq!(t.len(), 1);
        assert!(t[0].demoted);
        assert_eq!(t[0].trailing_fails, 2);
        assert_eq!(t[0].after_seq, 2);
        // A third fail does NOT re-demote: still one persistent state.
        let l2 = loaded(&[
            out(1, "groq", "coding", false),
            out(2, "groq", "coding", false),
            out(3, "groq", "coding", false),
        ]);
        assert_eq!(transitions(&l2).len(), 1);
    }

    #[test]
    fn single_fail_is_not_a_transition() {
        let l = loaded(&[out(1, "groq", "coding", false)]);
        assert!(transitions(&l).is_empty());
    }

    #[test]
    fn heal_then_re_demote_alternates() {
        let l = loaded(&[
            out(1, "groq", "coding", false),
            out(2, "groq", "coding", false),
            out(3, "groq", "coding", true),
            out(4, "groq", "coding", false),
            out(5, "groq", "coding", false),
        ]);
        let t = transitions(&l);
        let flags: Vec<bool> = t.iter().map(|x| x.demoted).collect();
        assert_eq!(flags, vec![true, false, true]);
        assert_eq!(t[1].after_seq, 3);
        assert_eq!(t[1].trailing_fails, 0);
        assert_eq!(t[2].after_seq, 5);
    }

    #[test]
    fn lanes_and_classes_are_independent() {
        let l = loaded(&[
            out(1, "groq", "coding", false),
            out(2, "groq", "coding", false),
            out(3, "nemotron", "coding", false),
            out(4, "groq", "review", false),
            out(5, "groq", "review", false),
        ]);
        let t = transitions(&l);
        assert_eq!(t.len(), 2);
        assert!(t.iter().all(|x| x.demoted));
        assert!(t.iter().any(|x| x.lane_id == "groq" && x.class == "review"));
    }

    #[test]
    fn promotion_markers_are_not_evidence() {
        // A marker ALONE must derive nothing — decay is outcome evidence or
        // it is a claim (QQ1a spirit).
        let l = loaded(&[prom(1, "groq", "coding", true)]);
        assert!(transitions(&l).is_empty());
        // Interleaved marker does not change derivation either.
        let l2 = loaded(&[
            out(1, "groq", "coding", false),
            prom(2, "groq", "coding", true),
            out(3, "groq", "coding", false),
        ]);
        let t = transitions(&l2);
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].after_seq, 3);
    }

    #[test]
    fn plan_prefix_law_appends_only_suffix() {
        let full = loaded(&[
            out(1, "groq", "coding", false),
            out(2, "groq", "coding", false),
            out(3, "groq", "coding", true),
        ]);
        // Nothing recorded: both transitions pending.
        let p0 = plan_scan(&full);
        assert_eq!(p0.append.len(), 2);
        assert_eq!(p0.report.marker_mismatch, 0);
        // First transition recorded (marker seq 4): only heal pending.
        let one = loaded(&[
            out(1, "groq", "coding", false),
            out(2, "groq", "coding", false),
            prom(3, "groq", "coding", true),
            out(4, "groq", "coding", true),
        ]);
        let p1 = plan_scan(&one);
        assert_eq!(p1.append.len(), 1);
        assert!(!p1.append[0].demoted);
        assert_eq!(p1.report.promotions_recorded, 1);
        // Both recorded: nothing pending — the idempotent re-scan.
        let two = loaded(&[
            out(1, "groq", "coding", false),
            out(2, "groq", "coding", false),
            prom(3, "groq", "coding", true),
            out(4, "groq", "coding", true),
            prom(5, "groq", "coding", false),
        ]);
        let p2 = plan_scan(&two);
        assert!(p2.append.is_empty());
        assert_eq!(p2.report.transitions_total, 2);
    }

    #[test]
    fn plan_counts_mismatch_and_never_repairs() {
        // Recorded says healed FIRST; derived history starts with demoted —
        // not a prefix. The pair is skipped and counted, nothing appended.
        let l = loaded(&[
            out(1, "groq", "coding", false),
            out(2, "groq", "coding", false),
            prom(3, "groq", "coding", false),
        ]);
        let p = plan_scan(&l);
        assert_eq!(p.report.marker_mismatch, 1);
        assert!(p.append.is_empty());
        // More recorded than derived is also a mismatch (orphan markers).
        let l2 = loaded(&[
            prom(1, "groq", "coding", true),
            prom(2, "groq", "coding", false),
        ]);
        let p2 = plan_scan(&l2);
        assert_eq!(p2.report.marker_mismatch, 1);
        assert!(p2.append.is_empty());
    }

    #[test]
    fn run_scan_appends_then_is_idempotent() {
        let dir = tmpdir("scan");
        let led = Ledger::new(dir.join("ledger.jsonl"));
        let alr = Alerts::new(dir.join("alerts.jsonl"));
        led.append_ts(
            &Row::Outcome(crate::ledger::OutcomeRow {
                card_id: "C".into(),
                task_class: "coding".into(),
                lane_id: "groq".into(),
                model: "m".into(),
                cost_tokens: 1,
                cost_usd_est: 0.001,
                latency_ms: 10,
                outcome: Outcome::Fail,
                escalated_to: None,
            }),
            "2026-08-28T00:00:00Z",
            LOCK_WAIT,
        )
        .unwrap();
        led.append_ts(
            &Row::Outcome(crate::ledger::OutcomeRow {
                card_id: "C".into(),
                task_class: "coding".into(),
                lane_id: "groq".into(),
                model: "m".into(),
                cost_tokens: 1,
                cost_usd_est: 0.001,
                latency_ms: 10,
                outcome: Outcome::Fail,
                escalated_to: None,
            }),
            "2026-08-28T00:00:01Z",
            LOCK_WAIT,
        )
        .unwrap();
        let rep = run_scan(&led, &alr, false).unwrap();
        assert_eq!(rep.promotions_appended, 1);
        assert_eq!(rep.alerts_appended, 1);
        assert!(!rep.dry_run);
        // Ledger gained the marker; alerts gained the degraded row.
        let l = led.load().unwrap();
        assert!(matches!(
            l.rows.last().map(|p| &p.row),
            Some(Row::Promotion(_))
        ));
        let a = alr.load().unwrap();
        assert_eq!(a.bad.len(), 0);
        assert_eq!(a.rows.len(), 1);
        assert_eq!(a.rows[0].alert.kind, AlertKind::Degraded);
        assert_eq!(a.rows[0].seq, 1);
        // Re-scan: prefix complete, nothing new.
        let rep2 = run_scan(&led, &alr, false).unwrap();
        assert_eq!(rep2.promotions_appended, 0);
        assert_eq!(alr.load().unwrap().rows.len(), 1);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn run_scan_dry_run_writes_nothing() {
        let dir = tmpdir("dry");
        let led = Ledger::new(dir.join("ledger.jsonl"));
        let alr = Alerts::new(dir.join("alerts.jsonl"));
        for _ in 1..=2u32 {
            led.append_ts(
                &Row::Outcome(crate::ledger::OutcomeRow {
                    card_id: "C".into(),
                    task_class: "coding".into(),
                    lane_id: "groq".into(),
                    model: "m".into(),
                    cost_tokens: 1,
                    cost_usd_est: 0.001,
                    latency_ms: 10,
                    outcome: Outcome::Fail,
                    escalated_to: None,
                }),
                "2026-08-28T00:00:00Z",
                LOCK_WAIT,
            )
            .unwrap();
        }
        let rep = run_scan(&led, &alr, true).unwrap();
        assert!(rep.dry_run);
        assert_eq!(rep.promotions_appended, 1, "dry run still reports intent");
        assert!(led
            .load()
            .unwrap()
            .rows
            .iter()
            .all(|p| !matches!(p.row, Row::Promotion(_))));
        assert!(
            !alr.path().exists(),
            "dry run never creates the alert stream"
        );
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn alerts_append_grows_seq_and_roundtrips() {
        let dir = tmpdir("append");
        let alr = Alerts::new(dir.join("alerts.jsonl"));
        let weird = "la\"ne\\x\n\t🪰";
        let a1 = Alert {
            kind: AlertKind::EscalationStop,
            lane_id: weird.into(),
            class: "coding".into(),
            detail: "chain spent MAX_HOPS decisions — fail-safe halt".into(),
        };
        let a2 = Alert {
            kind: AlertKind::Healed,
            lane_id: "groq".into(),
            class: "review".into(),
            detail: "promotion seq 7: trailing RED-TEST fails 0 — lane healed, one pass (QQ2)"
                .into(),
        };
        let s1 = alr.append_ts(&a1, "t\"s\\", LOCK_WAIT).unwrap();
        let s2 = alr.append_ts(&a2, "t2", LOCK_WAIT).unwrap();
        assert_eq!((s1, s2), (1, 2));
        let loaded = alr.load().unwrap();
        assert_eq!(loaded.bad.len(), 0);
        assert_eq!(loaded.rows.len(), 2);
        assert_eq!(loaded.rows[0].alert, a1, "escaping survives the round trip");
        assert_eq!(loaded.rows[1].alert, a2);
        assert_eq!(loaded.max_seq(), 2);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn parse_alert_stream_reports_bad_lines_honestly() {
        let text = "not json\n{\"seq\":1,\"ts\":\"t\",\"kind\":\"degraded\",\"lane_id\":\"g\",\"class\":\"c\",\"detail\":\"d\"}\n{\"seq\":0,\"ts\":\"t\",\"kind\":\"healed\",\"lane_id\":\"g\",\"class\":\"c\",\"detail\":\"d\"}\n{\"seq\":2,\"ts\":\"t\",\"kind\":\"mystery\",\"lane_id\":\"g\",\"class\":\"c\",\"detail\":\"d\"}";
        let l = parse_alert_stream(text);
        assert_eq!(l.rows.len(), 1);
        assert_eq!(l.bad.len(), 3);
        assert_eq!(l.max_seq(), 1);
    }

    #[test]
    fn escalation_stop_alert_maps_every_variant() {
        let ctx = EscalationCtx {
            task_class: "coding".into(),
            data_class: crate::lane::DataClass::Internal,
            failed_lane_id: "groq".into(),
            hops_so_far: 1,
            chain_spent_usd: 0.01,
        };
        let variants = [
            EscalationErr::MaxHops,
            EscalationErr::NoFloorForClass,
            EscalationErr::NoCeilingForClass,
            EscalationErr::UnknownFailedLane,
            EscalationErr::TopOfLadder,
            EscalationErr::OverCeiling,
        ];
        let mut details = std::collections::BTreeSet::new();
        for v in &variants {
            let a = Alert::from_escalation_stop(&ctx, v);
            assert_eq!(a.kind, AlertKind::EscalationStop);
            assert_eq!(a.lane_id, "groq");
            assert_eq!(a.class, "coding");
            assert!(!a.detail.is_empty());
            details.insert(a.detail);
        }
        assert_eq!(
            details.len(),
            variants.len(),
            "every variant speaks its own reason"
        );
    }

    #[test]
    fn route_stop_alert_maps_every_variant() {
        let variants = [
            RouteErr::NoAliveLane,
            RouteErr::NoPermittedLane,
            RouteErr::NoMeasuredLane,
            RouteErr::NoFloorForClass,
            RouteErr::BelowFloorFailClosed,
        ];
        let mut details = std::collections::BTreeSet::new();
        for v in &variants {
            let a = Alert::from_route_stop(DataClass::Secret, "coding", v);
            assert_eq!(a.kind, AlertKind::RouteStop);
            assert_eq!(a.lane_id, "", "no lane was chosen");
            assert_eq!(a.class, "coding");
            assert!(!a.detail.is_empty());
            details.insert(a.detail);
        }
        assert_eq!(
            details.len(),
            variants.len(),
            "every variant speaks its own reason"
        );
        // The data class names WHICH filter closed (F5 coordinates).
        assert!(
            Alert::from_route_stop(DataClass::Secret, "coding", &RouteErr::NoPermittedLane)
                .detail
                .contains("secret")
        );
    }

    #[test]
    fn route_stop_kind_survives_the_wire() {
        let a = Alert::from_route_stop(DataClass::Pii, "chair", &RouteErr::NoPermittedLane);
        let line = encode_alert(41, "2026-08-28T07:00:00Z", &a);
        let loaded = parse_alert_stream(&line);
        assert!(loaded.bad.is_empty());
        assert_eq!(loaded.rows.len(), 1);
        assert_eq!(loaded.rows[0].alert, a);
        assert_eq!(loaded.rows[0].seq, 41);
    }
}
