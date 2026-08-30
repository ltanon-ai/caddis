//! worker_board.rs — CARD-0217/0226/0243. The informative window
//! organ: one-shot render, and `--watch` — the FIXED live view (no
//! scroll, in-place redraw, values change; the frame height is
//! CONSTANT by law). One-shot, read-only: never spawns, never writes,
//! never drain::.

use std::env;

use crate::lineage;
use crate::worker_board_frame::Frame;
use crate::worker_board_sections as sections;
use crate::worker_board_state as st;

pub enum Error {
    Usage(String),
    Fail(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Usage(s) | Error::Fail(s) => write!(f, "{s}"),
        }
    }
}

struct BoardArgs {
    session: Option<String>,
    watch: bool,
    interval_ms: u64,
    frames: Option<u64>,
}

fn parse_args(rest: &[String]) -> Result<BoardArgs, Error> {
    let mut a = BoardArgs {
        session: None,
        watch: false,
        interval_ms: 1_000,
        frames: None,
    };
    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            "--session" => {
                i += 1;
                a.session = Some(
                    rest.get(i)
                        .cloned()
                        .ok_or_else(|| Error::Usage("missing --session value".into()))?,
                );
            }
            "--watch" => a.watch = true,
            "--interval-ms" => {
                i += 1;
                a.interval_ms = rest
                    .get(i)
                    .and_then(|v| v.parse().ok())
                    .ok_or_else(|| Error::Usage("--interval-ms must be a number".into()))?;
            }
            "--frames" => {
                i += 1;
                a.frames = Some(
                    rest.get(i)
                        .and_then(|v| v.parse().ok())
                        .ok_or_else(|| Error::Usage("--frames must be a number".into()))?,
                );
            }
            other => return Err(Error::Usage(format!("unknown argument {other}"))),
        }
        i += 1;
    }
    Ok(a)
}

pub fn run(args: &[String]) -> Result<(), Error> {
    let (id, rest) = lineage::take(args).map_err(Error::Usage)?;
    let opts = parse_args(&rest)?;
    let (session, watch, interval_ms, frames) =
        (opts.session, opts.watch, opts.interval_ms, opts.frames);
    if !watch {
        println!("{}", render(&id, session.as_deref())?);
        return Ok(());
    }
    // The FIXED live view: clear ONCE, then redraw in place — \x1b[H
    // homes the cursor and the constant-height frame overwrites every
    // prior line. No flicker, no scroll (CARD-0243).
    print!("\x1b[2J\x1b[H");
    let mut shown: u64 = 0;
    loop {
        let frame = crate::worker_dash::fixed_frame(&render(&id, session.as_deref())?);
        print!("\x1b[H{frame}\x1b[0m");
        use std::io::Write;
        let _ = std::io::stdout().flush(); // swallow: best-effort-telemetry — a dropped flush only delays one frame
        shown += 1;
        if frames == Some(shown) {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(interval_ms));
    }
}

fn render(id: &str, session: Option<&str>) -> Result<String, Error> {
    let home = env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(std::path::PathBuf::from)
        .ok_or_else(|| Error::Fail("HOME is unset".into()))?;
    let dir = lineage::dir(id).map_err(Error::Fail)?;
    if !dir.join("arm.receipt").is_file() {
        return Err(Error::Fail("no arm receipt on this lineage".into()));
    }
    let arm = st::arm_fields(&dir);
    let pace = st::pace_sentence(&dir).unwrap_or_else(|| "PACE unverified".into());
    let q = st::queue(&dir);
    let bee_busy = crate::worker_lock::is_busy(&dir);
    let bees = st::bee_recent(&dir, 3);
    let tools = st::tool_counts(&dir);
    let scan = st::scan_last(&dir);
    let scan_live = st::scan_live_last(&dir);
    let phase = st::phase_last(&dir);
    let (fold_at, fold_state) = st::fold_state(&home, &dir);
    let p = st::page(&home, id, session);

    let mut f = Frame::new();
    f.header(&format!("caddis worker board ── lineage {id}"));
    let scan_txt = scan
        .as_ref()
        .map(|s| s.verdict.clone())
        .unwrap_or_else(|| "none".into());
    sections::arm(&mut f, &arm, &pace, p.pct.unwrap_or(0), &scan_txt);
    let phase_idle = q.remaining.is_empty() && !bee_busy;
    sections::phase(&mut f, &phase, phase_idle);
    sections::queue(&mut f, &q);
    sections::events(&mut f, &dir);
    sections::fold(&mut f, fold_at, fold_state, &p);
    sections::eddy(&mut f, &dir);
    sections::scan(&mut f, &scan, &scan_live);
    sections::bee(&mut f, &bees, &tools);
    sections::page(&mut f, &p);
    Ok(f.finish())
}
