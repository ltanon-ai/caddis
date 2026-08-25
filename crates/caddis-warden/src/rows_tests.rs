//! Direct tests for the ONE ledger-row parser (CARD-0107).
//!
//! This file format is what the ledger's credibility rests on, and until now it
//! was only ever exercised through a spawned binary — so a parser bug could
//! only ever surface as a wrong NUMBER three layers away in a replay summary.
//! These pin the shapes the real ledger actually contains, including the two
//! that bit earlier cards: pipes inside a recorded command, and a `ts` written
//! quoted by the ledger but bare by the fixtures.

use super::*;

#[test]
fn a_real_ledger_row_yields_all_five_fields() {
    let line = r#"{"seq":42,"v":1,"id":"x","idem_key":"k","type":"tool.bash","from":"peleda","to":"warden","body":"allow|echo ok||","ts":"1787000000"}"#;
    let row = parse_row(line).expect("a well-formed row parses");
    assert_eq!(row.seq, 42);
    assert_eq!(row.from, "peleda");
    assert_eq!(row.ts, 1787000000);
    assert_eq!(row.tool, "tool.bash");
    assert_eq!(row.body, "allow|echo ok||");
}

#[test]
fn a_bare_ts_parses_too_because_the_fixtures_write_it_unquoted() {
    let line = r#"{"seq":1,"type":"tool.bash","from":"t","body":"allow|x||","ts":7}"#;
    assert_eq!(parse_row(line).expect("bare ts").ts, 7);
}

#[test]
fn a_line_that_is_not_a_ledger_row_is_none_not_a_panic() {
    assert!(parse_row("").is_none());
    assert!(parse_row("not json at all").is_none());
    // seq present but no type: still not a row this parser can use.
    assert!(parse_row(r#"{"seq":1}"#).is_none());
}

#[test]
fn a_missing_from_field_is_empty_rather_than_dropping_the_row() {
    // Rows predating the `from` label must still be counted, not silently lost.
    let line = r#"{"seq":3,"type":"tool.bash","body":"allow|x||","ts":1}"#;
    let row = parse_row(line).expect("the row survives a missing from");
    assert_eq!(row.from, "");
    assert_eq!(row.seq, 3);
}

#[test]
fn an_unparsable_ts_falls_back_to_zero_rather_than_dropping_the_row() {
    let line = r#"{"seq":4,"type":"tool.bash","from":"t","body":"allow|x||","ts":"later"}"#;
    assert_eq!(parse_row(line).expect("row survives a bad ts").ts, 0);
}

#[test]
fn extract_reads_a_string_field_to_its_unescaped_closing_quote() {
    let line = r#"{"body":"he said \"stop\" then left","ts":1}"#;
    assert_eq!(
        extract(line, "\"body\":\"").expect("body found"),
        r#"he said \"stop\" then left"#
    );
}

#[test]
fn extract_returns_none_for_an_unterminated_string() {
    // A truncated write must not be read as a complete row.
    assert!(extract(r#"{"body":"never closed"#, "\"body\":\"").is_none());
}

#[test]
fn unescape_handles_the_minimal_set_the_ledger_writer_produces() {
    assert_eq!(unescape(r"a\nb\tc\rd"), "a\nb\tc\rd");
    assert_eq!(unescape(r#"say \"hi\""#), "say \"hi\"");
    assert_eq!(unescape(r"back\\slash"), r"back\slash");
}

#[test]
fn a_trailing_lone_backslash_is_kept_rather_than_swallowed() {
    // The end-of-input branch: dropping it would silently shorten a command.
    assert_eq!(unescape(r"tail\"), r"tail\");
}

#[test]
fn split_body_keeps_pipes_that_belong_to_the_command() {
    // THE BUG THIS GUARDS: splitting from the left ate every piped command in
    // the ledger, so the busiest rows were the ones recorded wrong.
    let (tag, cmd) = split_body("allow|git log | grep x | wc -l|/repo|why").expect("splits");
    assert_eq!(tag, "allow");
    assert_eq!(cmd, "git log | grep x | wc -l");
}

#[test]
fn split_body_handles_the_ordinary_empty_path_and_why() {
    let (tag, cmd) = split_body("deny|rm -rf /||").expect("splits");
    assert_eq!(tag, "deny");
    assert_eq!(cmd, "rm -rf /");
}

#[test]
fn split_body_refuses_a_body_with_too_few_fields() {
    assert!(split_body("allow").is_none());
    assert!(split_body("allow|only-two").is_none());
    assert!(split_body("allow|cmd|path").is_none());
}

#[test]
fn first_line_capped_takes_one_line_and_at_most_sixty_characters() {
    assert_eq!(first_line_capped("head\ntail"), "head");
    assert_eq!(first_line_capped(""), "");
    let long = "x".repeat(200);
    assert_eq!(first_line_capped(&long).chars().count(), 60);
}

#[test]
fn first_line_capped_counts_characters_not_bytes() {
    // A multi-byte cut would panic or produce mojibake in the digest.
    let lithuanian = "ąčęėįšųūž".repeat(20);
    assert_eq!(first_line_capped(&lithuanian).chars().count(), 60);
}

#[test]
fn from_matches_a_bare_label_and_its_session_scoped_form() {
    // Rows written before CARD-0109 carry a bare label; rows written after
    // carry `<label>.<session>`. A reader asking for the lane must get both, or
    // the answer silently halves the moment sessions become distinguishable.
    assert!(from_matches("peleda", "peleda"));
    assert!(from_matches("peleda.a1b2c3d4", "peleda"));
    assert!(from_matches("peleda.a1b2c3d4", "peleda.a1b2c3d4"));
}

#[test]
fn from_matches_only_on_a_dot_boundary_never_a_bare_prefix() {
    // THE OBVIOUS WRONG FIX. A plain `starts_with` merges two different lanes
    // into one answer, which is worse than the bug it replaces: the first
    // failure hides rows, this one INVENTS them.
    assert!(!from_matches("peleda-two", "peleda"));
    assert!(!from_matches("peledax", "peleda"));
    assert!(!from_matches("peleda", "peleda.a1b2c3d4"));
    assert!(!from_matches("omp", "peleda"));
}

#[test]
fn from_matches_handles_the_empty_ends_without_panicking() {
    assert!(from_matches("", ""));
    assert!(!from_matches("", "peleda"));
    // An empty request matching every dotted label would be a silent no-op
    // filter; it matches only what it literally equals plus dotted children.
    assert!(from_matches(".x", ""));
}

#[test]
fn a_bracketed_law_id_is_found_and_an_unbracketed_reason_yields_none() {
    assert_eq!(
        law_id_bracketed("caddis-warden [fs.rmrf.wildcard]: no"),
        Some("fs.rmrf.wildcard".to_string())
    );
    assert_eq!(law_id_bracketed("a sensitive path, no id here"), None);
    // An EMPTY bracket is not an id — grouping by "" would invent a law.
    assert_eq!(law_id_bracketed("caddis-warden []: no"), None);
}
