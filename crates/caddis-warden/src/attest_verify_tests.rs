//! Direct tests for rendering and for the field readers the verifier stands on
//! (CARD-0114). The verifier's RED behaviour — a tampered bundle must fail — is
//! driven end to end through the real binary in `tests/attest_cli.rs`, because
//! a verifier nobody has watched go red is `assert(true == true)`.

use super::*;

fn bundle() -> Bundle {
    Bundle {
        card_id: "CARD-A".into(),
        card_path: "C:/w/_card_a.md".into(),
        card_hash: "abc123".into(),
        allowlist: vec!["src/a.rs".into()],
        blast: 2,
        card_readable: true,
        opened_at_row: 3,
        closed_at_row: 9,
        from: "peleda.s1".into(),
        allow: 5,
        steer: 1,
        deny: 2,
        files: [("src/a.rs".to_string(), 2u64)].into_iter().collect(),
        outside: vec!["src/x.rs".into()],
        red_test_seen: true,
        laws: [("fs.rmrf".to_string(), 2u64)].into_iter().collect(),
        unreadable: 4,
    }
}

#[test]
fn the_row_path_survives_pipes_inside_the_command() {
    assert_eq!(row_path("allow|git log | grep x|src/a.rs|why"), "src/a.rs");
    assert_eq!(row_path("allow|echo a||"), "");
    assert_eq!(row_path("nonsense"), "");
}

#[test]
fn the_text_bundle_leads_with_what_went_outside_the_declaration() {
    let t = render_text(&bundle());
    assert!(t.contains("OUTSIDE     : 1 file(s) written outside"), "{t}");
    assert!(t.contains("src/x.rs"), "{t}");
    assert!(t.contains("window      : ledger rows 3..9"), "{t}");
    assert!(t.contains("never seq"), "{t}");
}

#[test]
fn a_clean_bundle_says_outside_none_rather_than_omitting_the_line() {
    // An absent line reads as "not checked"; the word `none` reads as checked.
    let mut b = bundle();
    b.outside.clear();
    assert!(render_text(&b).contains("OUTSIDE     : none"));
}

#[test]
fn the_red_test_line_never_claims_more_than_attempted() {
    let t = render_text(&bundle());
    assert!(t.contains("ATTEMPTED"), "{t}");
    assert!(t.contains("not proof it passed"), "{t}");
}

#[test]
fn the_text_bundle_prints_every_limit() {
    let t = render_text(&bundle());
    for l in LIMITS {
        assert!(t.contains(l), "missing limit: {l}");
    }
}

#[test]
fn a_v1_card_says_it_bounds_nothing_rather_than_showing_an_empty_list() {
    let mut b = bundle();
    b.allowlist.clear();
    assert!(render_text(&b).contains("a v1 card bounds nothing"));
}

#[test]
fn the_json_bundle_carries_the_limits_too() {
    let j = render_json(&bundle());
    assert!(j.starts_with('{') && j.ends_with('}'), "{j}");
    assert!(j.contains("\"limits\":["), "{j}");
    assert!(j.contains("no exit code"), "{j}");
    assert!(j.contains("\"red_test_attempted\":true"), "{j}");
    assert!(
        j.contains("\"files_outside_allowlist\":[\"src/x.rs\"]"),
        "{j}"
    );
}

#[test]
fn a_claim_is_ok_only_when_the_two_sides_are_identical() {
    let same = Claim {
        name: "x",
        claimed: "3".into(),
        actual: "3".into(),
    };
    let diff = Claim {
        name: "x",
        claimed: "3".into(),
        actual: "4".into(),
    };
    let absent = Claim {
        name: "x",
        claimed: "(absent)".into(),
        actual: "0".into(),
    };
    assert!(same.ok());
    assert!(!diff.ok());
    // A DELETED field must not pass by comparing equal to nothing.
    assert!(!absent.ok());
}
