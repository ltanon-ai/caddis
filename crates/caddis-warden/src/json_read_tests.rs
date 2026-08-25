//! Direct tests for the bundle field readers (CARD-0114).
//!
//! ⛔ EVERY CASE HERE THAT USES A PATH USES A REALISTIC ONE. The bug these
//! replace was invisible for exactly one reason: every fixture in this crate
//! used relative paths like `src/a.rs`, and the readers counted characters.
//! `files` keys are PATHS, and a Windows path carries a colon.

use super::*;

const BUNDLE: &str = "{\"card_id\":\"CARD-A\",\"card_hash\":\"abc123\",\"allow\":5,\
    \"deny\":2,\"files\":{\"C:/w/src/a.rs\":2,\"D:/other/b.rs\":1},\
    \"files_outside_allowlist\":[\"C:/w/x.rs\"],\"unreadable\":0}";

#[test]
fn a_windows_path_key_counts_as_one_entry_not_two() {
    // The defect: counting ':' saw `C:/w/src/a.rs` as two keys, so an HONEST
    // bundle contradicted itself on files_distinct.
    assert_eq!(obj_len(BUNDLE, "files"), Some(2));
    assert_eq!(arr_len(BUNDLE, "files_outside_allowlist"), Some(1));
}

#[test]
fn a_comma_inside_a_path_is_data_not_structure() {
    let j = "{\"files\":{\"src/a,b.rs\":1},\"outside\":[\"x,y.rs\",\"z.rs\"]}";
    assert_eq!(obj_len(j, "files"), Some(1));
    assert_eq!(arr_len(j, "outside"), Some(2));
}

#[test]
fn an_escaped_quote_inside_a_value_is_data_not_structure() {
    let j = "{\"outside\":[\"src/we\\\"ird.rs\"],\"files\":{\"a\\\"b.rs\":1}}";
    assert_eq!(arr_len(j, "outside"), Some(1));
    assert_eq!(obj_len(j, "files"), Some(1));
}

#[test]
fn empty_containers_read_as_zero_rather_than_unreadable() {
    // ⛔ THE TAMPER THAT MATTERS MOST is emptying `files_outside_allowlist`, so
    // an empty array must read as 0 and compare unequal to a real count.
    let j = "{\"files\":{},\"outside\":[],\"laws\":{}}";
    assert_eq!(obj_len(j, "files"), Some(0));
    assert_eq!(arr_len(j, "outside"), Some(0));
    assert_eq!(obj_len(j, "laws"), Some(0));
}

#[test]
fn a_nested_container_does_not_end_the_count_early() {
    let j = "{\"outside\":[[\"a\",\"b\"],[\"c\"]],\"files\":{\"a\":{\"n\":1},\"b\":{\"n\":2}}}";
    assert_eq!(arr_len(j, "outside"), Some(2));
    assert_eq!(obj_len(j, "files"), Some(2));
}

#[test]
fn an_unterminated_container_is_none_rather_than_a_guess() {
    // A malformed bundle must read as unreadable, never as a number:
    // `(absent)` never compares equal to a real count, so the claim reports
    // CONTRADICTED instead of quietly passing.
    assert_eq!(obj_len("{\"files\":{\"a\":1", "files"), None);
    assert_eq!(arr_len("{\"outside\":[\"a\"", "outside"), None);
}

#[test]
fn an_absent_container_is_none() {
    assert_eq!(obj_len(BUNDLE, "nope"), None);
    assert_eq!(arr_len(BUNDLE, "nope"), None);
}

#[test]
fn numbers_are_read_by_name_and_an_absent_one_is_none() {
    assert_eq!(num(BUNDLE, "allow"), Some(5));
    assert_eq!(num(BUNDLE, "deny"), Some(2));
    assert_eq!(num(BUNDLE, "unreadable"), Some(0));
    assert_eq!(num(BUNDLE, "not_a_field"), None);
}

#[test]
fn strings_are_read_by_name() {
    assert_eq!(text_field(BUNDLE, "card_id").as_deref(), Some("CARD-A"));
    assert_eq!(text_field(BUNDLE, "card_hash").as_deref(), Some("abc123"));
    assert_eq!(text_field(BUNDLE, "nope"), None);
}

#[test]
fn a_string_value_containing_an_escaped_quote_survives_whole() {
    // `split('"').next()` truncated at the escape; a card path or a caller name
    // carrying a quote would have silently become a shorter, different value —
    // and a shorter value compares unequal, raising a FALSE contradiction.
    let j = "{\"from\":\"pe\\\"leda\",\"card_id\":\"X\"}";
    assert_eq!(text_field(j, "from").as_deref(), Some("pe\"leda"));
    assert_eq!(text_field(j, "card_id").as_deref(), Some("X"));
}

#[test]
fn an_unterminated_string_is_none_not_the_rest_of_the_file() {
    assert_eq!(text_field("{\"from\":\"peleda", "from"), None);
}
