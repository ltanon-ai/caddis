//! The caddis-router executable.
//!
//! `caddis-router verify  [--ledger <path>] [--home <dir>] [--json]`
//! `caddis-router collect [--councils <dir>] [--ledger <path>] [--home <dir>] [--dry-run] [--json]`
//!
//! Both wire the library to the organ's real state home: `<home>/ledger.jsonl`,
//! default home `~/.caddis/router` (voice-organ convention). `verify` exits
//! with the finding COUNT (model-voice convention — a ledger tool reports
//! what IS, never silently repairs); a ledger that does not exist YET is
//! clean and says so. `collect` replays the council-consult archive into
//! outcome rows (F2: retroactive telemetry IS the spine) and exits 0 unless
//! usage/IO fails (2, loudly) — its honest skips are COUNTS in the report,
//! not exit-code noise.
//!
//! The append path stays in the LIBRARY (F1: no dispatch in the crate);
//! `collect` appends telemetry rows only — it never dispatches anything.

use caddis_router::{
    collect_bees, collect_councils, collect_tinyagi, verify_path, BeeReport, CollectReport, Ledger,
    TinyagiReport, VERSION,
};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "usage:
  caddis-router verify          [--ledger <path>] [--home <dir>] [--json]
  caddis-router collect         [--councils <dir>] [--ledger <path>] [--home <dir>] [--dry-run] [--json]
  caddis-router collect-bees    [--cards <path>] [--ledger <path>] [--home <dir>] [--dry-run] [--json]
  caddis-router collect-tinyagi [--tinyagi <dir>] [--ledger <path>] [--home <dir>] [--dry-run] [--json]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    }
    match args[0].as_str() {
        "verify" => run_verify(&args[1..]),
        "collect-bees" => run_collect_bees(&args[1..]),
        "collect-tinyagi" => run_collect_tinyagi(&args[1..]),
        "collect" => run_collect(&args[1..]),
        "--version" => {
            println!("caddis-router {VERSION}");
            ExitCode::SUCCESS
        }
        "--help" | "-h" => {
            println!("{USAGE}");
            println!("  verify:  audit a ledger (exit code = finding count)");
            println!("    --ledger <path>   verify this exact ledger file (wins over --home)");
            println!("    --home <dir>      organ state home (default ~/.caddis/router)");
            println!("    --json            machine report on stdout");
            println!("  collect: replay council consults as outcome rows (idempotent)");
            println!("    --councils <dir>  consult archive (default H:\\ai_temp\\councils)");
            println!("    --dry-run         report what would land, append nothing");
            println!(
                "  collect-bees: replay bee cards (BEE-CARDS.json) as outcome rows (idempotent)"
            );
            println!("    --cards <path>    bee card file (default the sergeant state home)");
            println!("    --dry-run         report what would land, append nothing");
            println!("  collect-tinyagi: replay trajectory runs as outcome rows (idempotent)");
            println!("    --tinyagi <dir>   tinyagi home (default ~/.tinyagi; brackets from");
            println!("                      settings.json snapshots, provable edges only)");
            println!("    --dry-run         report what would land, append nothing");
            ExitCode::SUCCESS
        }
        other => {
            eprintln!("caddis-router: unknown subcommand {other:?}");
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

// --- shared arg mini-parser ------------------------------------------------

/// One flag scanner shared by both subcommands: `--ledger`/`--home` resolve
/// the ledger path the same way everywhere; unknown argv fails closed (2).
enum Flag {
    Ledger(PathBuf),
    Json,
}

fn scan_common(args: &[String], i: &mut usize) -> Result<Flag, String> {
    match args[*i].as_str() {
        "--json" => {
            *i += 1;
            Ok(Flag::Json)
        }
        "--ledger" if *i + 1 < args.len() => {
            let p = PathBuf::from(&args[*i + 1]);
            *i += 2;
            Ok(Flag::Ledger(p))
        }
        "--home" if *i + 1 < args.len() => {
            let p = PathBuf::from(&args[*i + 1]).join("ledger.jsonl");
            *i += 2;
            Ok(Flag::Ledger(p))
        }
        other => Err(other.to_string()),
    }
}

// --- verify ----------------------------------------------------------------

fn run_verify(args: &[String]) -> ExitCode {
    let mut ledger: Option<PathBuf> = None;
    let mut json = false;
    let mut i = 0;
    while i < args.len() {
        match scan_common(args, &mut i) {
            Ok(Flag::Ledger(p)) => ledger = Some(p),
            Ok(Flag::Json) => json = true,
            Err(other) => {
                eprintln!("caddis-router verify: unknown argument {other:?}");
                eprintln!("{USAGE}");
                return ExitCode::from(2);
            }
        }
    }
    let ledger = ledger.unwrap_or_else(|| default_home().join("ledger.jsonl"));
    match verify_path(&ledger) {
        Ok(rep) => {
            let exists = ledger.exists();
            if json {
                print_json(&ledger, exists, &rep);
            } else {
                print_human(&ledger, exists, &rep);
            }
            // rc = finding count, capped where the process contract ends
            // (u8); a ledger with >255 findings reports the cap honestly.
            let rc = rep.rc().min(255) as u8;
            ExitCode::from(rc)
        }
        Err(e) => {
            eprintln!("caddis-router: verify {}: {e}", ledger.display());
            ExitCode::from(2)
        }
    }
}

fn print_human(ledger: &Path, exists: bool, rep: &caddis_router::VerifyReport) {
    if exists {
        println!("ledger: {}", ledger.display());
    } else {
        println!(
            "ledger: {} (missing — no decisions recorded yet)",
            ledger.display()
        );
    }
    println!(
        "lines: {} rows_ok: {} findings: {}",
        rep.lines,
        rep.rows_ok,
        rep.findings.len()
    );
    for f in &rep.findings {
        println!("  line {}: {}: {}", f.line, f.code, f.detail);
    }
}

/// Hand-rolled flat JSON (crate law: zero deps; the same two-character
/// escaping discipline as the ledger encoder — free text goes through
/// `esc`, numbers and bools are Raw by construction).
fn print_json(ledger: &Path, exists: bool, rep: &caddis_router::VerifyReport) {
    let findings: Vec<String> = rep
        .findings
        .iter()
        .map(|f| {
            format!(
                "{{\"line\":{},\"code\":\"{}\",\"detail\":\"{}\"}}",
                f.line,
                esc(f.code),
                esc(&f.detail)
            )
        })
        .collect();
    println!(
        "{{\"version\":\"{}\",\"ledger\":\"{}\",\"exists\":{},\"lines\":{},\"rows_ok\":{},\"rc\":{},\"findings\":[{}]}}",
        VERSION,
        esc(&ledger.display().to_string()),
        exists,
        rep.lines,
        rep.rows_ok,
        rep.rc().min(255),
        findings.join(",")
    );
}

// --- collect ---------------------------------------------------------------

fn run_collect(args: &[String]) -> ExitCode {
    let mut ledger: Option<PathBuf> = None;
    let mut councils: Option<PathBuf> = None;
    let mut json = false;
    let mut dry = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--councils" if i + 1 < args.len() => {
                councils = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "--dry-run" => {
                dry = true;
                i += 1;
            }
            _ => match scan_common(args, &mut i) {
                Ok(Flag::Ledger(p)) => ledger = Some(p),
                Ok(Flag::Json) => json = true,
                Err(other) => {
                    eprintln!("caddis-router collect: unknown argument {other:?}");
                    eprintln!("{USAGE}");
                    return ExitCode::from(2);
                }
            },
        }
    }
    let councils = councils.unwrap_or_else(|| PathBuf::from(r"H:\ai_temp\councils"));
    let lpath = ledger.unwrap_or_else(|| default_home().join("ledger.jsonl"));
    match collect_councils(&councils, &Ledger::new(&lpath), dry) {
        Ok(rep) => {
            if json {
                print_collect_json(&councils, &lpath, &rep);
            } else {
                print_collect_human(&councils, &lpath, &rep);
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("caddis-router: collect {}: {e}", councils.display());
            ExitCode::from(2)
        }
    }
}

fn print_collect_human(councils: &Path, lpath: &Path, rep: &CollectReport) {
    println!("councils: {}", councils.display());
    println!("ledger: {}", lpath.display());
    println!(
        "consults: {} rows: {} (pass {} / fail {}){}",
        rep.consults_seen,
        rep.rows,
        rep.passes,
        rep.fails,
        if rep.dry_run {
            " [dry-run: nothing written]"
        } else {
            ""
        }
    );
    println!(
        "skipped: {} no-manifest, {} bad-manifest, {} no-verdicts, {} bad-verdicts, {} seat-identity, {} seat-verdict, {} already",
        rep.skipped_no_manifest,
        rep.skipped_manifest_bad,
        rep.skipped_no_verdicts,
        rep.skipped_verdicts_bad,
        rep.skipped_seat_no_identity,
        rep.skipped_seat_no_verdict,
        rep.skipped_already
    );
}

fn print_collect_json(councils: &Path, lpath: &Path, rep: &CollectReport) {
    println!(
        "{{\"version\":\"{}\",\"councils\":\"{}\",\"ledger\":\"{}\",\"dry_run\":{},\"consults_seen\":{},\"rows\":{},\"passes\":{},\"fails\":{},\"skipped\":{{\"no_manifest\":{},\"manifest_bad\":{},\"no_verdicts\":{},\"verdicts_bad\":{},\"seat_no_identity\":{},\"seat_no_verdict\":{},\"already\":{}}}}}",
        VERSION,
        esc(&councils.display().to_string()),
        esc(&lpath.display().to_string()),
        rep.dry_run,
        rep.consults_seen,
        rep.rows,
        rep.passes,
        rep.fails,
        rep.skipped_no_manifest,
        rep.skipped_manifest_bad,
        rep.skipped_no_verdicts,
        rep.skipped_verdicts_bad,
        rep.skipped_seat_no_identity,
        rep.skipped_seat_no_verdict,
        rep.skipped_already
    );
}

// --- collect-bees ------------------------------------------------------------

fn run_collect_bees(args: &[String]) -> ExitCode {
    let mut ledger: Option<PathBuf> = None;
    let mut cards: Option<PathBuf> = None;
    let mut json = false;
    let mut dry = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--cards" if i + 1 < args.len() => {
                cards = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "--dry-run" => {
                dry = true;
                i += 1;
            }
            _ => match scan_common(args, &mut i) {
                Ok(Flag::Ledger(p)) => ledger = Some(p),
                Ok(Flag::Json) => json = true,
                Err(other) => {
                    eprintln!("caddis-router collect-bees: unknown argument {other:?}");
                    eprintln!("{USAGE}");
                    return ExitCode::from(2);
                }
            },
        }
    }
    let cards = cards
        .unwrap_or_else(|| PathBuf::from(r"C:\Users\ashpac\.omp\sergeant\state\BEE-CARDS.json"));
    let lpath = ledger.unwrap_or_else(|| default_home().join("ledger.jsonl"));
    match collect_bees(&cards, &Ledger::new(&lpath), dry) {
        Ok(rep) => {
            if json {
                print_collect_bees_json(&cards, &lpath, &rep);
            } else {
                print_collect_bees_human(&cards, &lpath, &rep);
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("caddis-router: collect-bees {}: {e}", cards.display());
            ExitCode::from(2)
        }
    }
}

fn print_collect_bees_human(cards: &Path, lpath: &Path, rep: &BeeReport) {
    println!("cards: {}", cards.display());
    println!("ledger: {}", lpath.display());
    println!(
        "cards seen: {} rows: {} (all pass — the bee trail has no fail record){}",
        rep.cards_seen,
        rep.rows,
        if rep.dry_run {
            " [dry-run: nothing written]"
        } else {
            ""
        }
    );
    println!(
        "skipped: {} not-done, {} no-id, {} no-lane (claim-time quirk / unregistered), {} already",
        rep.skipped_not_done, rep.skipped_no_id, rep.skipped_no_lane, rep.skipped_already
    );
}

fn print_collect_bees_json(cards: &Path, lpath: &Path, rep: &BeeReport) {
    println!(
        "{{\"version\":\"{}\",\"cards\":\"{}\",\"ledger\":\"{}\",\"dry_run\":{},\"cards_seen\":{},\"rows\":{},\"passes\":{},\"skipped\":{{\"not_done\":{},\"no_id\":{},\"no_lane\":{},\"already\":{}}}}}",
        VERSION,
        esc(&cards.display().to_string()),
        esc(&lpath.display().to_string()),
        rep.dry_run,
        rep.cards_seen,
        rep.rows,
        rep.passes,
        rep.skipped_not_done,
        rep.skipped_no_id,
        rep.skipped_no_lane,
        rep.skipped_already
    );
}

// --- collect-tinyagi --------------------------------------------------------

fn run_collect_tinyagi(args: &[String]) -> ExitCode {
    let mut ledger: Option<PathBuf> = None;
    let mut tinyagi: Option<PathBuf> = None;
    let mut json = false;
    let mut dry = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--tinyagi" if i + 1 < args.len() => {
                tinyagi = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "--dry-run" => {
                dry = true;
                i += 1;
            }
            _ => match scan_common(args, &mut i) {
                Ok(Flag::Ledger(p)) => ledger = Some(p),
                Ok(Flag::Json) => json = true,
                Err(other) => {
                    eprintln!("caddis-router collect-tinyagi: unknown argument {other:?}");
                    eprintln!("{USAGE}");
                    return ExitCode::from(2);
                }
            },
        }
    }
    let tinyagi = tinyagi.unwrap_or_else(|| {
        std::env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".tinyagi")
    });
    let lpath = ledger.unwrap_or_else(|| default_home().join("ledger.jsonl"));
    match collect_tinyagi(&tinyagi, &Ledger::new(&lpath), dry) {
        Ok(rep) => {
            if json {
                print_collect_tinyagi_json(&tinyagi, &lpath, &rep);
            } else {
                print_collect_tinyagi_human(&tinyagi, &lpath, &rep);
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("caddis-router: collect-tinyagi {}: {e}", tinyagi.display());
            ExitCode::from(2)
        }
    }
}

fn print_collect_tinyagi_human(tinyagi: &Path, lpath: &Path, rep: &TinyagiReport) {
    println!("tinyagi home: {}", tinyagi.display());
    println!("ledger: {}", lpath.display());
    println!(
        "snapshots: {} ({} with roster) provable brackets: {}{}",
        rep.snapshots_seen,
        rep.snapshots_roster,
        rep.brackets_provable,
        if rep.dry_run {
            " [dry-run: nothing written]"
        } else {
            ""
        }
    );
    println!(
        "records: {} runs + {} failed -> rows: {} ({} pass, {} fail)",
        rep.runs_seen, rep.failed_seen, rep.rows, rep.passes, rep.fails
    );
    println!(
        "skipped: {} no-id, {} no-agent, {} no-bracket, {} empty-roster bracket (dark zone), {} no-lane, {} no-outcome, {} bad-line, {} already",
        rep.skipped_no_id,
        rep.skipped_no_agent,
        rep.skipped_no_bracket,
        rep.skipped_empty_roster,
        rep.skipped_no_lane,
        rep.skipped_no_outcome,
        rep.skipped_bad_line,
        rep.skipped_already
    );
}

fn print_collect_tinyagi_json(tinyagi: &Path, lpath: &Path, rep: &TinyagiReport) {
    println!(
        "{{\"version\":\"{}\",\"tinyagi\":\"{}\",\"ledger\":\"{}\",\"dry_run\":{},\"snapshots_seen\":{},\"snapshots_roster\":{},\"brackets_provable\":{},\"runs_seen\":{},\"failed_seen\":{},\"rows\":{},\"passes\":{},\"fails\":{},\"skipped\":{{\"no_id\":{},\"no_agent\":{},\"no_bracket\":{},\"empty_roster\":{},\"no_lane\":{},\"no_outcome\":{},\"bad_line\":{},\"already\":{}}}}}",
        VERSION,
        esc(&tinyagi.display().to_string()),
        esc(&lpath.display().to_string()),
        rep.dry_run,
        rep.snapshots_seen,
        rep.snapshots_roster,
        rep.brackets_provable,
        rep.runs_seen,
        rep.failed_seen,
        rep.rows,
        rep.passes,
        rep.fails,
        rep.skipped_no_id,
        rep.skipped_no_agent,
        rep.skipped_no_bracket,
        rep.skipped_empty_roster,
        rep.skipped_no_lane,
        rep.skipped_no_outcome,
        rep.skipped_bad_line,
        rep.skipped_already
    );
}

// --- shared helpers ----------------------------------------------------------

/// The two-character escaping discipline (shared by both JSON printers).
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// The organ's state home (voice-organ convention: USERPROFILE, no home
/// crate — zero deps is crate law).
fn default_home() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".caddis")
        .join("router")
}
