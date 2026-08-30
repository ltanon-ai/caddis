//! vilnius_time.rs — operator order 2026-08-29: the board renders
//! Europe/Vilnius local time. DST vectors pin the EU rule (last
//! Sunday March 01:00Z -> last Sunday October 01:00Z) and the
//! render format the board slices (hms takes positions 11..19).

use caddis_organs::util::{iso8601_from_unix_vilnius, unix_from_iso8601, vilnius_offset_secs};

fn unix(iso: &str) -> i64 {
    unix_from_iso8601(iso).expect("parse")
}

#[test]
fn summer_and_winter_offsets() {
    // Mid-summer noon UTC -> +3 (EEST).
    let jul = unix("2026-07-01T12:00:00Z");
    assert_eq!(vilnius_offset_secs(jul), 3 * 3600);
    // Mid-winter noon UTC -> +2 (EET).
    let jan = unix("2026-01-15T12:00:00Z");
    assert_eq!(vilnius_offset_secs(jan), 2 * 3600);
}

#[test]
fn dst_transitions_are_exact() {
    // 2026-03-29 is the last Sunday of March; switch at 01:00Z.
    let before = unix("2026-03-29T00:59:59Z");
    let after = unix("2026-03-29T01:00:00Z");
    assert_eq!(vilnius_offset_secs(before), 2 * 3600);
    assert_eq!(vilnius_offset_secs(after), 3 * 3600);
    // 2026-10-25 is the last Sunday of October; back at 01:00Z.
    let before = unix("2026-10-25T00:59:59Z");
    let after = unix("2026-10-25T01:00:00Z");
    assert_eq!(vilnius_offset_secs(before), 3 * 3600);
    assert_eq!(vilnius_offset_secs(after), 2 * 3600);
}

#[test]
fn render_format_is_sliceable_hhmmss() {
    // Summer noon UTC renders as 15:00:00 local with offset suffix.
    let iso = iso8601_from_unix_vilnius(unix("2026-07-01T12:00:00Z"));
    assert_eq!(&iso[11..19], "15:00:00");
    assert!(iso.ends_with("+03:00"), "suffix carries the offset: {iso}");
    // Winter noon UTC renders as 14:00:00.
    let iso = iso8601_from_unix_vilnius(unix("2026-01-15T12:00:00Z"));
    assert_eq!(&iso[11..19], "14:00:00");
    // Roundtrip: the local string parses back (parser ignores suffix).
    assert!(unix_from_iso8601(&iso).is_some());
}

#[test]
fn parser_roundtrip_against_utc_fn() {
    let secs = unix("2026-08-29T09:15:30Z");
    assert_eq!(unix_from_iso8601("2026-08-29T09:15:30Z"), Some(secs));
    assert!(unix_from_iso8601("not-a-time").is_none());
    assert!(unix_from_iso8601("2026-13-01T00:00:00Z").is_none());
}
