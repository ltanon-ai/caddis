//! page_report.rs — CARD-0160. Canary verdicts from the observe log.
//! Split from page.rs at the 280-line cap. Zero-dep: the log lines are
//! ours, so minimal substring readers suffice.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::page::Error;
use crate::page_report_tally::{tally_line, Tally};

fn newest_stem(dir: &Path) -> Result<String, Error> {
    let rd = fs::read_dir(dir).map_err(|e| Error::Fail(format!("no observe dir: {e}")))?;
    let mut best: Option<(std::time::SystemTime, String)> = None;
    for ent in rd {
        let ent = ent.map_err(|e| Error::Fail(format!("read observe dir: {e}")))?;
        let name = ent.file_name();
        let name = name.to_string_lossy();
        let Some(stem) = name.strip_suffix(".observe.jsonl") else {
            continue;
        };
        if stem.is_empty() {
            continue;
        }
        let mtime = ent
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::UNIX_EPOCH);
        let take = match &best {
            None => true,
            Some((t, s)) => mtime > *t || (mtime == *t && stem > s.as_str()),
        };
        if take {
            best = Some((mtime, stem.to_string()));
        }
    }
    best.map(|(_, s)| s)
        .ok_or_else(|| Error::Fail("no observe log".into()))
}

fn resolve_session(args: &[String], pager: &Path) -> Result<String, Error> {
    match crate::page::flag(args, "--session") {
        Ok(s) => Ok(s.to_string()),
        Err(_) => newest_stem(pager),
    }
}
fn pager_mode(pager: &Path, session: &str) -> &'static str {
    let v = env::var("CADDIS_PAGE_MODE").unwrap_or_default();
    if !v.is_empty() {
        return if v == "page" { "page" } else { "observe" };
    }
    // swallow: fail-safe-by-law
    if let Ok(s) = fs::read_to_string(pager.join(session).join("mode")) {
        return if s.trim() == "page" {
            "page"
        } else {
            "observe"
        };
    }
    match fs::read_to_string(pager.join("mode")) {
        Ok(s) if s.trim() == "page" => "page",
        _ => "observe",
    }
}

fn fold_at(args: &[String]) -> u64 {
    crate::page::flag(args, "--fold-at")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(90)
}
pub fn report(args: &[String]) -> Result<(), Error> {
    let home = env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or_else(|| Error::Fail("HOME is unset".into()))?;
    let pager = home.join(".caddis").join("pager");
    let session = resolve_session(args, &pager)?;
    let path = pager.join(format!("{session}.observe.jsonl"));
    let log = fs::read_to_string(&path)
        .map_err(|e| Error::Fail(format!("no observe log for {session}: {e}")))?;
    let mut t = Tally::default();
    for line in log.lines().filter(|l| !l.trim().is_empty()) {
        tally_line(line, &mut t);
    }
    println!("pager_mode={}", pager_mode(&pager, &session));
    if let Some(m) = crate::page_mark::resolve(&pager, &session) {
        println!("mark={m}");
        if let Some(se) = t.last_sent {
            println!("sent_mark_milli={}", se.saturating_mul(1000) / m);
        }
        if let Some(st) = t.last_stored {
            println!("stored_mark_milli={}", st.saturating_mul(1000) / m);
        }
    }
    print_tally(&session, &t, fold_at(args));
    Ok(())
}

fn print_tally(session: &str, t: &Tally, fold_at: u64) {
    println!("session={session}");
    println!("context_events={}", t.events);
    println!("parse_fail={}", t.fails);
    if t.events > 0 {
        println!(
            "parse_fail_milli={}",
            t.fails.saturating_mul(1000) / t.events
        );
    }
    println!("compact_before={}", t.before);
    println!("compact_auto={}", t.auto);
    println!("fault={}", t.n_fault);
    println!("ref={}", t.n_ref);
    if let Some(v) = t.last_recovery_ms {
        println!("last_recovery_ms={v}");
    }
    println!("last_n_messages={}", t.last_n.map_or(-1, |v| v as i64));
    println!("last_chars={}", t.last_chars.map_or(-1, |v| v as i64));
    println!(
        "last_largest_tool={}",
        t.last_largest.map_or(-1, |v| v as i64)
    );
    println!("last_custom={}", t.last_custom.map_or(-1, |v| v as i64));
    println!("last_user={}", t.last_user.map_or(-1, |v| v as i64));
    println!(
        "last_assistant={}",
        t.last_assistant.map_or(-1, |v| v as i64)
    );
    println!(
        "last_tool_result={}",
        t.last_tool_result.map_or(-1, |v| v as i64)
    );
    if let Some(p) = t.last_page_mode {
        println!("last_page_mode={p}");
    }
    println!(
        "last_n_stubbed={}",
        t.last_n_stubbed.map_or(-1, |v| v as i64)
    );
    println!(
        "last_n_evicted={}",
        t.last_n_evicted.map_or(-1, |v| v as i64)
    );
    println!(
        "last_user_chars={}",
        t.last_user_chars.map_or(-1, |v| v as i64)
    );
    println!(
        "last_assistant_chars={}",
        t.last_assistant_chars.map_or(-1, |v| v as i64)
    );
    println!(
        "last_tool_result_chars={}",
        t.last_tool_result_chars.map_or(-1, |v| v as i64)
    );
    print_stub_tool(t);
    println!("last_stored={}", t.last_stored.map_or(-1, |v| v as i64));
    println!("last_sent_est={}", t.last_sent.map_or(-1, |v| v as i64));
    if let (Some(st), Some(se)) = (t.last_stored, t.last_sent) {
        if se > 0 {
            println!("stored_sent_milli={}", st.saturating_mul(1000) / se);
        }
    }
    if let Some(p) = t.last_pct {
        println!("last_pct={p}");
    }
    if let Some(w) = t.last_window {
        println!("last_window={w}");
        if let Some(st) = t.last_stored {
            if w > 0 {
                let milli = st.saturating_mul(1000) / w;
                println!("stored_window_milli={milli}");
                println!(
                    "fold_headroom_milli={}",
                    fold_at.saturating_mul(10).saturating_sub(milli)
                );
            }
        }
    }
    crate::page_report_usage::print_usage(&t.last_usage);
}

fn print_stub_tool(t: &Tally) {
    let Some(tr) = t.last_tool_result.filter(|&n| n > 0) else {
        return;
    };
    let Some(st) = t.last_n_stubbed else { return };
    println!("stub_tool_milli={}", st.saturating_mul(1000) / tr);
}
