//! soak.rs — QQ4 SOAK TELEMETRY (T-35 verdict: R-C counters + R-D
//! measured-first instrument; MODE 2026-08-27 "next build").
//!
//! Two halves, one law each:
//!
//! - **R-C (counters):** everything the soak report must review is
//!   OBSERVABLE live in `/health` — per-LANE say counters (a lane is a
//!   generator; D8/F-A8: offline EN and network LT soak SEPARATELY, so
//!   nothing here is organ-wide only), the horn's transcribe counter, and
//!   the F-A1 detection telemetry (decisions per layer, cache hit/miss,
//!   cap-forced fallbacks, tie-breaks, mixed utterances). Misroute count
//!   needs ground truth and therefore rides the P3 loopback-STT slice —
//!   its slot in the report is documented, not invented here.
//! - **R-D (measured-first):** availability cannot be measured from a
//!   process that forgets on restart. Every terminal outcome appends one
//!   JSONL row to `<home>/soak-ledger.jsonl`; `/health` computes trailing
//!   window success rates (1h / 24h / 48h / all) straight off that file.
//!   The 48h soak gate and the 2-week LT baseline then read the SAME
//!   ledger the counters were derived from — one truth, restart-proof.
//!   Volume math: ~120 B/row at speech scale (~10² rows/day) ≈ 170 KiB
//!   for a 2-week baseline — no rotation machinery in v1.
//!
//! Routing-level degrades (R-B confirm degrade, registry inconsistency)
//! are NOT lane health: they ledger under [`ROUTE_LANE`] (`_route`) and
//! are excluded from lane availability windows by the leading-underscore
//! rule — a config gap must not drag a healthy lane's number, and a dead
//! lane must not hide behind a routing fix.
//!
//! Recording is best-effort by design on the IO side (a full disk must
//! never stop speech; the miss is eprintln'd loudly) and EXACT on the
//! counting side (one lock guards counters + the append, so JSONL lines
//! never interleave).

use crate::detect::Utterance;
use crate::json::{self, Value};
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// Routing-health pseudo-lane (leading underscore = excluded from lane
/// availability windows; see module doc).
pub const ROUTE_LANE: &str = "_route";
/// The STT lane's id in counters and the ledger.
pub const HORN_LANE: &str = "horn";

/// Trailing windows `/health` publishes (label, seconds). `all` is added
/// by [`SoakShared::windows`] itself.
const WINDOWS: [(&str, u64); 3] = [("1h", 3600), ("24h", 86_400), ("48h", 172_800)];

fn wall_s() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Per-lane lifetime counters (since organ boot; the durable view is the
/// ledger windows). One speaks-and-drops domain per lane.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LaneCounters {
    pub attempts: u64,
    pub spoke: u64,
    pub dropped: u64,
    pub degraded: u64,
    pub cache_hits: u64,
    pub total_ms: u64,
    pub max_ms: u64,
}

/// F-A1 per-decision detection telemetry, aggregated (R-C).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DetectTelemetry {
    pub l0_declared: u64,
    pub l1_diacritic: u64,
    pub l2_trigram: u64,
    pub cap_fallback: u64,
    pub tie_break: u64,
    pub mixed: u64,
    pub truncated: u64,
    pub cache_hit: u64,
    pub cache_miss: u64,
}

/// Consistent clone of everything under the lock.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SoakSnapshot {
    pub lanes: Vec<(String, LaneCounters)>,
    pub detect: DetectTelemetry,
}

/// One lane's ok/total inside a trailing window.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct WindowStat {
    pub ok: u64,
    pub total: u64,
    pub rate: f64,
}
/// Trailing-window availability computed off the ledger (R-D).
#[derive(Debug, Clone, Default)]
pub struct SoakWindows {
    /// One entry per window label, in publish order.
    pub windows: Vec<OneWindow>,
    pub unparsed: u64,
}

/// One window's per-lane stats + the combined view.
#[derive(Debug, Clone, Default)]
pub struct OneWindow {
    pub label: String,
    /// Per-lane stats sorted by lane name.
    pub lanes: Vec<(String, WindowStat)>,
    pub combined: WindowStat,
}

#[derive(Debug)]
struct SoakInner {
    lanes: HashMap<String, LaneCounters>,
    detect: DetectTelemetry,
}

/// Organ-wide soak state: counters + the append-only ledger. Shared
/// between the say worker, the transcribe route, and `/health`
/// (the `spawned_children` pattern: one home, many writers).
#[derive(Debug)]
pub struct SoakShared {
    inner: Mutex<SoakInner>,
    ledger: Option<PathBuf>,
}

impl SoakShared {
    pub fn new(ledger: Option<PathBuf>) -> SoakShared {
        SoakShared {
            inner: Mutex::new(SoakInner {
                lanes: HashMap::new(),
                detect: DetectTelemetry::default(),
            }),
            ledger,
        }
    }

    /// One terminal say outcome on a render lane. Counted exactly, then
    /// ledgered best-effort (IO failure is loud, never fatal).
    pub fn record_say(&self, lane: &str, spoke: bool, cache_hit: bool, ms: u64) {
        let row = ledger_row("say", lane, spoke, cache_hit, ms);
        let mut g = self.inner.lock().expect("soak lock");
        let c = g.lanes.entry(lane.to_string()).or_default();
        c.attempts += 1;
        if spoke {
            c.spoke += 1;
            if cache_hit {
                c.cache_hits += 1;
            }
        } else {
            c.dropped += 1;
        }
        c.total_ms += ms;
        c.max_ms = c.max_ms.max(ms);
        self.append(&row);
    }

    /// A routing-level degrade (R-B confirm path / missing voice): route
    /// health, not lane health (module doc law).
    pub fn record_degrade(&self) {
        let row = ledger_row("say", ROUTE_LANE, false, false, 0);
        let mut g = self.inner.lock().expect("soak lock");
        let c = g.lanes.entry(ROUTE_LANE.to_string()).or_default();
        c.attempts += 1;
        c.dropped += 1;
        c.degraded += 1;
        self.append(&row);
    }

    /// One terminal transcribe outcome (the horn lane).
    pub fn record_transcribe(&self, ok: bool, ms: u64) {
        let row = ledger_row("transcribe", HORN_LANE, ok, false, ms);
        let mut g = self.inner.lock().expect("soak lock");
        let c = g.lanes.entry(HORN_LANE.to_string()).or_default();
        c.attempts += 1;
        if ok {
            c.spoke += 1;
        } else {
            c.dropped += 1;
        }
        c.total_ms += ms;
        c.max_ms = c.max_ms.max(ms);
        self.append(&row);
    }

    /// Fold one detection ladder result into the R-C telemetry.
    pub fn record_detect(&self, utt: &Utterance) {
        let mut g = self.inner.lock().expect("soak lock");
        let d = &mut g.detect;
        for seg in &utt.segments {
            match seg.decision.layer {
                crate::detect::Layer::L0Declared => d.l0_declared += 1,
                crate::detect::Layer::L1Diacritic => d.l1_diacritic += 1,
                crate::detect::Layer::L2Trigram => d.l2_trigram += 1,
                crate::detect::Layer::CapFallback => d.cap_fallback += 1,
            }
            if seg.decision.tie_break {
                d.tie_break += 1;
            }
        }
        if utt.mixed {
            d.mixed += 1;
        }
        if utt.truncated {
            d.truncated += 1;
        }
        if utt.from_cache {
            d.cache_hit += 1;
        } else {
            d.cache_miss += 1;
        }
    }

    /// Clone the counters under the lock (lanes sorted for a stable
    /// `/health` body).
    pub fn snapshot(&self) -> SoakSnapshot {
        let g = self.inner.lock().expect("soak lock");
        let mut lanes: Vec<(String, LaneCounters)> = g
            .lanes
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        lanes.sort_by(|a, b| a.0.cmp(&b.0));
        SoakSnapshot {
            lanes,
            detect: g.detect,
        }
    }

    /// Trailing-window availability from the ledger file. Corrupt rows are
    /// skipped and counted (`unparsed`) — honest, never guessed. Lanes
    /// with zero rows inside a window are omitted from that window (a
    /// rate over total=0 is not a number).
    pub fn windows(&self) -> SoakWindows {
        let now = wall_s();
        let mut rows: Vec<(f64, String, bool)> = Vec::new();
        let mut unparsed = 0u64;
        if let Some(path) = &self.ledger {
            if let Ok(text) = std::fs::read_to_string(path) {
                for line in text.lines() {
                    if line.trim().is_empty() {
                        continue;
                    }
                    match json::parse(line) {
                        Ok(v) => {
                            let (Some(ts), Some(lane), Some(ok)) = (
                                v.get("ts").and_then(Value::as_f64),
                                v.get("lane").and_then(Value::as_str),
                                v.get("ok").and_then(Value::as_bool),
                            ) else {
                                unparsed += 1;
                                continue;
                            };
                            rows.push((ts, lane.to_string(), ok));
                        }
                        Err(_) => unparsed += 1,
                    }
                }
            }
        }
        let mut windows = Vec::new();
        for (label, secs) in WINDOWS.iter().chain(std::iter::once(&("all", u64::MAX))) {
            let floor = if *secs == u64::MAX {
                f64::NEG_INFINITY
            } else {
                now - *secs as f64
            };
            let mut lanes: HashMap<String, (u64, u64)> = HashMap::new();
            for (_ts, lane, ok) in rows.iter().filter(|(ts, _, _)| *ts >= floor) {
                // Underscore lanes are route health, not lane health.
                if lane.starts_with('_') {
                    continue;
                }
                let e = lanes.entry(lane.clone()).or_insert((0, 0));
                e.1 += 1;
                if *ok {
                    e.0 += 1;
                }
            }
            let mut per_lane: Vec<(String, WindowStat)> = lanes
                .into_iter()
                .map(|(lane, (ok, total))| (lane, stat(ok, total)))
                .collect();
            per_lane.sort_by(|a, b| a.0.cmp(&b.0));
            let (ok, total): (u64, u64) = per_lane
                .iter()
                .map(|(_, s)| (s.ok, s.total))
                .fold((0, 0), |(o, t), (so, st)| (o + so, t + st));
            windows.push(OneWindow {
                label: label.to_string(),
                lanes: per_lane,
                combined: stat(ok, total),
            });
        }
        SoakWindows { windows, unparsed }
    }

    /// The whole `/health` `"soak"` section: since-boot counters + the
    /// durable window view off the same ledger.
    pub fn health_value(&self) -> Value {
        let snap = self.snapshot();
        let win = self.windows();
        let lanes = Value::Obj(
            snap.lanes
                .iter()
                .map(|(lane, c)| {
                    (
                        lane.clone(),
                        Value::Obj(vec![
                            ("attempts".into(), Value::Num(c.attempts as f64)),
                            ("spoke".into(), Value::Num(c.spoke as f64)),
                            ("dropped".into(), Value::Num(c.dropped as f64)),
                            ("degraded".into(), Value::Num(c.degraded as f64)),
                            ("cache_hits".into(), Value::Num(c.cache_hits as f64)),
                            (
                                "avg_ms".into(),
                                Value::Num(if c.attempts == 0 {
                                    0.0
                                } else {
                                    c.total_ms as f64 / c.attempts as f64
                                }),
                            ),
                            ("max_ms".into(), Value::Num(c.max_ms as f64)),
                        ]),
                    )
                })
                .collect(),
        );
        let d = snap.detect;
        let detect = Value::Obj(vec![
            ("l0_declared".into(), Value::Num(d.l0_declared as f64)),
            ("l1_diacritic".into(), Value::Num(d.l1_diacritic as f64)),
            ("l2_trigram".into(), Value::Num(d.l2_trigram as f64)),
            ("cap_fallback".into(), Value::Num(d.cap_fallback as f64)),
            ("tie_break".into(), Value::Num(d.tie_break as f64)),
            ("mixed".into(), Value::Num(d.mixed as f64)),
            ("truncated".into(), Value::Num(d.truncated as f64)),
            ("cache_hit".into(), Value::Num(d.cache_hit as f64)),
            ("cache_miss".into(), Value::Num(d.cache_miss as f64)),
        ]);
        let windows = Value::Obj(
            win.windows
                .iter()
                .map(|w| {
                    (
                        w.label.clone(),
                        Value::Obj(vec![
                            (
                                "lanes".into(),
                                Value::Obj(
                                    w.lanes
                                        .iter()
                                        .map(|(lane, s)| (lane.clone(), stat_value(*s)))
                                        .collect(),
                                ),
                            ),
                            ("combined".into(), stat_value(w.combined)),
                        ]),
                    )
                })
                .collect(),
        );
        Value::Obj(vec![
            ("lanes".into(), lanes),
            ("detect".into(), detect),
            ("windows".into(), windows),
            (
                "unparsed_ledger_rows".into(),
                Value::Num(win.unparsed as f64),
            ),
        ])
    }

    /// Append one finished row to the ledger. Caller holds the inner
    /// lock (counters and row land atomically); failure is LOUD and
    /// non-fatal (module doc law).
    fn append(&self, row: &str) {
        let Some(path) = &self.ledger else { return };
        let mut f = match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            Ok(f) => f,
            Err(e) => {
                eprintln!(
                    "soak: ledger {}: {e} (row lost, counters kept)",
                    path.display()
                );
                return;
            }
        };
        if let Err(e) = f.write_all(row.as_bytes()) {
            eprintln!("soak: ledger write: {e} (row lost, counters kept)");
        }
    }
}

fn ledger_row(op: &str, lane: &str, ok: bool, cache_hit: bool, ms: u64) -> String {
    json::to_string(&Value::Obj(vec![
        ("ts".into(), Value::Num(wall_s())),
        ("op".into(), Value::Str(op.into())),
        ("lane".into(), Value::Str(lane.into())),
        ("ok".into(), Value::Bool(ok)),
        ("cache_hit".into(), Value::Bool(cache_hit)),
        ("ms".into(), Value::Num(ms as f64)),
    ])) + "\n"
}

/// Convenience for callers that own an Option (tests, degraded boots).
pub fn shared(ledger: Option<PathBuf>) -> Arc<SoakShared> {
    Arc::new(SoakShared::new(ledger))
}

fn stat(ok: u64, total: u64) -> WindowStat {
    WindowStat {
        ok,
        total,
        // Internal 0.0 for empty windows; serialization turns it into an
        // honest null (below) — a rate over zero attempts is not a
        // number, and NaN would make the whole /health body unparseable.
        rate: if total == 0 {
            0.0
        } else {
            ok as f64 / total as f64
        },
    }
}

fn stat_value(s: WindowStat) -> Value {
    Value::Obj(vec![
        ("ok".into(), Value::Num(s.ok as f64)),
        ("total".into(), Value::Num(s.total as f64)),
        (
            "rate".into(),
            if s.total == 0 {
                Value::Null
            } else {
                Value::Num((s.rate * 10_000.0).round() / 10_000.0)
            },
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::{DetectOptions, Detector};
    use std::fs;

    fn tmpdir(name: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("caddis-voice-soak-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn counters_match_ledger_rows() {
        let dir = tmpdir("counters");
        let path = dir.join("soak-ledger.jsonl");
        let s = SoakShared::new(Some(path.clone()));

        s.record_say("piper", true, false, 120);
        s.record_say("piper", false, false, 200);
        s.record_say("piper", true, true, 5);
        s.record_transcribe(true, 340);
        s.record_degrade();

        let snap = s.snapshot();
        let piper = snap
            .lanes
            .iter()
            .find(|(l, _)| l == "piper")
            .map(|(_, c)| c.clone())
            .expect("piper lane present");
        assert_eq!(piper.attempts, 3);
        assert_eq!(piper.spoke, 2);
        assert_eq!(piper.dropped, 1);
        assert_eq!(piper.cache_hits, 1);
        assert_eq!(piper.max_ms, 200);
        assert_eq!(piper.total_ms, 325);
        let horn = snap.lanes.iter().find(|(l, _)| l == HORN_LANE).unwrap();
        assert_eq!(horn.1.attempts, 1);
        assert_eq!(horn.1.spoke, 1);
        let route = snap.lanes.iter().find(|(l, _)| l == ROUTE_LANE).unwrap();
        assert_eq!((route.1.dropped, route.1.degraded), (1, 1));

        let text = fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 5, "one row per terminal outcome");
        let first = json::parse(lines[0]).unwrap();
        assert_eq!(first.get("op").and_then(Value::as_str), Some("say"));
        assert_eq!(first.get("lane").and_then(Value::as_str), Some("piper"));
        assert_eq!(first.get("ok").and_then(Value::as_bool), Some(true));
        assert_eq!(first.get("ms").and_then(Value::as_f64), Some(120.0));
        let horn_row = json::parse(lines[3]).unwrap();
        assert_eq!(
            horn_row.get("op").and_then(Value::as_str),
            Some("transcribe")
        );
        let route_row = json::parse(lines[4]).unwrap();
        assert_eq!(
            route_row.get("lane").and_then(Value::as_str),
            Some("_route")
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn windows_compute_per_lane_and_exclude_route() {
        let dir = tmpdir("windows");
        let path = dir.join("soak-ledger.jsonl");
        let now = wall_s();
        let row = |ts: f64, lane: &str, ok: bool| {
            format!(
                "{{\"ts\":{ts},\"op\":\"say\",\"lane\":\"{lane}\",\"ok\":{ok},\"cache_hit\":false,\"ms\":1}}\n"
            )
        };
        let mut text = String::new();
        text += &row(now - 10.0, "piper", true); // in every window
        text += &row(now - 10.0, "leonas", false); // network lane failure, 1h window
        text += &row(now - 2.0 * 3600.0, "piper", true); // out of 1h, in 24h/48h/all
        text += &row(now - 49.0 * 3600.0, "piper", false); // only in all
        text += &row(now - 10.0, "_route", false); // excluded everywhere
        text += "this line is not json\n"; // unparsed, skipped honestly
        fs::write(&path, text).unwrap();

        let s = SoakShared::new(Some(path));
        let w = s.windows();
        assert_eq!(w.unparsed, 1);

        let get = |label: &str, lane: &str| -> Option<WindowStat> {
            w.windows
                .iter()
                .find(|w| w.label == label)?
                .lanes
                .iter()
                .find(|(ln, _)| ln == lane)
                .map(|(_, s)| *s)
        };
        // 1h: piper 1/1, leonas 0/1.
        let p = get("1h", "piper").unwrap();
        assert_eq!((p.ok, p.total), (1, 1));
        let l = get("1h", "leonas").unwrap();
        assert_eq!((l.ok, l.total), (0, 1));
        assert!(get("1h", "_route").is_none(), "route lane never in windows");
        // 24h: piper 2/2.
        let p = get("24h", "piper").unwrap();
        assert_eq!((p.ok, p.total), (2, 2));
        // all: piper 2/3, leonas 0/1; combined 2/4 (route row excluded).
        let p = get("all", "piper").unwrap();
        assert_eq!((p.ok, p.total), (2, 3));
        let combined = &w
            .windows
            .iter()
            .find(|w| w.label == "all")
            .unwrap()
            .combined;
        assert_eq!((combined.ok, combined.total), (2, 4));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn detect_telemetry_counts_layers_and_cache() {
        let s = SoakShared::new(None);
        let mut d = Detector::new(DetectOptions::default());
        // Two sentences: marked LT (L1 decides) + unmarked EN (L2) — a
        // genuinely mixed utterance.
        let u1 = d.detect("Labas, čia diakritikai. Hello world friend.", None);
        s.record_detect(&u1);
        // Second call of the same text: verdict from cache.
        let u2 = d.detect("Labas, čia diakritikai. Hello world friend.", None);
        s.record_detect(&u2);
        let t = s.snapshot().detect;
        assert!(t.l1_diacritic >= 1, "diacritic segment decided at L1");
        assert!(t.l2_trigram >= 1, "unmarked EN segment decided at L2");
        assert_eq!(t.cache_hit, 1);
        assert_eq!(t.cache_miss, 1);
        assert!(t.mixed >= 1, "LT+EN utterance is mixed");
    }

    #[test]
    fn health_value_shape() {
        let dir = tmpdir("health");
        let s = SoakShared::new(Some(dir.join("soak-ledger.jsonl")));
        s.record_say("piper", true, false, 10);
        let v = s.health_value();
        assert!(v.get("lanes").and_then(Value::as_obj).is_some());
        assert!(v.get("detect").and_then(Value::as_obj).is_some());
        let windows = v.get("windows").and_then(Value::as_obj).unwrap();
        for label in ["1h", "24h", "48h", "all"] {
            assert!(
                windows.iter().any(|(k, _)| k == label),
                "window {label} present"
            );
        }
        let piper = v
            .get("lanes")
            .and_then(|l| l.get("piper"))
            .expect("piper in lanes");
        assert_eq!(piper.get("spoke").and_then(Value::as_f64), Some(1.0));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_ledger_yields_empty_windows() {
        let s = SoakShared::new(Some(PathBuf::from("Z:/definitely/not/here.jsonl")));
        let w = s.windows();
        assert_eq!(w.unparsed, 0);
        for w in &w.windows {
            assert!(w.lanes.is_empty());
            assert_eq!(w.combined.total, 0);
        }
    }

    #[test]
    fn health_body_is_parseable_json_even_when_empty() {
        let s = SoakShared::new(None);
        let body = json::to_string(&s.health_value());
        assert!(
            !body.contains("NaN"),
            "NaN would break every strict parser: {body}"
        );
        let v = json::parse(&body).expect("health body parses with our own parser");
        let all = v
            .get("windows")
            .and_then(|w| w.get("all"))
            .and_then(|w| w.get("combined"))
            .expect("all-window combined present");
        assert_eq!(
            all.get("rate"),
            Some(&Value::Null),
            "empty window rate is null"
        );
    }
}
