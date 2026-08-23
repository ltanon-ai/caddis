//! checks_v10_incident_keys.rs — LOW 7 from the fourth audit: the top-level
//! scanner could not tell a KEY from a string VALUE, and a vanished ref
//! rendered as an empty gap in a HARD denial.
//!
//! `top_level_value` matched a depth-1 quoted run by CONTENT, never asking
//! whether a colon follows. So a row whose value is the STRING `"resolved"`:
//!
//!   {"state": "resolved", "resolved": true, ...}
//!
//! matched the VALUE first, read an empty literal after it, and the row
//! stopped counting as resolved — a RESOLVED incident keeps denying pushes,
//! the opposite of the check's whole purpose.
//!
//! And `"new": null` (the ref vanished; the rewrite left nothing) flowed
//! into the denial message as an empty string: *"655f64d2 is not an ancestor
//! of ."* — a measurement-shaped sentence with nothing measured in it.

use caddis_warden::checks::incidents::{open_incidents_from, push_into_rewritten_repo_with};

#[test]
fn a_value_string_equal_to_the_key_does_not_shadow_the_real_key() {
    // The row IS resolved; the scanner must reach the real `"resolved": true`
    // past the decoy VALUE that spells the same word.
    let log = concat!(
        "{\"repo\": \"E:\\\\T\\\\r\", \"state\": \"resolved\", ",
        "\"resolved\": true, \"ref\": \"main\", \"old\": \"aaaa\", \"new\": \"bbbb\"}\n"
    );
    assert!(
        open_incidents_from(log).is_empty(),
        "a resolved row stays resolved even when a value string spells the key"
    );
}

#[test]
fn an_open_row_with_a_decoy_value_string_stays_open() {
    // The mirror control: `"resolved": false` is real and must win over the
    // decoy — dropping an open incident silently allows a push into a
    // rewritten repo, the failure direction that matters.
    let log = "{\"repo\": \"E:\\\\T\\\\r\", \"state\": \"resolved\", \"resolved\": false}\n";
    let got = open_incidents_from(log);
    assert_eq!(got.len(), 1, "an open row must stay open");
}

#[test]
fn a_nested_key_is_still_not_a_top_level_key() {
    // The depth-1 rule must survive the colon fix: a `"resolved"` nested
    // inside an object is not top-level even though a colon follows it.
    let log = "{\"note\": {\"resolved\": true}, \"repo\": \"E:\\\\T\\\\n\"}\n";
    let got = open_incidents_from(log);
    assert_eq!(got.len(), 1, "the nested key must not resolve the row");
}

#[test]
fn a_vanished_ref_renders_honestly_in_the_denial() {
    // `new: null` — the rewrite left no new commit. The denial used to print
    // "is not an ancestor of ." with nothing in the gap.
    let log = concat!(
        "{\"repo\": \"E:\\\\T\\\\gone\", \"ref\": \"main\", ",
        "\"old\": \"655f64d2aaaa\", \"new\": null, \"verdict\": \"vanished\"}\n"
    );
    let incidents = open_incidents_from(log);
    let finding = push_into_rewritten_repo_with("git -C E:/T/gone push origin main", &incidents)
        .expect("the push into the rewritten repo must still deny");
    assert!(
        finding.contains("vanished"),
        "the denial must say the ref vanished instead of an empty gap: {finding}"
    );
    assert!(
        !finding.contains("of ."),
        "no empty measurement gap may render: {finding}"
    );
}
