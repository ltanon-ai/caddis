//! Direct tests for the integrity reading end (WARDEN-LEDGER-INTEGRITY
//! 2026-08-26, recommendation 3). The counts a handoff will quote are pinned
//! by assertion, not by a stdout substring (CARD-0107 style).

use super::*;

fn good(seq: u64, from: &str, ts: u64) -> String {
    format!(
        "{{\"seq\":{seq},\"v\":11,\"type\":\"tool.bash\",\
         \"body\":\"allow|echo a||\",\"from\":\"{from}\",\"ts\":{ts},\"id\":\"i{seq}\"}}"
    )
}

/// The two corruption shapes the live ledger actually shows (survey L1):
/// two writers' rows fused mid-line, and an orphaned tail fragment.
fn fused() -> String {
    "{\"seq\":{\"seq\":32773277,\"v\":,\"v\":11,\"type\":\"tool.bas".to_string()
}

fn orphan() -> String {
    "\"}".to_string()
}

#[test]
fn a_clean_ledger_has_zero_findings_and_reports_clean() {
    let text = format!(
        "{}\n{}\n{}\n",
        good(1, "omp", 10),
        good(2, "omp", 20),
        good(3, "caddis", 30)
    );
    let s = scan(&text);
    assert_eq!((s.scanned, s.rows, s.unparseable_total), (3, 3, 0));
    assert!(dup_seqs(&s).is_empty());
    let d = digest("L", &s);
    assert!(d.contains("status: CLEAN"), "{d}");
    assert!(d.contains("coverage: 100.00%"), "{d}");
}

#[test]
fn fused_and_orphan_lines_are_unparseable_with_line_numbers() {
    let text = format!(
        "{}\n{}\n{}\n{}\n",
        good(1, "omp", 10),
        fused(),
        good(2, "omp", 20),
        orphan()
    );
    let s = scan(&text);
    assert_eq!(s.unparseable_total, 2);
    assert_eq!(s.unparseable[0].0, 2, "the fused row sits on line 2");
    assert_eq!(s.unparseable[1].0, 4, "the orphan tail sits on line 4");
    assert!(digest("L", &s).contains("status: FINDINGS"));
}

#[test]
fn examples_are_capped_but_the_total_counts_every_corrupt_line() {
    let mut lines: Vec<String> = Vec::new();
    for i in 0..7 {
        lines.push(good(i + 1, "omp", 10));
        lines.push(fused());
    }
    let s = scan(&lines.join("\n"));
    assert_eq!(s.unparseable_total, 7);
    assert_eq!(s.unparseable.len(), EXAMPLES);
    assert_eq!(s.rows, 7);
}

#[test]
fn dup_seqs_rank_worst_first_and_ties_break_on_seq_ascending() {
    let mut text = String::new();
    for seq in [1usize, 2, 5] {
        let n = if seq == 5 { 2 } else { 3 };
        for k in 0..n {
            text.push_str(&good(100 + (seq as u64) * 10 + k, "omp", 10));
            text.push('\n');
            // re-append the same seq from a different writer (L2 shape)
            text.push_str(&good(seq as u64, "bee", 10));
            text.push('\n');
        }
    }
    let s = scan(&text);
    let dups = dup_seqs(&s);
    assert_eq!(dups[0], (1, 3));
    assert_eq!(dups[1], (2, 3));
    assert_eq!(dups[2], (5, 2));
    let j = json_of("L", &s);
    assert!(j.contains("\"dup_seq_values\":3"), "{j}");
    assert!(j.contains("\"dup_seq_rows\":8"), "{j}");
    assert!(j.contains("\"seq\":1,\"count\":3"), "{j}");
}

#[test]
fn blank_lines_are_counted_not_treated_as_corruption() {
    let text = format!("\n\n{}\n\n", good(1, "omp", 10));
    let s = scan(&text);
    assert_eq!((s.scanned, s.blank, s.rows, s.unparseable_total), (1, 3, 1, 0));
    assert!(digest("L", &s).contains("status: CLEAN"));
}

#[test]
fn coverage_is_integer_math_and_an_empty_ledger_is_vacuously_complete() {
    let empty = scan("");
    assert_eq!(coverage_hundredths(&empty), 10000);
    let text = format!("{}\n{}\n", good(1, "omp", 10), fused());
    let s = scan(&text);
    assert_eq!(coverage_hundredths(&s), 5000);
    let j = json_of("L", &s);
    assert!(j.contains("\"coverage_pct\":50.00"), "{j}");
    assert!(j.contains("\"status\":\"findings\""), "{j}");
}

#[test]
fn json_carries_locatable_examples_and_escapes_the_head() {
    let evil = "{\"seq\":1 \"type\": unclosed".to_string();
    let text = format!("{}\n{}\n", good(1, "omp", 10), evil);
    let s = scan(&text);
    let j = json_of("L", &s);
    assert!(j.contains("\"line\":2"), "{j}");
    assert!(j.contains("\\\"seq\\\":1"), "quotes in the head must survive: {j}");
}

#[test]
fn run_reads_a_positional_ledger_and_codes_findings_vs_clean() {
    let dir = std::env::temp_dir();
    let bad = dir.join(format!("caddis-verify-bad-{}", std::process::id()));
    let ok = dir.join(format!("caddis-verify-ok-{}", std::process::id()));
    std::fs::write(&bad, format!("{}\n{}\n", good(1, "omp", 10), fused())).unwrap();
    std::fs::write(&ok, format!("{}\n", good(1, "omp", 10))).unwrap();
    let rc_bad = run(&[
        "caddis-warden".to_string(),
        "verify".to_string(),
        bad.to_string_lossy().into_owned(),
    ]);
    let rc_ok = run(&[
        "caddis-warden".to_string(),
        "verify".to_string(),
        ok.to_string_lossy().into_owned(),
    ]);
    let _ = std::fs::remove_file(&bad);
    let _ = std::fs::remove_file(&ok);
    assert_eq!(rc_bad, FINDINGS);
    assert_eq!(rc_ok, 0);
}
