//! ledger.rs tests — determinism law: every append goes through `append_ts`
//! with a fixed ts; only the clock test reads the real clock.

use super::*;
use std::fs;
use std::sync::Arc;
use std::thread;

fn tmpdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("rtr-ledger-{}-{}", tag, std::process::id()));
    fs::create_dir_all(&d).unwrap();
    d
}

fn decision(route_id: &str, lane: &str) -> Row {
    Row::Decision(DecisionRow {
        route_id: route_id.into(),
        card_id: "CARD-1".into(),
        task_class: "coding".into(),
        lane_id: lane.into(),
        lane_tier: LaneTier::Free,
        cost_per_task_usd: 0.0,
        degraded: false,
    })
}

fn outcome(lane: &str, pass: bool) -> Row {
    Row::Outcome(OutcomeRow {
        card_id: "CARD-1".into(),
        task_class: "coding".into(),
        lane_id: lane.into(),
        model: "gemini-2.5-pro".into(),
        cost_tokens: 1200,
        cost_usd_est: 0.0042,
        latency_ms: 8500,
        outcome: if pass { Outcome::Pass } else { Outcome::Fail },
        escalated_to: if pass { None } else { Some("gpt-5.2".into()) },
    })
}

const WAIT: Duration = Duration::from_secs(2);

#[test]
fn decision_roundtrip_and_seq_growth() {
    let dir = tmpdir("rt");
    let led = Ledger::new(dir.join("d.jsonl"));
    let s1 = led
        .append_ts(&decision("r-1", "groq-free"), "2026-08-28T00:00:00Z", WAIT)
        .unwrap();
    let s2 = led
        .append_ts(&outcome("groq-free", false), "2026-08-28T00:00:01Z", WAIT)
        .unwrap();
    assert_eq!((s1, s2), (1, 2));
    let loaded = led.load().unwrap();
    assert_eq!(loaded.bad.len(), 0);
    assert_eq!(loaded.rows.len(), 2);
    assert_eq!(loaded.rows[0].row, decision("r-1", "groq-free"));
    assert_eq!(loaded.rows[1].row, outcome("groq-free", false));
    assert_eq!(loaded.rows[1].seq, 2);
    fs::remove_dir_all(dir).ok();
}

#[test]
fn escaping_survives_roundtrip() {
    let dir = tmpdir("esc");
    let led = Ledger::new(dir.join("d.jsonl"));
    let weird = "la\"ne\\x\n\t🪰\u{0001}";
    let row = Row::Outcome(OutcomeRow {
        card_id: "c".into(),
        task_class: "t".into(),
        lane_id: weird.into(),
        model: "m".into(),
        cost_tokens: 0,
        cost_usd_est: 0.0,
        latency_ms: 0,
        outcome: Outcome::Pass,
        escalated_to: Some("null".into()), // the sniffing trap: a LANE NAMED null
    });
    led.append_ts(&row, "t\"s\\", WAIT).unwrap();
    let loaded = led.load().unwrap();
    assert_eq!(loaded.bad.len(), 0, "no torn lines from control chars");
    assert_eq!(
        loaded.rows[0].row, row,
        "exact roundtrip incl. lane named null"
    );
    fs::remove_dir_all(dir).ok();
}

#[test]
fn one_append_is_one_line() {
    let dir = tmpdir("oneline");
    let led = Ledger::new(dir.join("d.jsonl"));
    led.append_ts(&outcome("groq-free", true), "2026-08-28T00:00:00Z", WAIT)
        .unwrap();
    let raw = fs::read_to_string(led.path()).unwrap();
    assert_eq!(raw.matches('\n').count(), 1, "exactly one newline");
    assert!(raw.ends_with("}\n"));
    fs::remove_dir_all(dir).ok();
}

#[test]
fn seq_comes_from_max_not_line_count() {
    // Model-voice lesson: a hand-forked file (duplicate seq) must not make
    // the next append fork further. max(seq)+1, never line count.
    let dir = tmpdir("fork");
    let path = dir.join("d.jsonl");
    fs::write(&path, "").unwrap();
    let led = Ledger::new(&path);
    let r1 = decision("r-1", "l");
    let r2 = decision("r-2", "l");
    led.append_ts(&r1, "t", WAIT).unwrap();
    led.append_ts(&r2, "t", WAIT).unwrap();
    // Hand-append a FORK: same max seq 2, different route id.
    let mut text = fs::read_to_string(&path).unwrap();
    text.push_str(&text.lines().last().unwrap().replace("r-2", "r-2-fork"));
    text.push('\n');
    fs::write(&path, text).unwrap();
    let s = led.append_ts(&decision("r-3", "l"), "t", WAIT).unwrap();
    assert_eq!(s, 3, "seq = max+1 even with a forked file");
    let loaded = led.load().unwrap();
    assert_eq!(
        loaded.rows.iter().filter(|p| p.seq == 2).count(),
        2,
        "fork is visible"
    );
    fs::remove_dir_all(dir).ok();
}

#[test]
fn r6_concurrent_appends_are_serialized() {
    // THE R6 proof: two writers, 10 rows each, one stream — 20 unique seqs,
    // zero torn lines, verify-clean file.
    let dir = tmpdir("r6");
    let led = Arc::new(Ledger::new(dir.join("d.jsonl")));
    let mk = |n: usize| {
        (0..10)
            .map(|i| decision(&format!("r-{n}-{i}"), "lane-a"))
            .collect::<Vec<_>>()
    };
    let rows_a = Arc::new(mk(1));
    let rows_b = Arc::new(mk(2));
    let a = Arc::clone(&led);
    let ra = Arc::clone(&rows_a);
    let h1 = thread::spawn(move || {
        for r in ra.iter() {
            a.append_ts(r, "t", WAIT).unwrap();
        }
    });
    let b = Arc::clone(&led);
    let rb = Arc::clone(&rows_b);
    let h2 = thread::spawn(move || {
        for r in rb.iter() {
            b.append_ts(r, "t", WAIT).unwrap();
        }
    });
    h1.join().unwrap();
    h2.join().unwrap();
    let loaded = led.load().unwrap();
    assert_eq!(loaded.bad.len(), 0);
    assert_eq!(loaded.rows.len(), 20);
    let mut seqs: Vec<u64> = loaded.rows.iter().map(|p| p.seq).collect();
    seqs.sort_unstable();
    let expected: Vec<u64> = (1..=20).collect();
    assert_eq!(seqs, expected, "no dup, no gap, no loss");
    fs::remove_dir_all(dir).ok();
}

#[test]
fn r6_held_lock_fails_closed() {
    let dir = tmpdir("failclosed");
    let path = dir.join("d.jsonl");
    let led = Ledger::new(&path);
    // Foreign FRESH lock: append must refuse (concurrent append forbidden).
    fs::write(path.with_extension("lock"), "someone-else").unwrap();
    let err = led
        .append_ts(&decision("r", "l"), "t", Duration::from_millis(30))
        .unwrap_err();
    assert_eq!(err, LedgerErr::LockBusy);
    assert!(!path.exists(), "nothing was written without exclusion");
    fs::remove_file(path.with_extension("lock")).unwrap();
    fs::remove_dir_all(dir).ok();
}

#[test]
fn non_finite_cost_is_refused() {
    let dir = tmpdir("nan");
    let led = Ledger::new(dir.join("d.jsonl"));
    let mut row = decision("r", "l");
    if let Row::Decision(d) = &mut row {
        d.cost_per_task_usd = f64::NAN;
    }
    assert_eq!(
        led.append_ts(&row, "t", Duration::from_millis(50))
            .unwrap_err(),
        LedgerErr::BadRow("decision cost_per_task_usd not finite")
    );
    fs::remove_dir_all(dir).ok();
}

#[test]
fn oversized_fields_elide_not_corrupt() {
    let dir = tmpdir("elide");
    let led = Ledger::new(dir.join("d.jsonl"));
    let huge = "x".repeat(10_000);
    let row = Row::Outcome(OutcomeRow {
        card_id: huge.clone(),
        task_class: "t".into(),
        lane_id: "l".into(),
        model: "m".into(),
        cost_tokens: 1,
        cost_usd_est: 1.0,
        latency_ms: 1,
        outcome: Outcome::Pass,
        escalated_to: None,
    });
    let seq = led.append_ts(&row, "t", WAIT).unwrap();
    assert_eq!(seq, 1);
    let raw = fs::read_to_string(led.path()).unwrap();
    assert!(raw.len() < ROW_CAP + 512, "row bounded by construction");
    let loaded = led.load().unwrap();
    assert_eq!(loaded.bad.len(), 0, "elided row still parses");
    match &loaded.rows[0].row {
        Row::Outcome(o) => {
            assert!(o.card_id.ends_with("..."), "cut is visible");
            assert!(o.card_id.len() < huge.len());
        }
        _ => panic!("wrong kind"),
    }
    fs::remove_dir_all(dir).ok();
}

#[test]
fn parser_rejects_nesting_and_garbage() {
    assert!(
        parse_full("{\"seq\":1,\"ts\":\"t\",\"kind\":{\"a\":1}}").is_err(),
        "nesting refused"
    );
    assert!(parse_full("not json at all").is_err());
    assert!(
        parse_full("{\"seq\":1,\"ts\":\"t\",\"kind\":\"banana\"}").is_err(),
        "unknown kind"
    );
    assert!(
        parse_full("{\"seq\":0,\"ts\":\"t\",\"kind\":\"decision\"}").is_err(),
        "seq starts at 1"
    );
    assert!(
        parse_full("{\"seq\":1.5,\"ts\":\"t\",\"kind\":\"decision\"}").is_err(),
        "fractional seq"
    );
}

#[test]
fn o2_droid_tier_is_unparseable_in_the_ledger_too() {
    let line =
        "{\"seq\":1,\"ts\":\"t\",\"kind\":\"decision\",\"route_id\":\"r\",\"card_id\":\"c\",\
                \"task_class\":\"t\",\"lane_id\":\"l\",\"tier\":\"droid\",\
                \"cost_per_task_usd\":0,\"degraded\":false}";
    let err = parse_full(line).unwrap_err();
    assert!(err.contains("droid"), "got: {err}");
}

#[test]
fn iso_clock_known_epochs() {
    assert_eq!(iso_from_unix(0), "1970-01-01T00:00:00Z");
    assert_eq!(iso_from_unix(1_000_000_000), "2001-09-09T01:46:40Z");
    assert_eq!(iso_from_unix(1_234_567_890), "2009-02-13T23:31:30Z");
    assert_eq!(iso_from_unix(2_000_000_000), "2033-05-18T03:33:20Z");
    assert_eq!(
        iso_from_unix(951_782_400),
        "2000-02-29T00:00:00Z",
        "leap day"
    );
}

#[test]
fn f64_costs_roundtrip_shortest() {
    let dir = tmpdir("f64");
    let led = Ledger::new(dir.join("d.jsonl"));
    let mut row = decision("r", "l");
    if let Row::Decision(d) = &mut row {
        d.cost_per_task_usd = 0.1 + 0.2; // 0.30000000000000004
    }
    led.append_ts(&row, "t", WAIT).unwrap();
    match &led.load().unwrap().rows[0].row {
        Row::Decision(d) => assert_eq!(d.cost_per_task_usd, 0.1 + 0.2),
        _ => panic!(),
    }
    fs::remove_dir_all(dir).ok();
}

#[test]
fn first_append_materializes_missing_state_home() {
    let dir = tmpdir("home-birth");
    // a DEEP missing path — the organ's real first write on a fresh box
    let led = Ledger::new(dir.join("a/b/c/ledger.jsonl"));
    led.append_ts(&decision("r1", "l1"), "t", WAIT).unwrap();
    assert_eq!(led.load().unwrap().rows.len(), 1);
    fs::remove_dir_all(dir).ok();
}

#[test]
fn promotion_row_roundtrips_and_folds_nothing() {
    let dir = tmpdir("promo");
    let led = Ledger::new(dir.join("d.jsonl"));
    let row = Row::Promotion(PromotionRow {
        lane_id: "groq-free".into(),
        task_class: "coding".into(),
        demoted: true,
        trailing_fails: 2,
    });
    led.append_ts(&row, "2026-08-28T00:00:00Z", WAIT).unwrap();
    let loaded = led.load().unwrap();
    assert_eq!(loaded.bad.len(), 0);
    assert_eq!(loaded.rows[0].row, row);
    // QQ1a spirit: a promotion marker is NOT capability evidence — the
    // stats fold must stay empty over a markers-only stream.
    let caps = crate::stats::CapsReport::from_rows(&loaded);
    assert_eq!(caps, crate::stats::CapsReport::default());
    fs::remove_dir_all(dir).ok();
}
