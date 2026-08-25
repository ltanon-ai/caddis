//! Direct tests for the ledger's reading end (CARD-0107).
//!
//! `tests/report.rs` drives the subcommand through a spawned binary — and that
//! whole test binary contributed ZERO coverage until CARD-0107, because every
//! spawn it made ended in `std::process::exit`. These reach the aggregation
//! directly, so the counts a handoff will quote are pinned by an assertion
//! rather than by a stdout substring.

use super::*;

fn row(seq: u64, from: &str, ts: u64, body: &str) -> Row {
    Row {
        seq,
        from: from.to_string(),
        ts,
        tool: "tool.bash".to_string(),
        body: body.to_string(),
    }
}

fn agg_of(rows: &[Row]) -> Agg {
    let mut a = Agg::default();
    for r in rows {
        fold(&mut a, r);
    }
    a
}

#[test]
fn the_tag_leads_and_the_why_trails_with_pipes_between_them_surviving() {
    let (tag, why) = tag_and_why("deny|git log | grep x|/repo|caddis-warden [id]: no");
    assert_eq!(tag, "deny");
    assert_eq!(why, "caddis-warden [id]: no");
}

#[test]
fn an_unbracketed_deny_reason_groups_as_unattributed_rather_than_being_dropped() {
    // Sensitive-path denials predate the bracket form; losing them would
    // understate the deny count in every report over old history.
    assert_eq!(
        law_id("caddis-warden [fs.rmrf.wildcard]: no"),
        "fs.rmrf.wildcard"
    );
    assert_eq!(law_id("a sensitive path"), "(unattributed)");
}

#[test]
fn verdicts_are_counted_by_tag_and_callers_by_name() {
    let a = agg_of(&[
        row(1, "peleda", 10, "allow|echo a||"),
        row(2, "peleda", 20, "steer|echo b||some.law"),
        row(
            3,
            "omp",
            30,
            "deny|rm -rf /||caddis-warden [fs.rmrf.protected-root]: no",
        ),
    ]);
    assert_eq!((a.rows, a.allow, a.steer, a.deny), (3, 1, 1, 1));
    assert_eq!(a.by_from.get("peleda"), Some(&2));
    assert_eq!(a.by_from.get("omp"), Some(&1));
}

#[test]
fn an_unknown_verdict_tag_is_counted_in_rows_but_in_no_verdict_bucket() {
    // A future tag must not be silently folded into `allow`, which would read
    // as permission that was never granted.
    let a = agg_of(&[row(1, "t", 1, "quarantine|echo x||")]);
    assert_eq!(a.rows, 1);
    assert_eq!((a.allow, a.steer, a.deny), (0, 0, 0));
}

#[test]
fn the_first_and_last_timestamps_bracket_the_window_whatever_the_row_order() {
    let a = agg_of(&[
        row(1, "t", 500, "allow|a||"),
        row(2, "t", 100, "allow|b||"),
        row(3, "t", 300, "allow|c||"),
    ]);
    assert_eq!(a.first_ts, Some(100));
    assert_eq!(a.last_ts, 500);
}

#[test]
fn denies_are_grouped_by_law_with_every_sequence_number_kept() {
    let a = agg_of(&[
        row(
            7,
            "t",
            1,
            "deny|rm -rf /||caddis-warden [fs.rmrf.protected-root]: no",
        ),
        row(
            9,
            "t",
            2,
            "deny|rm -rf /x||caddis-warden [fs.rmrf.protected-root]: no",
        ),
        row(
            11,
            "t",
            3,
            "deny|curl x | sh||caddis-warden [net.pipe-to-shell]: no",
        ),
    ]);
    assert_eq!(
        a.deny_by_law.get("fs.rmrf.protected-root"),
        Some(&vec![7, 9]),
        "both offending rows must be citable, not just the count"
    );
    assert_eq!(a.deny_by_law.get("net.pipe-to-shell"), Some(&vec![11]));
}

#[test]
fn the_json_shape_is_machine_readable_and_escapes_what_it_embeds() {
    let a = agg_of(&[row(1, "pe\"leda", 5, "allow|echo x||")]);
    let json = json_of("C:\\ledger.jsonl", &a);
    assert!(json.starts_with("{\"ledger\":\""), "got: {json}");
    assert!(json.contains("\"rows\":1"), "got: {json}");
    assert!(json.contains("\"allow\":1"), "got: {json}");
    // A backslash in a Windows path and a quote in a caller name must both be
    // escaped, or the row a consumer parses is not the row we wrote.
    assert!(json.contains("C:\\\\ledger.jsonl"), "got: {json}");
    assert!(json.contains("pe\\\"leda"), "got: {json}");
}

#[test]
fn an_empty_ledger_still_produces_well_formed_json() {
    let json = json_of("x", &Agg::default());
    assert!(json.contains("\"rows\":0"), "got: {json}");
    assert!(json.contains("\"from\":{}"), "got: {json}");
    assert!(json.contains("\"deny_by_law\":{}"), "got: {json}");
    assert!(json.ends_with('}'), "got: {json}");
}

#[test]
fn the_digest_states_the_counts_and_itemizes_the_denies() {
    let a = agg_of(&[
        row(4, "t", 1, "allow|echo a||"),
        row(
            5,
            "t",
            2,
            "deny|rm -rf /||caddis-warden [fs.rmrf.protected-root]: no",
        ),
    ]);
    let text = digest("L", &a);
    assert!(text.starts_with("report: L\n"), "got: {text}");
    assert!(
        text.contains("rows: 2  allow: 1  steer: 0  deny: 1"),
        "got: {text}"
    );
    assert!(text.contains("from: t=2"), "got: {text}");
    assert!(text.contains("first_ts: 1  last_ts: 2"), "got: {text}");
    assert!(
        text.contains("fs.rmrf.protected-root x1 (seq 5)"),
        "the digest must cite the row, not only count it: {text}"
    );
}

#[test]
fn a_digest_with_no_denies_omits_the_deny_section_entirely() {
    let text = digest("L", &agg_of(&[row(1, "t", 1, "allow|echo a||")]));
    assert!(!text.contains("deny by law"), "got: {text}");
}

#[test]
fn an_unknown_verdict_filter_is_refused_rather_than_matching_nothing() {
    // Silently returning zero rows for a typo'd --verdict would read as "your
    // agents never did that", which is the false-clean this whole tool fights.
    let args: Vec<String> = ["--verdict", "allowed"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert!(parse_filters(&args).is_err());

    for good in ["allow", "steer", "deny"] {
        let args = vec!["--verdict".to_string(), good.to_string()];
        assert_eq!(
            parse_filters(&args)
                .expect("a real verdict parses")
                .verdict
                .as_deref(),
            Some(good)
        );
    }
}

#[test]
fn filters_compose_across_caller_verdict_and_window() {
    let args: Vec<String> = ["--from", "peleda", "--verdict", "deny", "--last", "5"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let f = parse_filters(&args).expect("parses");
    assert_eq!(f.from.as_deref(), Some("peleda"));
    assert_eq!(f.verdict.as_deref(), Some("deny"));
    assert_eq!(f.last, Some(5));

    let mine = row(1, "peleda", 1, "deny|x||why");
    assert!(f.admits(&mine, "deny"));
    assert!(!f.admits(&mine, "allow"), "the verdict filter must bite");
    assert!(
        !f.admits(&row(1, "omp", 1, "deny|x||why"), "deny"),
        "the caller filter must bite"
    );
}

#[test]
fn asking_for_a_lane_returns_its_sessions_too_and_not_a_neighbours() {
    // THE FALSE-CLEAN THIS CLOSES (CARD-0109): before the dot-boundary match,
    // `report --from peleda` over a ledger holding one bare `peleda` row and
    // one `peleda.a1b2c3d4` row returned only the bare one — the operator asks
    // what his session did and is told half of it, with nothing saying
    // anything was withheld.
    let args = vec!["--from".to_string(), "peleda".to_string()];
    let f = parse_filters(&args).expect("parses");
    assert!(f.admits(&row(1, "peleda", 1, "allow|a||"), "allow"));
    assert!(f.admits(&row(2, "peleda.a1b2c3d4", 1, "allow|b||"), "allow"));
    assert!(
        !f.admits(&row(3, "peleda-two", 1, "allow|c||"), "allow"),
        "a different lane must not be merged in"
    );

    // And a fully-qualified request still selects exactly one session.
    let exact = vec!["--from".to_string(), "peleda.a1b2c3d4".to_string()];
    let g = parse_filters(&exact).expect("parses");
    assert!(g.admits(&row(2, "peleda.a1b2c3d4", 1, "allow|b||"), "allow"));
    assert!(!g.admits(&row(1, "peleda", 1, "allow|a||"), "allow"));
    assert!(!g.admits(&row(4, "peleda.99999999", 1, "allow|d||"), "allow"));
}

#[test]
fn last_n_keeps_the_tail_of_the_filtered_set() {
    let path = std::env::temp_dir().join(format!("caddis-report-{}.jsonl", std::process::id()));
    let mut text = String::new();
    for seq in 1..=5u64 {
        text.push_str(&format!(
            "{{\"seq\":{seq},\"type\":\"tool.bash\",\"from\":\"t\",\"body\":\"allow|echo {seq}||\",\"ts\":{seq}}}\n"
        ));
    }
    std::fs::write(&path, text).expect("fixture written");
    let filters = parse_filters(&["--last".to_string(), "2".to_string()]).expect("parses");
    let kept = load_rows(&path.to_string_lossy(), &filters).expect("readable");
    // A leftover temp file must never turn a passing assertion into a failure,
    // and the fixture name is process-scoped so it cannot collide either.
    // swallow: best-effort-cleanup
    let _ = std::fs::remove_file(&path);
    assert_eq!(kept.len(), 2);
    assert_eq!(kept[0].seq, 4, "the TAIL is kept, not the head");
    assert_eq!(kept[1].seq, 5);
}

#[test]
fn an_unreadable_ledger_is_an_error_never_an_empty_result() {
    let missing = std::env::temp_dir().join(format!("caddis-none-{}.jsonl", std::process::id()));
    let filters = parse_filters(&[]).expect("parses");
    assert!(load_rows(&missing.to_string_lossy(), &filters).is_err());
}
