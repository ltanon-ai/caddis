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
    alerts::Alerts, collect_bees, collect_councils, collect_tinyagi, encode_policy, gate::Gate,
    load_policy, load_registry, profile_from_card, run_scan, verify_path, BeeReport, CapsReport,
    CollectReport, DataClass, LaneRegistry, Ledger, Loaded, RegistryErr, RoutePolicy, ScanReport,
    TinyagiReport, VERSION,
};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "usage:
  caddis-router verify          [--ledger <path>] [--home <dir>] [--json]
  caddis-router collect         [--councils <dir>] [--ledger <path>] [--home <dir>] [--dry-run] [--json]
  caddis-router collect-bees    [--cards <path>] [--ledger <path>] [--home <dir>] [--dry-run] [--json]
  caddis-router collect-tinyagi [--tinyagi <dir>] [--ledger <path>] [--home <dir>] [--dry-run] [--json]
  caddis-router scan            [--ledger <path>|--home <dir>] [--alerts <path>] [--dry-run] [--json]
  caddis-router policy          [--policy <path>|--home <dir>] [--json]
  caddis-router route-gated   --card <path> --data <secret|pii|internal|public> (--alive <ids>|--assume-alive <ids>) [--lanes <path>|--home <dir>] [--policy <path>] [--ledger <path>] [--alerts <path>] [--json]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    }
    match args[0].as_str() {
        "verify" => run_verify(&args[1..]),
        "scan" => run_scan_cmd(&args[1..]),
        "policy" => run_policy(&args[1..]),
        "collect-bees" => run_collect_bees(&args[1..]),
        "collect-tinyagi" => run_collect_tinyagi(&args[1..]),
        "route-gated" => run_route_gated(&args[1..]),
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
            println!("  scan: promote persistent lane decay to ledger rows + alerts (R2/R4)");
            println!(
                "    --alerts <path>   alert stream (default: alerts.jsonl beside the ledger)"
            );
            println!("    --dry-run         report what would land, append nothing");
            println!("  policy: audit the ruling policy the router would obey (READ-ONLY;");
            println!("        exit 1 = malformed file, routing must refuse — fail closed)");
            println!("    --policy <path>   this exact policy file (wins over --home)");
            println!("    --home <dir>      look for <dir>/policy.json (default ~/.caddis/router)");
            println!("    --json            machine report on stdout");
            println!("  route-gated: the SUBPROCESS consumption surface (P4 slice 4) —");
            println!("        route one task card and print the versioned decision JSON");
            println!("        ({{v:1,...}}); the CALLER dispatches (F1: this binary never does).");
            println!("        exit 0 = routed (decision row persisted, seq in stdout);");
            println!("        exit 1 = refused (routing stop, alert persisted — honest halt);");
            println!("        exit 2 = usage/environment defect (missing or malformed card,");
            println!("        registry, policy — never a routing decision; fail closed).");
            println!("    --card <path>      task card: id+class frontmatter, Done-When, RED-TEST");
            println!("    --data <class>     secret|pii|internal|public (F5 vocabulary)");
            println!("    --alive <ids>      comma list of PROBED-alive lane ids (your probe)");
            println!("    --assume-alive <ids>  same set as a NAMED assumption (auditable);");
            println!("        exactly ONE of the two is required — silence is never consent");
            println!("        (council Q3); an id outside lanes.jsonl is a usage stop.");
            println!("    --lanes <path>     lane registry (default <home>/lanes.jsonl —");
            println!("        operator-authored, static-until-ruled; JSONL flat objects:");
            println!("        id | family | tier | cost_per_task_usd, nothing else)");
            println!("    --policy <path>    ruling policy (default <home>/policy.json; absent");
            println!("        = builtin conservative priors, auditable via `policy`)");
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

// --- scan --------------------------------------------------------------------

fn run_scan_cmd(args: &[String]) -> ExitCode {
    let mut ledger: Option<PathBuf> = None;
    let mut alerts: Option<PathBuf> = None;
    let mut json = false;
    let mut dry = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--alerts" if i + 1 < args.len() => {
                alerts = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "--ledger" if i + 1 < args.len() => {
                let p = PathBuf::from(&args[i + 1]);
                alerts = alerts
                    .clone()
                    .or_else(|| p.parent().map(|d| d.join("alerts.jsonl")));
                ledger = Some(p);
                i += 2;
            }
            "--home" if i + 1 < args.len() => {
                let d = PathBuf::from(&args[i + 1]);
                alerts = alerts.clone().or_else(|| Some(d.join("alerts.jsonl")));
                ledger = Some(d.join("ledger.jsonl"));
                i += 2;
            }
            "--dry-run" => {
                dry = true;
                i += 1;
            }
            "--json" => {
                json = true;
                i += 1;
            }
            other => {
                eprintln!("caddis-router scan: unknown argument {other:?}");
                eprintln!("{USAGE}");
                return ExitCode::from(2);
            }
        }
    }
    let lpath = ledger.unwrap_or_else(|| default_home().join("ledger.jsonl"));
    let apath = alerts.unwrap_or_else(|| {
        lpath
            .parent()
            .map(|d| d.join("alerts.jsonl"))
            .unwrap_or_else(|| PathBuf::from("alerts.jsonl"))
    });
    match run_scan(&Ledger::new(&lpath), &Alerts::new(&apath), dry) {
        Ok(rep) => {
            if json {
                print_scan_json(&lpath, &apath, &rep);
            } else {
                print_scan_human(&lpath, &apath, &rep);
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("caddis-router: scan {}: {e}", lpath.display());
            ExitCode::from(2)
        }
    }
}

fn print_scan_human(lpath: &Path, apath: &Path, rep: &ScanReport) {
    println!("ledger: {}", lpath.display());
    println!("alerts: {}", apath.display());
    println!(
        "outcomes: {} transitions: {} recorded: {} appended: {} alerts: {} mismatch: {}{}",
        rep.outcomes_scanned,
        rep.transitions_total,
        rep.promotions_recorded,
        rep.promotions_appended,
        rep.alerts_appended,
        rep.marker_mismatch,
        if rep.dry_run {
            " [dry-run: nothing written]"
        } else {
            ""
        }
    );
}

fn print_scan_json(lpath: &Path, apath: &Path, rep: &ScanReport) {
    println!(
        "{{\"version\":\"{}\",\"ledger\":\"{}\",\"alerts\":\"{}\",\"dry_run\":{},\"outcomes_scanned\":{},\"transitions_total\":{},\"promotions_recorded\":{},\"promotions_appended\":{},\"alerts_appended\":{},\"marker_mismatch\":{}}}",
        VERSION,
        esc(&lpath.display().to_string()),
        esc(&apath.display().to_string()),
        rep.dry_run,
        rep.outcomes_scanned,
        rep.transitions_total,
        rep.promotions_recorded,
        rep.promotions_appended,
        rep.alerts_appended,
        rep.marker_mismatch
    );
}

// --- policy ------------------------------------------------------------------

/// Audit the ruling the router would obey. NEVER writes the file — the
/// policy is authored by the operator/warden path (P5 propose->confirm);
/// the router only obeys, and this command shows exactly WHAT it would
/// obey. Exit 0 = loadable ruling (file or builtin defaults); exit 1 =
/// the file exists and is malformed (one finding — routing must refuse);
/// exit 2 = usage.
fn run_policy(args: &[String]) -> ExitCode {
    let mut path: Option<PathBuf> = None;
    let mut json = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => {
                json = true;
                i += 1;
            }
            "--policy" if i + 1 < args.len() => {
                path = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "--home" if i + 1 < args.len() => {
                path = Some(PathBuf::from(&args[i + 1]).join("policy.json"));
                i += 2;
            }
            other => {
                eprintln!("caddis-router policy: unknown argument {other:?}");
                eprintln!("{USAGE}");
                return ExitCode::from(2);
            }
        }
    }
    let ppath = path.unwrap_or_else(|| default_home().join("policy.json"));
    let present = ppath.exists();
    match load_policy(&ppath) {
        Ok(policy) => {
            if json {
                print_policy_json(&ppath, present, policy.as_ref());
            } else {
                print_policy_human(&ppath, present, policy.as_ref());
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            let finding = match e {
                caddis_router::PolicyFileErr::Read(m) => format!("cannot read: {m}"),
                caddis_router::PolicyFileErr::Malformed(m) => m,
            };
            if json {
                println!(
                    "{{\"version\":\"{}\",\"policy_file\":\"{}\",\"present\":true,\"policy\":null,\"findings\":[\"{}\"]}}",
                    VERSION,
                    esc(&ppath.display().to_string()),
                    esc(&finding)
                );
            } else {
                println!("policy file: {}", ppath.display());
                println!("finding 1: {finding}");
                println!("routing must refuse — fail closed (never fall back to defaults)");
            }
            ExitCode::from(1)
        }
    }
}

fn print_policy_human(ppath: &Path, present: bool, policy: Option<&RoutePolicy>) {
    println!("policy file: {}", ppath.display());
    if present {
        println!("source: file (the whole policy — unruled pieces fail closed)");
    } else {
        println!("source: builtin conservative defaults (no policy file — F5 priors)");
    }
    let resolved = policy.cloned().unwrap_or_default();
    // What the operator audits is what the router obeys: the exact wire
    // form the loader consumes, not a parallel rendering.
    println!("policy: {}", encode_policy(&resolved));
    if resolved.floors().is_empty() {
        println!("note: no floors ruled — every class fails NoFloorForClass");
    }
    if resolved.ceilings().is_empty() {
        println!("note: no cost ceilings ruled — escalation stays closed (R1)");
    }
}

fn print_policy_json(ppath: &Path, present: bool, policy: Option<&RoutePolicy>) {
    let resolved = policy.cloned().unwrap_or_default();
    println!(
        "{{\"version\":\"{}\",\"policy_file\":\"{}\",\"present\":{},\"source\":\"{}\",\"policy\":{},\"floors\":{},\"ceilings\":{}}}",
        VERSION,
        esc(&ppath.display().to_string()),
        present,
        if present { "file" } else { "defaults" },
        encode_policy(&resolved),
        resolved.floors().len(),
        resolved.ceilings().len()
    );
}

// --- route-gated --------------------------------------------------------------

/// The subprocess consumption surface (P4 slice 4, council-folded):
/// load the organ home's inputs (lane registry + policy + ledger fold),
/// route ONE task card through the gate, print the versioned decision.
/// Exit 0 = routed (the decision row IS in the ledger — the caller may
/// dispatch on it); 1 = refused (a routing stop, its alert persisted —
/// an honest halt, not a defect); 2 = usage or environment defect (bad
/// argv, unreadable/malformed card, registry, or policy — never a routing
/// decision; fail closed). The binary NEVER dispatches (F1): the caller
/// consumes {v:1, lane_id, seq} and dispatches itself. Liveness is the
/// CALLER's declaration — `--alive` (probed) or `--assume-alive` (named
/// assumption), exactly one; silence is never consent (council Q3).
fn run_route_gated(args: &[String]) -> ExitCode {
    let mut card_path: Option<PathBuf> = None;
    let mut data_word: Option<String> = None;
    let mut alive_arg: Option<String> = None;
    let mut assume_arg: Option<String> = None;
    let mut lanes_path: Option<PathBuf> = None;
    let mut policy_path: Option<PathBuf> = None;
    let mut ledger_path: Option<PathBuf> = None;
    let mut alerts_path: Option<PathBuf> = None;
    let mut json = false;
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => {
                json = true;
                i += 1;
            }
            "--card" if i + 1 < args.len() => {
                card_path = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "--data" if i + 1 < args.len() => {
                data_word = Some(args[i + 1].clone());
                i += 2;
            }
            "--alive" if i + 1 < args.len() => {
                alive_arg = Some(args[i + 1].clone());
                i += 2;
            }
            "--assume-alive" if i + 1 < args.len() => {
                assume_arg = Some(args[i + 1].clone());
                i += 2;
            }
            "--lanes" if i + 1 < args.len() => {
                lanes_path = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "--policy" if i + 1 < args.len() => {
                policy_path = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "--alerts" if i + 1 < args.len() => {
                alerts_path = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "--ledger" if i + 1 < args.len() => {
                ledger_path = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "--home" if i + 1 < args.len() => {
                let h = PathBuf::from(&args[i + 1]);
                if ledger_path.is_none() {
                    ledger_path = Some(h.join("ledger.jsonl"));
                }
                if lanes_path.is_none() {
                    lanes_path = Some(h.join("lanes.jsonl"));
                }
                if policy_path.is_none() {
                    policy_path = Some(h.join("policy.json"));
                }
                if alerts_path.is_none() {
                    alerts_path = Some(h.join("alerts.jsonl"));
                }
                i += 2;
            }
            other => {
                eprintln!("caddis-router route-gated: unknown argument {other:?}");
                eprintln!("{USAGE}");
                return ExitCode::from(2);
            }
        }
    }

    // Q3 law: exactly ONE liveness declaration. Both at once is ambiguous
    // provenance; neither is silence-as-consent.
    let (alive_list, assumed) = match (alive_arg, assume_arg) {
        (Some(_), Some(_)) => {
            eprintln!(
                "caddis-router route-gated: --alive and --assume-alive are mutually exclusive (pick one liveness provenance)"
            );
            return ExitCode::from(2);
        }
        (Some(ids), None) => (ids, false),
        (None, Some(ids)) => (ids, true),
        (None, None) => {
            eprintln!(
                "caddis-router route-gated: exactly one of --alive <ids> | --assume-alive <ids> is required (council Q3: silence is never consent)"
            );
            return ExitCode::from(2);
        }
    };

    let Some(card_path) = card_path else {
        eprintln!("caddis-router route-gated: --card <path> is required");
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let Some(data_word) = data_word else {
        eprintln!("caddis-router route-gated: --data <secret|pii|internal|public> is required");
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let Some(data_class) = DataClass::parse(&data_word) else {
        eprintln!(
            "caddis-router route-gated: unknown data class {data_word:?} (vocabulary: secret|pii|internal|public)"
        );
        return ExitCode::from(2);
    };

    let home = default_home();
    let ledger_path = ledger_path.unwrap_or_else(|| home.join("ledger.jsonl"));
    let lanes_path = lanes_path.unwrap_or_else(|| home.join("lanes.jsonl"));
    let policy_path = policy_path.unwrap_or_else(|| home.join("policy.json"));
    let alerts_path = alerts_path.unwrap_or_else(|| {
        ledger_path
            .parent()
            .unwrap_or(Path::new("."))
            .join("alerts.jsonl")
    });

    // Card -> profile. The F3 read surface is minimal; a card that does
    // not parse (or lacks the routing sections) is a construction defect.
    let card_text = match std::fs::read_to_string(&card_path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!(
                "caddis-router route-gated: cannot read card {}: {e}",
                card_path.display()
            );
            return ExitCode::from(2);
        }
    };
    let card = match caddis_card::Card::parse(&card_text) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("caddis-router route-gated: card does not parse: {e:?}");
            return ExitCode::from(2);
        }
    };
    let profile = match profile_from_card(&card) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("caddis-router route-gated: card lacks the routing surface: {e:?}");
            return ExitCode::from(2);
        }
    };

    // Policy: absent = builtin conservative priors (auditable via
    // `policy`); present-but-malformed = refuse (fail closed past an
    // authored file).
    let policy = match load_policy(&policy_path) {
        Ok(Some(p)) => p,
        Ok(None) => RoutePolicy::default(),
        Err(e) => {
            eprintln!(
                "caddis-router route-gated: policy {}: {:?}",
                policy_path.display(),
                e
            );
            return ExitCode::from(2);
        }
    };

    // Registry: absent = the operator has not ruled lanes yet — fail
    // closed with the exact message; malformed likewise. Never a routing
    // decision, never an empty-universe guess.
    let registry: LaneRegistry = match load_registry(&lanes_path) {
        Ok(Some(r)) => r,
        Ok(None) => {
            eprintln!(
                "caddis-router route-gated: no lane registry at {} — lanes.jsonl is the operator-authored ruling home (static-until-ruled; `caddis-router policy` shows the law analog)",
                lanes_path.display()
            );
            return ExitCode::from(2);
        }
        Err(RegistryErr::Read(m)) | Err(RegistryErr::Malformed(m)) => {
            eprintln!(
                "caddis-router route-gated: registry {}: {}",
                lanes_path.display(),
                m
            );
            return ExitCode::from(2);
        }
    };

    // Caller liveness set. The authored universe is the routing universe:
    // an id outside it is a caller typo, stopped here — never routed.
    let mut alive = std::collections::BTreeSet::new();
    let flag_name = if assumed { "assume-alive" } else { "alive" };
    for id in alive_list.split(',') {
        let id = id.trim();
        if id.is_empty() {
            eprintln!("caddis-router route-gated: empty lane id in --{flag_name} list");
            return ExitCode::from(2);
        }
        if !registry.knows(id) {
            eprintln!(
                "caddis-router route-gated: lane {id:?} is not in the registry — fix the registry ruling or the caller list"
            );
            return ExitCode::from(2);
        }
        alive.insert(id.to_string());
    }

    // Capability fold. A missing ledger is an honest empty history (the
    // append below materializes the file); an unreadable existing one is
    // a defect.
    let ledger = Ledger::new(&ledger_path);
    let loaded = if ledger_path.exists() {
        match ledger.load() {
            Ok(l) => l,
            Err(e) => {
                eprintln!(
                    "caddis-router route-gated: ledger {}: {e}",
                    ledger_path.display()
                );
                return ExitCode::from(2);
            }
        }
    } else {
        Loaded::default()
    };
    let caps = CapsReport::from_rows(&loaded);
    let lanes = registry.lanes(&caps, &alive);

    let alerts = Alerts::new(&alerts_path);
    let gate = Gate::new(&ledger, &alerts);
    match gate.route_gated(&profile, data_class, &lanes, &policy) {
        Ok((d, seq)) => {
            let liveness = if assumed { "assumed" } else { "probed" };
            if json {
                println!(
                    "{{\"v\":1,\"status\":\"routed\",\"route_id\":\"{}\",\"card_id\":\"{}\",\"task_class\":\"{}\",\"lane_id\":\"{}\",\"lane_tier\":\"{}\",\"cost_per_task_usd\":{},\"degraded\":{},\"seq\":{},\"liveness\":\"{}\"}}",
                    esc(&d.route_id),
                    esc(&d.card_id),
                    esc(&d.task_class),
                    esc(&d.lane_id),
                    d.lane_tier.as_str(),
                    d.cost_per_task_usd,
                    d.degraded,
                    seq,
                    liveness
                );
            } else {
                println!(
                    "routed {} ({}) -> {} [{}] ${:.4} seq {}{} (liveness: {liveness})",
                    d.card_id,
                    d.task_class,
                    d.lane_id,
                    d.lane_tier.as_str(),
                    d.cost_per_task_usd,
                    seq,
                    if d.degraded { " DEGRADED" } else { "" }
                );
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            if json {
                println!(
                    "{{\"v\":1,\"status\":\"refused\",\"error\":\"{}\"}}",
                    esc(&e.to_string())
                );
            } else {
                println!("refused: {e}");
            }
            ExitCode::from(1)
        }
    }
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
