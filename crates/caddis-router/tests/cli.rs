//! CLI e2e — the REAL binary against REAL files (P2 remainder law: a bin is
//! verified by running it, not by calling its functions). Every fixture line
//! below is the exact wire format `ledger::encode` emits, hand-written so a
//! fork/gap is a deliberate ACT OF MAN, exactly the corruption the verifier
//! exists to catch.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_caddis-router");

fn tmpdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("rtr-cli-{}-{}", tag, std::process::id()));
    fs::create_dir_all(&d).unwrap();
    d
}

fn run(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(BIN).args(args).output().unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

const DECISION_1: &str = "{\"seq\":1,\"ts\":\"2026-08-28T00:00:00Z\",\"kind\":\"decision\",\"route_id\":\"r-1\",\"card_id\":\"CARD-1\",\"task_class\":\"coding\",\"lane_id\":\"groq-free\",\"tier\":\"free\",\"cost_per_task_usd\":0,\"degraded\":false}";
const OUTCOME_2: &str = "{\"seq\":2,\"ts\":\"2026-08-28T00:00:01Z\",\"kind\":\"outcome\",\"card_id\":\"CARD-1\",\"task_class\":\"coding\",\"lane_id\":\"groq-free\",\"model\":\"gpt-oss-120b\",\"cost_tokens\":1200,\"cost_usd_est\":0.0042,\"latency_ms\":8500,\"verify_outcome\":\"fail\",\"escalated_to\":\"gemini-2.5-pro\"}";
const OUTCOME_3: &str = "{\"seq\":3,\"ts\":\"2026-08-28T00:00:02Z\",\"kind\":\"outcome\",\"card_id\":\"CARD-2\",\"task_class\":\"coding\",\"lane_id\":\"gemini-2.5-pro\",\"model\":\"gemini-2.5-pro\",\"cost_tokens\":900,\"cost_usd_est\":0.011,\"latency_ms\":6200,\"verify_outcome\":\"pass\",\"escalated_to\":null}";

fn write(tag: &str, name: &str, body: &str) -> PathBuf {
    let d = tmpdir(tag);
    let p = d.join(name);
    fs::write(&p, body).unwrap();
    p
}

#[test]
fn missing_ledger_is_clean_and_says_so() {
    let p = tmpdir("missing").join("none.jsonl");
    let (rc, out, err) = run(&["verify", "--ledger", p.to_str().unwrap()]);
    assert_eq!(rc, 0, "first run has no findings: {err}");
    assert!(
        out.contains("missing — no decisions recorded yet"),
        "honest first-run marker: {out}"
    );
    // JSON mode agrees and stays machine-parseable flat JSON.
    let (rc, out, _) = run(&["verify", "--ledger", p.to_str().unwrap(), "--json"]);
    assert_eq!(rc, 0);
    assert!(out.contains("\"exists\":false"), "{out}");
    assert!(out.contains("\"rc\":0"), "{out}");
    fs::remove_dir_all(p.parent().unwrap()).ok();
}

#[test]
fn clean_ledger_zero_findings() {
    let p = write(
        "clean",
        "ledger.jsonl",
        &format!("{DECISION_1}\n{OUTCOME_2}\n{OUTCOME_3}\n"),
    );
    let (rc, out, err) = run(&["verify", "--ledger", p.to_str().unwrap()]);
    assert_eq!(rc, 0, "{err}");
    assert!(out.contains("lines: 3"), "{out}");
    assert!(out.contains("rows_ok: 3"), "{out}");
    assert!(out.contains("findings: 0"), "{out}");
    fs::remove_dir_all(p.parent().unwrap()).ok();
}

#[test]
fn fork_and_gap_reported_with_count_exit() {
    // A hand-forked ledger: seq 2 repeated (the fork signature), then a
    // jump to 5. Two findings, exit code = 2.
    let forked = format!(
        "{DECISION_1}\n{OUTCOME_2}\n{OUTCOME_3}\n{{\"seq\":5,\"ts\":\"2026-08-28T00:00:09Z\",\"kind\":\"outcome\",\"card_id\":\"CARD-9\",\"task_class\":\"coding\",\"lane_id\":\"x\",\"model\":\"m\",\"cost_tokens\":1,\"cost_usd_est\":0.001,\"latency_ms\":10,\"verify_outcome\":\"pass\",\"escalated_to\":null}}\n"
    );
    // rewrite line 3 with seq 2: the classic duplicate-seq fork
    let forked = forked.replacen("{\"seq\":3,", "{\"seq\":2,", 1);
    let p = write("fork", "ledger.jsonl", &forked);
    let (rc, out, err) = run(&["verify", "--ledger", p.to_str().unwrap()]);
    assert_eq!(rc, 2, "exit = finding count: {err}");
    assert!(out.contains("seq-dup"), "{out}");
    assert!(out.contains("seq-gap"), "{out}");
    let (rc, out, _) = run(&["verify", "--ledger", p.to_str().unwrap(), "--json"]);
    assert_eq!(rc, 2);
    assert!(out.contains("\"rc\":2"), "{out}");
    assert!(out.contains("\"exists\":true"), "{out}");
    fs::remove_dir_all(p.parent().unwrap()).ok();
}

#[test]
fn bad_line_is_a_finding_not_a_crash() {
    // The torn middle row leaves TWO honest findings: itself (bad-line)
    // and the seq jump 1->3 it tore open (seq-gap).
    let p = write(
        "bad",
        "ledger.jsonl",
        &format!("{DECISION_1}\nnot json at all\n{OUTCOME_3}\n"),
    );
    let (rc, out, err) = run(&["verify", "--ledger", p.to_str().unwrap()]);
    assert_eq!(rc, 2, "{err}");
    assert!(out.contains("bad-line"), "{out}");
    assert!(out.contains("seq-gap"), "{out}");
    fs::remove_dir_all(p.parent().unwrap()).ok();
}

#[test]
fn home_override_reads_home_ledger_jsonl() {
    let d = tmpdir("home");
    fs::write(d.join("ledger.jsonl"), format!("{DECISION_1}\n")).unwrap();
    let (rc, out, err) = run(&["verify", "--home", d.to_str().unwrap()]);
    assert_eq!(rc, 0, "{err}");
    let shown = d.join("ledger.jsonl").display().to_string();
    assert!(out.contains(&shown), "{out}");
    assert!(out.contains("rows_ok: 1"), "{out}");
    fs::remove_dir_all(&d).ok();
}

#[test]
fn unknown_argument_fails_closed() {
    let (rc, _, err) = run(&["verify", "--nonsense"]);
    assert_eq!(rc, 2);
    assert!(err.contains("unknown argument"), "{err}");
    let (rc, _, _) = run(&[]);
    assert_eq!(rc, 2);
    let (rc, out, _) = run(&["--version"]);
    assert_eq!(rc, 0);
    assert!(out.contains("caddis-router"), "{out}");
}

// --- collect e2e -------------------------------------------------------------

fn councils_fixture(tag: &str) -> PathBuf {
    let root = tmpdir(tag).join("councils");
    let mk = |name: &str, manifest: Option<&str>, verdicts: Option<&str>| {
        let d = root.join(name);
        fs::create_dir_all(&d).unwrap();
        if let Some(m) = manifest {
            fs::write(d.join("MANIFEST.json"), m).unwrap();
        }
        if let Some(v) = verdicts {
            fs::write(d.join("VERDICTS.json"), v).unwrap();
        }
    };
    mk(
        "20260801-010101-alpha",
        Some(
            r#"{"skeptic": {"provider": "grok-coding", "model": "grok-4.6"}, "chair": {"provider": "gemini", "model": "gemini-2.5-flash"}}"#,
        ),
        Some(
            r#"{"skeptic": {"stance": "mixed", "verdict": "sound but two fixes needed"}, "chair": {"stance": "approve", "verdict": "sound"}}"#,
        ),
    );
    mk("20260801-020202-noid", None, None); // no transport record
    root
}

#[test]
fn collect_appends_then_is_idempotent() {
    let root = councils_fixture("capi");
    let ledger = root.parent().unwrap().join("ledger.jsonl");
    let l = ledger.to_str().unwrap();
    let (rc, out, err) = run(&[
        "collect",
        "--councils",
        root.to_str().unwrap(),
        "--ledger",
        l,
    ]);
    assert_eq!(rc, 0, "{err}");
    assert!(out.contains("consults: 2"), "{out}");
    assert!(out.contains("rows: 2 (pass 2 / fail 0)"), "{out}");
    assert!(out.contains("1 no-manifest"), "{out}");

    // the appended stream verifies clean through the OTHER subcommand
    let (rc, out, err) = run(&["verify", "--ledger", l]);
    assert_eq!(rc, 0, "{err}");
    assert!(out.contains("rows_ok: 2"), "{out}");

    // re-run: everything already there, nothing new
    let (rc, out, err) = run(&[
        "collect",
        "--councils",
        root.to_str().unwrap(),
        "--ledger",
        l,
    ]);
    assert_eq!(rc, 0, "{err}");
    assert!(out.contains("rows: 0"), "{out}");
    assert!(out.contains("2 already"), "{out}");
    fs::remove_dir_all(root.parent().unwrap()).ok();
}

#[test]
fn collect_dry_run_writes_nothing_and_json_reports() {
    let root = councils_fixture("cdry");
    let ledger = root.parent().unwrap().join("ledger.jsonl");
    let (rc, out, err) = run(&[
        "collect",
        "--councils",
        root.to_str().unwrap(),
        "--ledger",
        ledger.to_str().unwrap(),
        "--dry-run",
        "--json",
    ]);
    assert_eq!(rc, 0, "{err}");
    assert!(out.contains("\"rows\":2"), "{out}");
    assert!(out.contains("\"dry_run\":true"), "{out}");
    assert!(!ledger.exists(), "dry-run must not materialize a ledger");
    fs::remove_dir_all(root.parent().unwrap()).ok();
}

#[test]
fn collect_unknown_argument_fails_closed() {
    let (rc, _, err) = run(&["collect", "--nonsense"]);
    assert_eq!(rc, 2);
    assert!(err.contains("unknown argument"), "{err}");
}

// --- scan e2e ------------------------------------------------------------------

const SCAN_FAIL_1: &str = "{\"seq\":1,\"ts\":\"2026-08-28T00:00:00Z\",\"kind\":\"outcome\",\"card_id\":\"CARD-9\",\"task_class\":\"coding\",\"lane_id\":\"groq-free\",\"model\":\"gpt-oss-120b\",\"cost_tokens\":100,\"cost_usd_est\":0.001,\"latency_ms\":900,\"verify_outcome\":\"fail\",\"escalated_to\":null}";
const SCAN_FAIL_2: &str = "{\"seq\":2,\"ts\":\"2026-08-28T00:00:01Z\",\"kind\":\"outcome\",\"card_id\":\"CARD-10\",\"task_class\":\"coding\",\"lane_id\":\"groq-free\",\"model\":\"gpt-oss-120b\",\"cost_tokens\":100,\"cost_usd_est\":0.001,\"latency_ms\":900,\"verify_outcome\":\"fail\",\"escalated_to\":null}";
const SCAN_PASS_3: &str = "{\"seq\":3,\"ts\":\"2026-08-28T00:00:02Z\",\"kind\":\"outcome\",\"card_id\":\"CARD-11\",\"task_class\":\"coding\",\"lane_id\":\"groq-free\",\"model\":\"gpt-oss-120b\",\"cost_tokens\":100,\"cost_usd_est\":0.001,\"latency_ms\":900,\"verify_outcome\":\"pass\",\"escalated_to\":null}";

#[test]
fn scan_appends_promotion_and_alert_then_is_idempotent() {
    let ledger = write(
        "scan1",
        "ledger.jsonl",
        &format!("{SCAN_FAIL_1}\n{SCAN_FAIL_2}\n"),
    );
    let l = ledger.to_str().unwrap();
    let (rc, out, err) = run(&["scan", "--ledger", l]);
    assert_eq!(rc, 0, "{err}");
    assert!(
        out.contains("transitions: 1 recorded: 0 appended: 1 alerts: 1 mismatch: 0"),
        "{out}"
    );
    // the alert stream lands beside the ledger with a degraded row
    let alerts = ledger.parent().unwrap().join("alerts.jsonl");
    let body = fs::read_to_string(&alerts).unwrap();
    assert_eq!(body.lines().count(), 1);
    assert!(body.contains("\"kind\":\"degraded\""), "{body}");
    assert!(body.contains("\"lane_id\":\"groq-free\""), "{body}");
    // the ledger gained a promotion row and still verifies clean
    let (rc, out, err) = run(&["verify", "--ledger", l]);
    assert_eq!(rc, 0, "{err}");
    assert!(out.contains("rows_ok: 3"), "{out}");
    // re-scan: prefix complete, nothing new (idempotent)
    let (rc, out, err) = run(&["scan", "--ledger", l, "--json"]);
    assert_eq!(rc, 0, "{err}");
    assert!(out.contains("\"promotions_appended\":0"), "{out}");
    assert_eq!(fs::read_to_string(&alerts).unwrap().lines().count(), 1);
    fs::remove_dir_all(ledger.parent().unwrap()).ok();
}

#[test]
fn scan_follows_heal_with_second_promotion() {
    let ledger = write(
        "scan2",
        "ledger.jsonl",
        &format!("{SCAN_FAIL_1}\n{SCAN_FAIL_2}\n{SCAN_PASS_3}\n"),
    );
    let l = ledger.to_str().unwrap();
    let (rc, out, err) = run(&["scan", "--ledger", l]);
    assert_eq!(rc, 0, "{err}");
    assert!(out.contains("appended: 2 alerts: 2"), "{out}");
    let body = fs::read_to_string(ledger.parent().unwrap().join("alerts.jsonl")).unwrap();
    assert!(body.contains("\"kind\":\"degraded\""), "{body}");
    assert!(body.contains("\"kind\":\"healed\""), "{body}");
    fs::remove_dir_all(ledger.parent().unwrap()).ok();
}

#[test]
fn scan_dry_run_writes_nothing_and_home_override() {
    let home = tmpdir("scanhome");
    fs::write(
        home.join("ledger.jsonl"),
        format!("{SCAN_FAIL_1}\n{SCAN_FAIL_2}\n"),
    )
    .unwrap();
    let (rc, out, err) = run(&[
        "scan",
        "--home",
        home.to_str().unwrap(),
        "--dry-run",
        "--json",
    ]);
    assert_eq!(rc, 0, "{err}");
    assert!(out.contains("\"promotions_appended\":1"), "{out}");
    assert!(out.contains("\"dry_run\":true"), "{out}");
    assert!(
        !home.join("alerts.jsonl").exists(),
        "dry run writes no alerts"
    );
    let body = fs::read_to_string(home.join("ledger.jsonl")).unwrap();
    assert_eq!(body.lines().count(), 2, "dry run appends no promotions");
    fs::remove_dir_all(home).ok();
}

#[test]
fn scan_unknown_argument_fails_closed() {
    let (rc, _, err) = run(&["scan", "--nonsense"]);
    assert_eq!(rc, 2);
    assert!(err.contains("unknown argument"), "{err}");
}

// --- policy e2e ----------------------------------------------------------------

#[test]
fn policy_defaults_when_no_file() {
    let home = tmpdir("pol-defaults");
    let (rc, out, err) = run(&["policy", "--home", home.to_str().unwrap()]);
    assert_eq!(rc, 0, "{out}{err}");
    assert!(out.contains("builtin conservative defaults"), "{out}");
    // The audit shows the EXACT wire form the loader consumes.
    assert!(out.contains("\"floor.skeptic\":0.85"), "{out}");
    assert!(out.contains("\"tier.secret\":\"local\""), "{out}");
    fs::remove_dir_all(home).ok();
}

#[test]
fn policy_loads_authored_file_and_shows_its_whole_ruling() {
    let file = write(
        "pol-file",
        "policy.json",
        "{\"tier.secret\":\"local\",\"tier.internal\":\"local,free\",\"floor.skeptic\":0.9,\"ceiling.coding\":1.5,\"min_samples\":6}",
    );
    let (rc, out, err) = run(&["policy", "--policy", file.to_str().unwrap()]);
    assert_eq!(rc, 0, "{out}{err}");
    assert!(out.contains("source: file"), "{out}");
    assert!(out.contains("\"floor.skeptic\":0.9"), "{out}");
    assert!(out.contains("\"ceiling.coding\":1.5"), "{out}");
    assert!(out.contains("\"min_samples\":6"), "{out}");
    // The file is the WHOLE policy: defaults must NOT leak into the audit.
    assert!(!out.contains("\"floor.chair\""), "{out}");
    assert!(!out.contains("\"tier.public\""), "{out}");
    fs::remove_dir_all(file.parent().unwrap()).ok();
}

#[test]
fn policy_malformed_file_is_one_finding_exit_one() {
    let file = write("pol-bad", "policy.json", "{\"floors.skeptic\":0.9}");
    let (rc, out, _) = run(&["policy", "--policy", file.to_str().unwrap()]);
    assert_eq!(rc, 1, "exit = finding count");
    assert!(out.contains("finding 1"), "{out}");
    assert!(out.contains("unknown field"), "{out}");
    assert!(out.contains("fail closed"), "{out}");
    fs::remove_dir_all(file.parent().unwrap()).ok();
}

#[test]
fn policy_json_report_parses_and_flags_absence() {
    let home = tmpdir("pol-json");
    let (rc, out, _) = run(&["policy", "--home", home.to_str().unwrap(), "--json"]);
    assert_eq!(rc, 0);
    assert!(
        out.starts_with("{") && out.trim_end().ends_with("}"),
        "{out}"
    );
    assert!(out.contains("\"present\":false"), "{out}");
    assert!(out.contains("\"source\":\"defaults\""), "{out}");
    fs::remove_dir_all(home).ok();
}

#[test]
fn policy_unknown_argument_fails_closed() {
    let (rc, _, err) = run(&["policy", "--nonsense"]);
    assert_eq!(rc, 2);
    assert!(err.contains("unknown argument"), "{err}");
}

// --- route-gated e2e (P4 slice 4) ---------------------------------------------

/// One seed outcome row per line, seq 1..5 — groq-free PASSED class
/// "chair" five times (EWMA 1.0, samples 5 = the F2 holdout cleared;
/// default floor chair 0.70).
fn chair_pass_seeds() -> String {
    (1..=5)
        .map(|n| {
            format!(
                "{{\"seq\":{n},\"ts\":\"2026-08-28T00:00:0{n}Z\",\"kind\":\"outcome\",\"card_id\":\"SEED-{n}\",\"task_class\":\"chair\",\"lane_id\":\"groq-free\",\"model\":\"gpt-oss-120b\",\"cost_tokens\":900,\"cost_usd_est\":0.011,\"latency_ms\":6200,\"verify_outcome\":\"pass\",\"escalated_to\":null}}"
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

const LANES_FILE: &str = concat!(
    "{\"id\":\"gemini-mid\",\"family\":\"google\",\"tier\":\"mid\",\"cost_per_task_usd\":0.011}\n",
    "{\"id\":\"groq-free\",\"family\":\"openai-compat\",\"tier\":\"free\",\"cost_per_task_usd\":0}\n",
);

const CHAIR_CARD: &str = "---\nid: CARD-7\nclass: chair\nowner: loop\n---\n# Done-When\n\nThe tray feed renders.\n\n# RED-TEST\n\ne2e-tray 17/17 green.\n";

const SKEPTIC_CARD: &str = "---\nid: CARD-8\nclass: skeptic\nowner: loop\n---\n# Done-When\n\nThe claim survives review.\n\n# RED-TEST\n\nreview-findings 0 high.\n";

fn rg_home(tag: &str, card_class: &str) -> PathBuf {
    let home = tmpdir(tag);
    fs::write(home.join("lanes.jsonl"), LANES_FILE).unwrap();
    fs::write(home.join("ledger.jsonl"), chair_pass_seeds()).unwrap();
    fs::write(
        home.join("card.md"),
        if card_class == "skeptic" {
            SKEPTIC_CARD
        } else {
            CHAIR_CARD
        },
    )
    .unwrap();
    home
}

#[test]
fn route_gated_routes_persists_row_and_prints_contract() {
    let home = rg_home("rg-ok", "chair");
    let (rc, out, err) = run(&[
        "route-gated",
        "--card",
        home.join("card.md").to_str().unwrap(),
        "--data",
        "public",
        "--alive",
        "groq-free,gemini-mid",
        "--home",
        home.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(rc, 0, "{out}{err}");
    // Versioned contract, cheapest lane (O3: groq-free $0 < gemini $0.011),
    // liveness provenance named, seq continues the seed stream.
    assert!(out.contains("\"v\":1"), "{out}");
    assert!(out.contains("\"status\":\"routed\""), "{out}");
    assert!(out.contains("\"lane_id\":\"groq-free\""), "{out}");
    assert!(out.contains("\"liveness\":\"probed\""), "{out}");
    assert!(out.contains("\"seq\":6"), "{out}");
    // F3: the decision row IS in the ledger.
    let body = fs::read_to_string(home.join("ledger.jsonl")).unwrap();
    let last = body.lines().last().unwrap();
    assert!(last.contains("\"kind\":\"decision\""), "{last}");
    assert!(last.contains("\"card_id\":\"CARD-7\""), "{last}");
    fs::remove_dir_all(home).ok();
}

#[test]
fn route_gated_assume_alive_is_a_named_assumption() {
    let home = rg_home("rg-assume", "chair");
    let (rc, out, err) = run(&[
        "route-gated",
        "--card",
        home.join("card.md").to_str().unwrap(),
        "--data",
        "public",
        "--assume-alive",
        "groq-free",
        "--home",
        home.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(rc, 0, "{out}{err}");
    assert!(out.contains("\"liveness\":\"assumed\""), "{out}");
    fs::remove_dir_all(home).ok();
}

#[test]
fn route_gated_no_registry_fails_closed() {
    let home = tmpdir("rg-noreg");
    fs::write(home.join("card.md"), CHAIR_CARD).unwrap();
    let (rc, _, err) = run(&[
        "route-gated",
        "--card",
        home.join("card.md").to_str().unwrap(),
        "--data",
        "public",
        "--alive",
        "groq-free",
        "--home",
        home.to_str().unwrap(),
    ]);
    assert_eq!(rc, 2);
    assert!(err.contains("no lane registry"), "{err}");
    fs::remove_dir_all(home).ok();
}

#[test]
fn route_gated_malformed_registry_refused_droid() {
    let home = tmpdir("rg-droid");
    fs::write(home.join("card.md"), CHAIR_CARD).unwrap();
    fs::write(
        home.join("lanes.jsonl"),
        "{\"id\":\"x\",\"family\":\"f\",\"tier\":\"droid\",\"cost_per_task_usd\":0}\n",
    )
    .unwrap();
    let (rc, _, err) = run(&[
        "route-gated",
        "--card",
        home.join("card.md").to_str().unwrap(),
        "--data",
        "public",
        "--alive",
        "x",
        "--home",
        home.to_str().unwrap(),
    ]);
    assert_eq!(rc, 2);
    assert!(err.contains("droid is refused"), "{err}");
    fs::remove_dir_all(home).ok();
}

#[test]
fn route_gated_liveness_flag_law() {
    let home = rg_home("rg-flags", "chair");
    let card = home.join("card.md").to_str().unwrap().to_string();
    let h = home.to_str().unwrap().to_string();
    // Neither: silence is never consent.
    let (rc, _, err) = run(&[
        "route-gated",
        "--card",
        &card,
        "--data",
        "public",
        "--home",
        &h,
    ]);
    assert_eq!(rc, 2);
    assert!(err.contains("silence is never consent"), "{err}");
    // Both: ambiguous provenance.
    let (rc, _, err) = run(&[
        "route-gated",
        "--card",
        &card,
        "--data",
        "public",
        "--alive",
        "groq-free",
        "--assume-alive",
        "groq-free",
        "--home",
        &h,
    ]);
    assert_eq!(rc, 2);
    assert!(err.contains("mutually exclusive"), "{err}");
    fs::remove_dir_all(home).ok();
}

#[test]
fn route_gated_alive_id_outside_registry_is_a_usage_stop() {
    let home = rg_home("rg-typo", "chair");
    let (rc, _, err) = run(&[
        "route-gated",
        "--card",
        home.join("card.md").to_str().unwrap(),
        "--data",
        "public",
        "--alive",
        "grqq-free",
        "--home",
        home.to_str().unwrap(),
    ]);
    assert_eq!(rc, 2);
    assert!(err.contains("not in the registry"), "{err}");
    fs::remove_dir_all(home).ok();
}

#[test]
fn route_gated_refusal_persists_alert_no_row_exit_one() {
    let home = rg_home("rg-refuse", "skeptic");
    let before = fs::read_to_string(home.join("ledger.jsonl")).unwrap();
    let (rc, out, err) = run(&[
        "route-gated",
        "--card",
        home.join("card.md").to_str().unwrap(),
        "--data",
        "public",
        "--alive",
        "groq-free",
        "--home",
        home.to_str().unwrap(),
        "--json",
    ]);
    // groq-free is measured for chair ONLY — class skeptic has no measured
    // lane: NoMeasuredLane, an honest routing stop.
    assert_eq!(rc, 1, "{out}{err}");
    assert!(out.contains("\"status\":\"refused\""), "{out}");
    assert!(out.contains("NoMeasuredLane"), "{out}");
    // The stop is LOUD: an alert row exists; the ledger gained NO decision.
    let alerts = fs::read_to_string(home.join("alerts.jsonl")).unwrap();
    assert!(alerts.contains("skeptic"), "{alerts}");
    let after = fs::read_to_string(home.join("ledger.jsonl")).unwrap();
    assert_eq!(before, after, "a refused routing must not write a row");
    fs::remove_dir_all(home).ok();
}

// --- warden e2e (R5) ------------------------------------------------------------

#[test]
fn warden_mint_signs_route_gated_rows_and_catches_tampering() {
    let home = rg_home("w-sign", "chair");
    // Mint once, activated at the CURRENT max seq: the 5 seed rows stay
    // honestly unsigned, everything the organ appends from now is signed.
    let (rc, out, err) = run(&["warden", "mint", "--home", home.to_str().unwrap()]);
    assert_eq!(rc, 0, "{out}{err}");
    assert!(out.contains("activated_seq 5"), "{out}");
    assert!(out.contains("fingerprint "), "{out}");
    // A key is born once — the second mint refuses, never overwrites.
    let (rc, _, err) = run(&["warden", "mint", "--home", home.to_str().unwrap()]);
    assert_eq!(rc, 2);
    assert!(err.contains("refusing"), "{err}");

    // route-gated success appends a SIGNED decision row (seq 6).
    let (rc, out, err) = run(&[
        "route-gated",
        "--card",
        home.join("card.md").to_str().unwrap(),
        "--data",
        "public",
        "--alive",
        "groq-free,gemini-mid",
        "--home",
        home.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(rc, 0, "{out}{err}");
    let body = fs::read_to_string(home.join("ledger.jsonl")).unwrap();
    let last = body.lines().last().unwrap();
    assert!(last.contains("\"kind\":\"decision\""), "{last}");
    assert!(last.contains("\"sig\":\""), "{last}");

    // verify: 0 findings — 1 signed, 5 honestly-unsigned pre-activation.
    let (rc, out, err) = run(&["verify", "--home", home.to_str().unwrap()]);
    assert_eq!(rc, 0, "{out}{err}");
    assert!(out.contains("warden: key "), "{out}");
    assert!(out.contains("sig: signed 1 unsigned 5"), "{out}");

    // status agrees, machine form included.
    let (rc, out, err) = run(&[
        "warden",
        "status",
        "--home",
        home.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(rc, 0, "{err}");
    assert!(out.contains("\"state\":\"key\""), "{out}");
    assert!(out.contains("\"signed\":1"), "{out}");
    assert!(out.contains("\"unsigned\":5"), "{out}");

    // The R5 attack, end to end: hand-edit the signed decision row's card
    // identity — the parsed values no longer match what was signed, verify
    // exits 1 with sig-mismatch. (A whitespace-only edit is deliberately NOT
    // this attack: the signature attests VALUES, not byte formatting.)
    let forged = body.replace("CARD-7", "CARD-99");
    fs::write(home.join("ledger.jsonl"), forged).unwrap();
    let (rc, out, _) = run(&["verify", "--home", home.to_str().unwrap()]);
    assert_eq!(rc, 1, "{out}");
    assert!(out.contains("sig-mismatch"), "{out}");
    fs::remove_dir_all(home).ok();
}

#[test]
fn warden_status_absent_and_broken_key_shapes() {
    let home = rg_home("w-absent", "chair");
    let (rc, out, err) = run(&["warden", "status", "--home", home.to_str().unwrap()]);
    assert_eq!(rc, 0, "{out}{err}");
    assert!(out.contains("no key"), "{out}");
    // verify reports the unsigned era honestly and stays 0 findings.
    let (rc, out, _) = run(&["verify", "--home", home.to_str().unwrap()]);
    assert_eq!(rc, 0);
    assert!(out.contains("warden: no key"), "{out}");
    assert!(out.contains("sig: signed 0 unsigned 5"), "{out}");
    // A broken key file is LOUD: status exits 2 and says appends will refuse.
    fs::write(home.join("warden.key"), "garbage\n").unwrap();
    let (rc, _, err) = run(&["warden", "status", "--home", home.to_str().unwrap()]);
    assert_eq!(rc, 2);
    assert!(err.contains("KEY FILE BROKEN"), "{err}");
    fs::remove_dir_all(home).ok();
}

#[test]
fn route_gated_unknown_argument_fails_closed() {
    let (rc, _, err) = run(&["route-gated", "--nonsense"]);
    assert_eq!(rc, 2);
    assert!(err.contains("unknown argument"), "{err}");
}
