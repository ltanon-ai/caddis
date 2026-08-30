//! worker_board_sections.rs — the per-section renderers of the worker
//! board, split out of worker_board.rs under the CCN law (gate ccn ≤ 10
//! per function; the monolithic render measured 33). Same organ, same
//! law: read-only, never spawns, never writes. CARD-0243: variable
//! sections pad to FIXED row counts and the feed/fold/eddy lenses show
//! the estate's unique mechanisms live.

use std::path::Path;

use crate::worker_board_frame::{self as fr, Frame};
use crate::worker_board_state as st;
use crate::worker_board_state::Page;
use crate::worker_board_tail;

pub(crate) fn opt(v: Option<u64>) -> String {
    v.map(|n| n.to_string()).unwrap_or_else(|| "-".into())
}

pub(crate) fn arm(f: &mut Frame, arm: &st::Arm, pace: &str, mem_pct: u64, scan_txt: &str) {
    f.strip(pace, &format!("mem {mem_pct}% │ scan {scan_txt}"));
    f.section("⚙", "ARM");
    f.row(
        "receipt",
        &format!(
            "kind={} model={} pane={} pace={}",
            arm.kind, arm.model, arm.pane, arm.pace
        ),
        fr::CYAN,
    );
    let pace_color = pace_color(pace);
    f.row("pace", pace, pace_color);
}

fn pace_color(pace: &str) -> &'static str {
    if pace.contains("STOP") {
        fr::RED
    } else if pace.contains("BUSY") {
        fr::YELLOW
    } else {
        fr::GREEN
    }
}

/// CARD-0256: when `idle` is true (empty queue, no live bee), the
/// phases.log tail is a ghost — render the idle marker, never the
/// stale card/phase. The log stays append-only; this is a display
/// truthfulness fix.
pub(crate) fn phase(f: &mut Frame, phase: &Option<(String, String)>, idle: bool) {
    f.section("⧗", "PHASE");
    if idle {
        // FIXED ROWS: two rows to match the Some arm's height.
        f.row("card", "·", fr::TEXT);
        f.row("phase", "idle", fr::TEXT);
        return;
    }
    match phase {
        Some((pcard, pphase)) => {
            let pc = if pphase == "fail" {
                fr::RED
            } else {
                fr::YELLOW
            };
            f.row("card", pcard, pc);
            f.row("phase", pphase, pc);
        }
        None => {
            // FIXED ROWS: the Some arm renders two rows; pad to match.
            f.row("phase", "none", fr::TEXT);
            f.row("card", "·", fr::TEXT);
        }
    }
}

pub(crate) fn queue(f: &mut Frame, q: &st::Queue) {
    f.section("▸", "QUEUE");
    f.row(
        "depth",
        &format!("remaining={} done={}", q.remaining.len(), q.done),
        fr::TEXT,
    );
    // FIXED ROWS (CARD-0243): always five card rows, padded — the
    // frame height must not depend on queue depth.
    for n in 0..5 {
        let text = match q.remaining.get(n) {
            Some(line) => format!("{} {}", if n == 0 { "▶" } else { " " }, line),
            None => "·".to_string(),
        };
        f.row("card", &text, fr::YELLOW);
    }
}

/// THE EVENT FEED (CARD-0243): last 5 events merged across the
/// lineage's journals, newest first, ALWAYS five rows (padded).
pub(crate) fn events(f: &mut Frame, dir: &Path) {
    f.section("⟫", "EVENTS (last 5)");
    let feed = worker_board_tail::last_events(dir, 5);
    for i in 0..5 {
        match feed.get(i) {
            Some(e) => f.row("evt", &e.text, fr::TEXT),
            None => f.row("evt", "·", fr::TEXT),
        }
    }
}

/// The FOLD lens (CARD-0243): threshold, state, and the context bar —
/// the fold/unfold organ, visible live. CARD-0247: when over-capacity,
/// render the TRUE number, name the subject, and append the remedy.
pub(crate) fn fold(f: &mut Frame, fold_at: u64, fold_state: &str, p: &Page) {
    f.section("◫", "FOLD / CONTEXT");
    f.row(
        "fold",
        &format!("threshold={fold_at}% state={fold_state}"),
        if fold_state == "warned" {
            fr::YELLOW
        } else {
            fr::GREEN
        },
    );
    // CARD-0247 SUBJECT: name the observed omp session's accounting —
    // never the worker's (the worker is a binary with no context).
    f.row("source", &format!("obs={}", p.session), fr::CYAN);
    let pct = p.pct.unwrap_or(0);
    if p.over {
        // CARD-0247 VALUE + ACTION: true ratio and remedy, never the
        // clamped 100%. Implementation in worker_board_over (sibling).
        crate::worker_board_over::render_over(f, p, pct);
    } else {
        f.bar(
            "ctx",
            pct,
            &format!(
                "stored={} sent={} pct={} stubbed={}",
                opt(p.stored),
                opt(p.sent),
                opt(p.pct),
                opt(p.stubbed)
            ),
        );
    }
}
/// The EDDY lens (CARD-0243): the loop organ's trail for THIS lineage
/// — dispatch verdicts, the unprovable streak, the last verdict.
pub(crate) fn eddy(f: &mut Frame, dir: &Path) {
    f.section("∋", "EDDY (loop organ)");
    let trail = worker_board_tail::eddy_trail(dir);
    let unprovable = trail
        .iter()
        .rev()
        .take_while(|t| t.status_class == caddis_organs::eddy::StatusClass::Unprovable)
        .count();
    let last = trail
        .last()
        .map(|t| t.status_class.as_str())
        .unwrap_or("none");
    let verdict = match caddis_organs::eddy::verdict(&trail) {
        caddis_organs::eddy::Verdict::Continue => "continue",
        caddis_organs::eddy::Verdict::Stagnant => "stagnant",
        caddis_organs::eddy::Verdict::UnprovableDone { .. } => "UNPROVABLE-DONE",
        caddis_organs::eddy::Verdict::Halt(_) => "halt",
    };
    let color = if verdict == "continue" {
        fr::GREEN
    } else {
        fr::YELLOW
    };
    f.row(
        "trail",
        &format!(
            "ticks={} last={} unprov-streak={}/3 verdict={}",
            trail.len(),
            last,
            unprovable,
            verdict
        ),
        color,
    );
    // Last 3 tick statuses, fixed rows:
    for i in 0..3 {
        let text = match trail.len().checked_sub(3 - i) {
            Some(k) => format!("seq={} {}", trail[k].seq, trail[k].status_class.as_str()),
            None => "·".to_string(),
        };
        f.row("tick", &text, fr::TEXT);
    }
}

pub(crate) fn scan(f: &mut Frame, scan: &Option<st::Scan>, live: &Option<String>) {
    f.section("✦", "SCAN");
    // FIXED ROWS: the live row always renders (padded when absent).
    f.row("live", live.as_deref().unwrap_or("·"), fr::YELLOW);
    match scan {
        Some(s) => {
            let vc = if s.verdict == "pass" {
                fr::GREEN
            } else {
                fr::RED
            };
            f.row("verdict", &format!("verdict={}", s.verdict), vc);
            let marks: Vec<String> = s
                .checks
                .iter()
                .map(|(name, ok)| check_mark(name, *ok))
                .collect();
            f.row("checks", &marks.join("  "), fr::TEXT);
        }
        None => {
            f.row("verdict", "none", fr::TEXT);
            f.row("checks", "·", fr::TEXT);
        }
    }
}

fn check_mark(name: &str, ok: bool) -> String {
    let (m, c) = if ok {
        ("✓", fr::GREEN)
    } else {
        ("✗", fr::RED)
    };
    format!("{c}{m} {name}{}", fr::RESET)
}

pub(crate) fn bee(f: &mut Frame, bees: &[st::Bee], tools: &[(String, usize)]) {
    f.section("❯", "BEE");
    // FIXED ROWS: always three run rows, padded (CARD-0243).
    for n in 0..3 {
        let (text, color) = match bees.get(n) {
            Some(b) => {
                let (sym, color) = if b.exit == 0 {
                    ("✓", fr::GREEN)
                } else {
                    ("✗", fr::RED)
                };
                (
                    format!(
                        "{sym} card={} argv0={} exit={} {}",
                        b.card,
                        b.argv0,
                        b.exit,
                        crate::worker_board_tail::hms_local(&b.ts)
                    ),
                    color,
                )
            }
            None => ("·".to_string(), fr::TEXT),
        };
        f.row("run", &text, color);
    }
    let tool_s: Vec<String> = tools.iter().map(|(t, n)| format!("{t}×{n}")).collect();
    f.row("tools", &tool_s.join("  "), fr::CYAN);
}

pub(crate) fn page(f: &mut Frame, p: &st::Page) {
    f.section("▤", "PAGE");
    f.row("mode", &format!("mode={}", p.mode), fr::TEXT);
    f.row("session", &format!("session={}", p.session), fr::TEXT);
    f.row(
        "mark",
        &format!("mark={}", if p.mark.is_empty() { "-" } else { &p.mark }),
        fr::TEXT,
    );
    f.row("spans", &format!("cold={}", p.cold), fr::TEXT);
    f.row(
        "evict",
        &format!("stubbed={} evicted={}", opt(p.stubbed), opt(p.evicted)),
        fr::TEXT,
    );
    let usage = usage_text(&p.usage);
    f.row("usage", &usage, fr::CYAN);
}

fn usage_text(usage: &[(String, u64)]) -> String {
    if usage.is_empty() {
        return "-".to_string();
    }
    usage
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(" ")
}
