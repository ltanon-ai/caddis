//! sessions_tests.rs — P3 slice 1 session-card tests: round-trips per
//! evt, exact-field refusals, disk append/parse law.

use std::path::PathBuf;

use crate::sessions::*;

fn open_row() -> SessionRow {
    SessionRow::Open(SessionOpen {
        conv: "c1".into(),
        kind: "council".into(),
        pin: "a".repeat(64),
        stakes: "medium".into(),
        rerun_of: String::new(),
        actor: "terminal.ashpac".into(),
        warden_card: "CARD-0007".into(),
    })
}

fn usage_row() -> SessionRow {
    SessionRow::Usage(SessionUsage {
        conv: "c1".into(),
        lane: "groq/llama".into(),
        lane_type: crate::LaneType::Bridge,
        provider: "groq".into(),
        model: "transport-served-model".into(),
        cost_class: crate::CostClass::Free,
        tokens_in: 120,
        tokens_out: 340,
    })
}

fn close_row() -> SessionRow {
    SessionRow::Close(SessionClose {
        conv: "c1".into(),
        verdict_digest: "b".repeat(64),
        ship: 1,
        ship_with_changes: 2,
        do_not_ship: 1,
    })
}

fn round_trip(row: SessionRow) {
    let line = encode_row(&row);
    let parsed = parse_rows(&line).unwrap();
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0], row);
}

#[test]
fn open_row_round_trips() {
    round_trip(open_row());
}

#[test]
fn usage_row_round_trips() {
    round_trip(usage_row());
}

#[test]
fn close_row_round_trips() {
    round_trip(close_row());
}

#[test]
fn encode_is_flat_deterministic_json() {
    let line = encode_row(&usage_row());
    assert!(line.starts_with("{\"class\":\"session\",\"evt\":\"usage\""));
    assert!(line.ends_with('}'));
    assert!(!line.contains(": "));
    // No nested objects, no timestamps (MV11).
    assert!(!line.contains("\"ts\""));
    let again = encode_row(&usage_row());
    assert_eq!(line, again);
}

#[test]
fn append_then_parse_from_disk_round_trips_in_order() {
    let dir = std::env::temp_dir().join(format!("caddis-sessions-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path: PathBuf = dir.join("sessions.jsonl");
    let _ = std::fs::remove_file(&path);

    append_row(&path, &open_row()).unwrap();
    append_row(&path, &usage_row()).unwrap();
    append_row(&path, &close_row()).unwrap();

    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(text.lines().count(), 3);
    assert!(text.ends_with('\n'));
    let rows = parse_rows(&text).unwrap();
    assert_eq!(rows, vec![open_row(), usage_row(), close_row()]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unknown_field_is_refused_with_line_number() {
    let mut line = encode_row(&close_row());
    // Inject an unknown field before the final brace.
    line.insert(line.len() - 1, ',');
    line.insert_str(line.len() - 1, "\"extra\":1");
    let err = parse_rows(&line).unwrap_err();
    assert_eq!(
        err,
        SessionRowErr::Malformed {
            line: 1,
            msg: "unknown field 'extra'".into()
        }
    );
}

#[test]
fn missing_field_is_refused() {
    let line = "{\"class\":\"session\",\"evt\":\"close\",\"conv\":\"c1\"}";
    let err = parse_rows(line).unwrap_err();
    match err {
        SessionRowErr::Malformed { line: 1, msg } => assert!(msg.contains("exact field law")),
        other => panic!("{other:?}"),
    }
}

#[test]
fn wrong_class_and_evt_are_refused() {
    let err = parse_rows("{\"class\":\"seat\",\"evt\":\"open\"}").unwrap_err();
    assert!(format!("{err}").contains("this stream carries session rows"));
    let err = parse_rows("{\"class\":\"session\",\"evt\":\"explode\"}").unwrap_err();
    assert!(format!("{err}").contains("open | usage | close"));
}

#[test]
fn empty_lines_are_skipped_and_line_numbers_are_1_based() {
    let line = encode_row(&open_row());
    let text = format!("\n{line}\n\n\"not json\"\n");
    let err = parse_rows(&text).unwrap_err();
    match err {
        SessionRowErr::Malformed { line: 4, .. } => {}
        other => panic!("expected line-4 malformed, got {other:?}"),
    }
}
