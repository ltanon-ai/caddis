//! Direct tests for the replay renderer (CARD-0107).
//!
//! These assert the EXACT lines, because the rendering is the part that can
//! lie: a coverage line that rounds a third of a ledger up to "100%", or a
//! capped list that truncates without saying so, is the specific dishonesty
//! CARD-0106 was written to end. `tests/replay.rs` pins the same strings
//! through a spawned binary; this pins them without one.

use super::*;

fn reasons(pairs: &[(&'static str, u64)]) -> BTreeMap<&'static str, u64> {
    pairs.iter().copied().collect()
}

#[test]
fn coverage_states_the_fraction_and_names_every_reason() {
    let lines = coverage_lines(
        14036,
        4656,
        9380,
        &reasons(&[("masked", 5200), ("noncmd", 4180)]),
    );
    assert_eq!(
        lines[0],
        "coverage: 4656 of 14036 rows re-judged (33.2%); 9380 could not be"
    );
    // BTreeMap order, so the reasons are stable rather than hash-shuffled:
    // sorted by REASON, which puts "masked" before "noncmd" whatever the counts.
    assert_eq!(lines[1], "  5200 masked");
    assert_eq!(lines[2], "  4180 noncmd");
}

#[test]
fn an_empty_ledger_reports_zero_percent_and_never_divides_by_zero() {
    let lines = coverage_lines(0, 0, 0, &reasons(&[]));
    assert_eq!(
        lines,
        vec!["coverage: 0 of 0 rows re-judged (0.0%); 0 could not be"]
    );
}

#[test]
fn full_coverage_is_not_rounded_up_from_a_near_miss() {
    // 4655/4656 must not print as 100.0% — a reader would take that as "all of
    // it", which is exactly the omission this line exists to close.
    let lines = coverage_lines(4656, 4655, 1, &reasons(&[]));
    assert!(lines[0].contains("(100.0%)"), "got: {}", lines[0]);
    assert!(lines[0].contains("1 could not be"), "got: {}", lines[0]);
}

#[test]
fn a_capped_list_says_how_many_it_did_not_show() {
    let items: Vec<String> = (0..13).map(|n| format!("STEER+ seq={n}")).collect();
    let lines = capped_lines(&items, DRIFT_SHOWN);
    assert_eq!(lines.len(), DRIFT_SHOWN + 1);
    assert_eq!(lines[DRIFT_SHOWN], "... and 3 more not shown");
}

#[test]
fn a_list_that_fits_gets_no_truncation_notice() {
    let items = vec!["one".to_string(), "two".to_string()];
    assert_eq!(capped_lines(&items, DRIFT_SHOWN), items);
}

#[test]
fn an_exactly_full_list_gets_no_truncation_notice() {
    // The off-by-one that would claim "... and 0 more not shown".
    let items: Vec<String> = (0..DRIFT_SHOWN).map(|n| n.to_string()).collect();
    assert_eq!(capped_lines(&items, DRIFT_SHOWN).len(), DRIFT_SHOWN);
}

#[test]
fn law_fires_are_counted_per_id_and_the_unfired_are_named() {
    // A REAL registered id, so the never-fired list has something to exclude,
    // and the exclusion is checked against the SPLIT list rather than by
    // substring: every real id is dotted, so `contains("fs.rmrf")` would also
    // match a longer sibling and pass for the wrong reason.
    let fired_id = checks::registered_ids()[0].to_string();
    let deny: BTreeMap<String, u64> = [(fired_id.clone(), 3)].into_iter().collect();
    let steer: BTreeMap<String, u64> = [(fired_id.clone(), 1), ("zz.invented".to_string(), 2)]
        .into_iter()
        .collect();
    let lines = law_fire_lines(&deny, &steer);
    assert_eq!(lines[0], "law fires (current law over judged rows):");
    assert!(
        lines.contains(&"  zz.invented deny=0 steer=2".to_string()),
        "{lines:?}"
    );
    assert!(
        lines.contains(&format!("  {fired_id} deny=3 steer=1")),
        "{lines:?}"
    );
    let never = lines.last().expect("the never-fired line always trails");
    let unfired: Vec<&str> = never["never fired: ".len()..].split(", ").collect();
    assert!(never.starts_with("never fired: "), "got: {never}");
    assert!(
        !unfired.contains(&fired_id.as_str()),
        "a law that fired is not unfired: {never}"
    );
}

#[test]
fn no_fires_says_none_and_still_lists_every_registered_law() {
    let lines = law_fire_lines(&BTreeMap::new(), &BTreeMap::new());
    assert_eq!(lines[0], "law fires: none");
    let never = lines.last().expect("the never-fired line always trails");
    // Nothing fired, so EVERY registered id must appear — a shorter list here
    // would mean the report quietly forgot laws it never exercised.
    for id in checks::registered_ids() {
        assert!(never.contains(id), "{id} missing from: {never}");
    }
}
