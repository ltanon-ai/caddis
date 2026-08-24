//! report.rs — REPORT-1: `caddis-warden report` aggregates the ledger the
//! warden itself writes. The ledger exists to answer "what did the agent do
//! last night"; report is the reading end — counts by verdict and caller,
//! first/last timestamps, deny reasons grouped by the law id the why field
//! carries. These tests pin the contract at the binary level, fixture
//! ledger via CADDIS_WARDEN_LEDGER, hand-computed expectations.

use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

fn row(seq: u64, from: &str, ts: u64, body: &str) -> String {
    format!(
        "{{\"seq\":{seq},\"v\":1,\"id\":\"x{seq:016}\",\"idem_key\":\"k{seq}\",\
         \"type\":\"tool.bash\",\"from\":\"{from}\",\"to\":\"warden\",\"body\":{body:?},\"ts\":{ts}}}\n",
        seq = seq,
        from = from,
        ts = ts,
        body = body
    )
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn fixture(tag: &str) -> (std::path::PathBuf, u64) {
    let recent = now_secs();
    let path =
        std::env::temp_dir().join(format!("caddis-report-{}-{tag}.jsonl", std::process::id()));
    let rows = format!(
        "{}{}{}{}{}{}",
        row(1, "omp", 100, "allow|echo ok||"),
        row(2, "omp", 200, "allow|ls||"),
        row(
            3,
            "rlm",
            300,
            "deny|git push --force origin main||caddis-warden [git.push.force-to-protected]: x"
        ),
        row(4, "rlm", 400, "steer|git reset --hard||git.reset.discards-uncommitted"),
        row(5, "omp", recent, "allow|pwd||"),
        row(6, "rlm", 600, "steer|git status||git.reset.discards-uncommitted"),
    );
    std::fs::write(&path, rows).unwrap();
    (path, recent)
}

/// Run `caddis-warden report` against `path` — the ledger location is the
/// shared CADDIS_WARDEN_LEDGER env var, exactly as the warden itself reads it.
fn report(path: &std::path::Path, extra: &[&str]) -> (String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_caddis-warden"))
        .arg("report")
        .args(extra)
        .env("CADDIS_WARDEN_LEDGER", path)
        .stdin(Stdio::null())
        .output()
        .expect("spawn warden");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn report_counts_match_hand_computed() {
    let (path, _recent) = fixture("counts");
    let (digest, code) = report(&path, &[]);
    let _ = std::fs::remove_file(&path);
    assert_eq!(code, 0, "report exits clean, got: {digest}");
    assert!(
        digest.contains("rows: 6") && digest.contains("allow: 3")
            && digest.contains("steer: 2") && digest.contains("deny: 1"),
        "hand-computed verdict counts, got: {digest}"
    );
    assert!(
        digest.contains("omp=3") && digest.contains("rlm=3"),
        "per-caller counts, got: {digest}"
    );
    assert!(
        digest.contains("first_ts") || digest.contains("first: 100"),
        "first timestamp is reported, got: {digest}"
    );
    assert!(
        digest.contains("git.push.force-to-protected"),
        "deny reasons are grouped by the law id in the why field, got: {digest}"
    );
}

#[test]
fn report_filters_compose() {
    let (path, _recent) = fixture("filters");
    let (digest, code) = report(&path, &["--from", "rlm", "--verdict", "steer"]);
    let _ = std::fs::remove_file(&path);
    assert_eq!(code, 0, "got: {digest}");
    assert!(
        digest.contains("rows: 2") && digest.contains("steer: 2"),
        "from+verdict filters compose, got: {digest}"
    );
    assert!(
        !digest.contains("allow: 1"),
        "omp's allows are outside the window, got: {digest}"
    );

    let (path, _recent) = fixture("lastn");
    let (digest, code) = report(&path, &["--verdict", "allow", "--last", "1"]);
    let _ = std::fs::remove_file(&path);
    assert_eq!(code, 0, "got: {digest}");
    assert!(
        digest.contains("rows: 1"),
        "--last takes the most recent N of the filtered set, got: {digest}"
    );

    let (path, _recent) = fixture("since");
    let (digest, code) = report(&path, &["--since", "1"]);
    let _ = std::fs::remove_file(&path);
    assert_eq!(code, 0, "got: {digest}");
    assert!(
        digest.contains("rows: 1"),
        "--since keeps only the fresh row, got: {digest}"
    );
}

#[test]
fn report_json_is_machine_readable() {
    let (path, recent) = fixture("json");
    let (json, code) = report(&path, &["--json"]);
    let _ = std::fs::remove_file(&path);
    assert_eq!(code, 0, "got: {json}");
    assert!(
        json.contains("\"rows\":6") && json.contains("\"deny\":1")
            && json.contains("\"first_ts\":100") && json.contains(&format!("\"last_ts\":{recent}")),
        "flat json with the aggregates, got: {json}"
    );
    assert!(
        json.contains("\"git.push.force-to-protected\""),
        "deny-by-law grouping survives the json shape, got: {json}"
    );
}

#[test]
fn empty_ledger_reports_honest_zeros() {
    let path =
        std::env::temp_dir().join(format!("caddis-report-{}-empty.jsonl", std::process::id()));
    std::fs::write(&path, "").unwrap();
    let (digest, code) = report(&path, &[]);
    let _ = std::fs::remove_file(&path);
    assert_eq!(code, 0, "an empty ledger is not an error, got: {digest}");
    assert!(
        digest.contains("rows: 0") && digest.contains("allow: 0"),
        "zeros, stated plainly, got: {digest}"
    );
}

#[test]
fn report_rejects_an_unknown_verdict_filter() {
    let (path, _recent) = fixture("badverdict");
    let (_out, code) = report(&path, &["--verdict", "maybe"]);
    let _ = std::fs::remove_file(&path);
    assert_eq!(code, 2, "usage errors exit 2, never a bogus report");
}
