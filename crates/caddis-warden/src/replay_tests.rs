//! Direct tests for the re-judgement half of replay (CARD-0107).
//!
//! `tests/replay.rs` drives the whole command through a spawned binary. These
//! reach `classify` and the filters directly, which is where the honesty lives:
//! the difference between "no law change would deny anything you did" and "most
//! of this ledger could not be re-judged at all" is decided in this file, and
//! until now nothing tested it without a process boundary in the way.

use super::*;
use crate::rows::Row;

fn row(tool: &str, body: &str) -> Row {
    Row {
        seq: 1,
        from: "t".to_string(),
        ts: 0,
        tool: tool.to_string(),
        body: body.to_string(),
    }
}

fn skip_reason(r: &Row) -> &'static str {
    match classify(r).0 {
        Outcome::Skipped(reason) => reason,
        other => panic!("expected a skip, got {:?}", outcome_name(&other)),
    }
}

fn outcome_name(o: &Outcome) -> &'static str {
    match o {
        Outcome::Unchanged => "unchanged",
        Outcome::NewDeny => "new-deny",
        Outcome::Freed => "freed",
        Outcome::NowSteers => "now-steers",
        Outcome::NoLongerSteers => "no-longer-steers",
        Outcome::Skipped(_) => "skipped",
    }
}

#[test]
fn a_write_or_edit_row_is_skipped_as_not_a_command() {
    // write/edit keep no content BY DESIGN, so there is nothing to re-judge.
    // Counting them as "unchanged" is the omission CARD-0106 closed.
    assert_eq!(
        skip_reason(&row("tool.write", "allow|x|/some/path|")),
        SKIP_NOT_A_COMMAND
    );
}

#[test]
fn an_unparsable_body_is_skipped_as_unreadable_never_guessed() {
    assert_eq!(
        skip_reason(&row("tool.bash", "no-pipes-at-all")),
        SKIP_UNREADABLE
    );
}

#[test]
fn a_masked_or_elided_command_is_skipped_because_secrecy_outranks_fidelity() {
    assert_eq!(
        skip_reason(&row("tool.bash", "allow|export K=***redacted||")),
        SKIP_WITHHELD
    );
    assert_eq!(
        skip_reason(&row("tool.bash", "allow|cat big [4096 bytes truncated]||")),
        SKIP_WITHHELD
    );
}

#[test]
fn powershell_rows_are_re_judged_too_not_only_bash() {
    // The estate runs both shells; skipping one would silently halve coverage.
    assert_ne!(
        outcome_name(&classify(&row("tool.powershell", "allow|echo ok||")).0),
        "skipped"
    );
}

#[test]
fn a_command_todays_law_denies_but_the_ledger_allowed_is_a_new_deny() {
    let r = row("tool.bash", "allow|git push --force origin main||");
    assert_eq!(outcome_name(&classify(&r).0), "new-deny");
}

#[test]
fn a_command_the_ledger_denied_and_todays_law_allows_is_freed() {
    let r = row("tool.bash", "deny|echo harmless||why");
    assert_eq!(outcome_name(&classify(&r).0), "freed");
}

#[test]
fn a_verdict_that_did_not_move_is_unchanged() {
    let r = row("tool.bash", "allow|echo ok||");
    assert_eq!(outcome_name(&classify(&r).0), "unchanged");
}

#[test]
fn a_new_deny_reports_the_law_id_that_fired() {
    let (_, fires) = classify(&row("tool.bash", "allow|git push --force origin main||"));
    assert!(!fires.is_empty(), "a deny must name the law that drew it");
    assert!(
        fires.iter().all(|(kind, _)| *kind == "deny"),
        "got: {fires:?}"
    );
}

#[test]
fn fired_ids_reads_a_deny_reason_and_splits_a_steer_why() {
    let deny = Verdict::Deny {
        reason: "caddis-warden [fs.rmrf.wildcard]: no".to_string(),
    };
    assert_eq!(
        fired_ids(&deny),
        vec![("deny", "fs.rmrf.wildcard".to_string())]
    );

    let steer = Verdict::Steer {
        law: "l".to_string(),
        why: "one.law, two.law".to_string(),
    };
    assert_eq!(
        fired_ids(&steer),
        vec![
            ("steer", "one.law".to_string()),
            ("steer", "two.law".to_string())
        ]
    );

    assert!(fired_ids(&Verdict::Allow).is_empty());
}

#[test]
fn a_deny_reason_with_no_bracketed_id_fires_nothing_rather_than_inventing_one() {
    let deny = Verdict::Deny {
        reason: "a sensitive path".to_string(),
    };
    assert!(fired_ids(&deny).is_empty());
}

#[test]
fn filters_default_to_admitting_everything() {
    let f = parse_filters(&["caddis-warden".to_string(), "--replay".to_string()]);
    assert!(f.from.is_none() && f.since_hours.is_none());
    assert!(f.admits(&row("tool.bash", "allow|x||")));
}

#[test]
fn the_from_filter_admits_only_the_named_caller() {
    let args: Vec<String> = ["x", "--from", "peleda"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let f = parse_filters(&args);
    assert_eq!(f.from.as_deref(), Some("peleda"));
    let mut mine = row("tool.bash", "allow|x||");
    mine.from = "peleda".to_string();
    assert!(f.admits(&mine));
    assert!(
        !f.admits(&row("tool.bash", "allow|x||")),
        "from=t is not peleda"
    );
}

#[test]
fn replays_from_filter_also_matches_on_the_dot_boundary() {
    // replay and report must answer the SAME question the same way; a lane
    // filter that agrees in one tool and not the other is worse than either.
    let args: Vec<String> = ["x", "--from", "peleda"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let f = parse_filters(&args);
    let mut scoped = row("tool.bash", "allow|x||");
    scoped.from = "peleda.a1b2c3d4".to_string();
    let mut neighbour = row("tool.bash", "allow|x||");
    neighbour.from = "peleda-two".to_string();
    assert!(f.admits(&scoped));
    assert!(!f.admits(&neighbour));
}

#[test]
fn a_row_with_no_timestamp_is_excluded_by_a_since_window_not_assumed_recent() {
    // ts == 0 means "unknown", and treating unknown as inside the window would
    // quietly widen every --since report.
    let args: Vec<String> = ["x", "--since", "24"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let f = parse_filters(&args);
    assert_eq!(f.since_hours, Some(24));
    assert!(!f.admits(&row("tool.bash", "allow|x||")));

    let mut fresh = row("tool.bash", "allow|x||");
    fresh.ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    assert!(
        f.admits(&fresh),
        "a row from just now is inside a 24h window"
    );
}

#[test]
fn an_unparsable_since_value_is_ignored_rather_than_silently_filtering_everything() {
    let args: Vec<String> = ["x", "--since", "soon"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert!(parse_filters(&args).since_hours.is_none());
}

#[test]
fn a_missing_ledger_path_is_a_usage_error_not_a_crash() {
    assert_eq!(
        run(&["caddis-warden".to_string(), "--replay".to_string()]),
        2
    );
}

#[test]
fn an_unreadable_ledger_is_reported_rather_than_read_as_empty() {
    // "empty" and "unreadable" must never look alike: a counter that swallows
    // an unreadable file reports 0 and calls the history clean.
    let missing = std::env::temp_dir().join(format!("caddis-absent-{}.jsonl", std::process::id()));
    let args = vec![
        "caddis-warden".to_string(),
        "--replay".to_string(),
        missing.to_string_lossy().into_owned(),
    ];
    assert_eq!(run(&args), 2);
}
