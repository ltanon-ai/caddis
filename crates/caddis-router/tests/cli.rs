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
