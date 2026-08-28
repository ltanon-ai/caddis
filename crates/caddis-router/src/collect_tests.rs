//! Collector tests — REAL fixture dirs + REAL ledger files in temp (the
//! trail shapes are hand-written wire fixtures; every count asserted).

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use super::*;

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("rtr-collect-{}-{}", tag, std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

/// Build one consult dir. `manifest`/`verdicts` = None writes nothing.
fn consult(root: &Path, name: &str, manifest: Option<&str>, verdicts: Option<&str>) {
    let d = root.join(name);
    fs::create_dir_all(&d).unwrap();
    if let Some(m) = manifest {
        fs::write(d.join("MANIFEST.json"), m).unwrap();
    }
    if let Some(v) = verdicts {
        fs::write(d.join("VERDICTS.json"), v).unwrap();
    }
}

const MANIFEST_A: &str = r#"{
  "skeptic": {
    "dispatch_id": "skeptic",
    "provider": "grok-coding",
    "model": "grok-4.6",
    "retried": false,
    "fileref": [{"path": "C:/x/spec.md", "action": "inlined", "bytes": 8363}]
  },
  "chair": {
    "dispatch_id": "nemotron",
    "provider": "openai-compat",
    "model": "nvidia/nemotron-3-super-120b-a12b",
    "retried": false,
    "fileref": []
  },
  "broken-seat": {
    "dispatch_id": "broken",
    "role": "mystery"
  },
  "not-an-object": 7
}"#;

const VERDICTS_A: &str = r#"{
  "skeptic": {"stance": "reject", "verdict": "the plan freezes via a second drain", "confidence": 4},
  "chair": {"stance": "approve", "verdict": "the plan is sound", "confidence": 4},
  "broken-seat": {"stance": "none", "verdict": "", "confidence": null},
  "not-an-object": {"stance": "mixed", "verdict": "usable"}
}"#;

#[test]
fn consult_dir_pattern() {
    assert!(is_consult_dir("20260828-040422-caddis-worker-bees"));
    assert!(is_consult_dir("20260704-091043-toolkit-smoke"));
    assert!(!is_consult_dir("README.md"));
    assert!(!is_consult_dir("20260828-council"));
    assert!(!is_consult_dir("2026082804042-council-x"));
    assert!(!is_consult_dir("x0260828-040422-council"));
}

#[test]
fn manifest_parses_transport_identity_verbatim() {
    let (seats, no_id) = parse_manifest(MANIFEST_A).unwrap();
    assert_eq!(no_id, 2, "broken-seat (no provider/model) + not-an-object");
    assert_eq!(seats.len(), 2);
    let sk = &seats[0];
    assert_eq!(sk.seat, "skeptic");
    assert_eq!(sk.lane_id, "grok-coding/grok-4.6");
    assert_eq!(sk.model, "grok-4.6");
    assert_eq!(
        seats[1].lane_id,
        "openai-compat/nvidia/nemotron-3-super-120b-a12b"
    );
}

#[test]
fn verdicts_parse_stance_and_text() {
    let v = parse_verdicts(VERDICTS_A).unwrap();
    assert_eq!(
        v.get("skeptic").unwrap(),
        &(
            "reject".to_string(),
            "the plan freezes via a second drain".to_string()
        )
    );
    // missing fields read as empty — the outcome mapping turns that into a
    // fail, never a crash
    assert_eq!(
        parse_verdicts(r#"{"s": {"stance": "approve"}}"#)
            .unwrap()
            .get("s")
            .unwrap(),
        &("approve".to_string(), String::new())
    );
}

#[test]
fn split_members_handles_escapes_and_unicode() {
    let text = r#"{"aé": {"x": "line\nbreak \"quoted\" \u0041"}, "b": [1, {"deep": true}]}"#;
    let m = split_members(text).unwrap();
    assert_eq!(m.len(), 2);
    assert_eq!(m[0].0, "aé");
    assert!(m[0].1.starts_with('{'));
    assert_eq!(m[1].1, "[1, {\"deep\": true}]");
    let leaf = leaf_members(&m[0].1).unwrap();
    assert_eq!(str_val(&leaf, "x").unwrap(), "line\nbreak \"quoted\" A");
}

#[test]
fn collect_replays_consults_with_outcome_mapping() {
    let root = tmp("map");
    consult(
        &root,
        "20260801-010101-alpha",
        Some(MANIFEST_A),
        Some(VERDICTS_A),
    );
    let ledger = Ledger::new(root.join("ledger.jsonl"));
    let rep = collect_councils(&root, &ledger, false).unwrap();

    // 4 manifest entries: 2 with identity; broken-seat has identity? no —
    // broken-seat lacks provider/model (counted no-identity), so rows come
    // from skeptic + chair + not-an-object (identity-less: NOT a row).
    assert_eq!(rep.consults_seen, 1);
    assert_eq!(rep.skipped_seat_no_identity, 2);
    assert_eq!(rep.rows, 2);
    assert_eq!(
        rep.passes, 2,
        "reject-with-text and approve-with-text both pass"
    );
    assert_eq!(rep.fails, 0);

    // Wire roundtrip: the appended rows read back through the ledger's own
    // parser with the transport identity intact.
    let loaded = ledger.load().unwrap();
    let mut lanes = BTreeSet::new();
    for pr in &loaded.rows {
        if let Row::Outcome(o) = &pr.row {
            assert_eq!(o.task_class, TASK_CLASS_CONSULT);
            assert_eq!(o.card_id, "council/20260801-010101-alpha");
            assert_eq!(o.cost_tokens, 0);
            assert_eq!(o.latency_ms, 0);
            lanes.insert(o.lane_id.clone());
        }
    }
    assert_eq!(
        lanes,
        BTreeSet::from([
            "grok-coding/grok-4.6".to_string(),
            "openai-compat/nvidia/nemotron-3-super-120b-a12b".to_string(),
        ])
    );
}

#[test]
fn stance_none_and_empty_verdict_are_fails() {
    let root = tmp("failmap");
    let manifest = r#"{
  "a": {"provider": "p1", "model": "m1"},
  "b": {"provider": "p1", "model": "m2"},
  "c": {"provider": "p1", "model": "m3"}
}"#;
    let verdicts = r#"{
  "a": {"stance": "none", "verdict": "", "confidence": null},
  "b": {"stance": "approve", "verdict": ""},
  "c": {"stance": "none", "verdict": "parsed but stance none"}
}"#;
    consult(
        &root,
        "20260801-020202-beta",
        Some(manifest),
        Some(verdicts),
    );
    let ledger = Ledger::new(root.join("ledger.jsonl"));
    let rep = collect_councils(&root, &ledger, false).unwrap();
    assert_eq!(rep.rows, 3);
    assert_eq!(rep.passes, 0);
    assert_eq!(rep.fails, 3, "none/empty/approve-without-text all fail");
}

#[test]
fn rerun_is_a_noop_by_card_and_lane() {
    let root = tmp("idem");
    consult(
        &root,
        "20260801-030303-gamma",
        Some(MANIFEST_A),
        Some(VERDICTS_A),
    );
    let ledger = Ledger::new(root.join("ledger.jsonl"));
    let first = collect_councils(&root, &ledger, false).unwrap();
    assert_eq!(first.rows, 2);
    let second = collect_councils(&root, &ledger, false).unwrap();
    assert_eq!(second.rows, 0);
    assert_eq!(second.skipped_already, 2);
    // stream unchanged
    let loaded = ledger.load().unwrap();
    assert_eq!(loaded.rows.len(), 2);
    assert_eq!(loaded.bad.len(), 0);
}

#[test]
fn same_lane_twice_in_one_panel_keeps_both_samples() {
    let root = tmp("samelane");
    let manifest = r#"{
  "x": {"provider": "pi", "model": "ollama-cloud/gpt-oss:20b"},
  "y": {"provider": "pi", "model": "ollama-cloud/gpt-oss:20b"}
}"#;
    let verdicts = r#"{
  "x": {"stance": "approve", "verdict": "one"},
  "y": {"stance": "mixed", "verdict": "two"}
}"#;
    consult(
        &root,
        "20260801-040404-delta",
        Some(manifest),
        Some(verdicts),
    );
    let ledger = Ledger::new(root.join("ledger.jsonl"));
    let rep = collect_councils(&root, &ledger, false).unwrap();
    assert_eq!(
        rep.rows, 2,
        "within one run both same-lane seats are samples"
    );
    let again = collect_councils(&root, &ledger, false).unwrap();
    assert_eq!(again.rows, 0);
    assert_eq!(again.skipped_already, 2);
}

#[test]
fn missing_and_bad_records_are_counted_skips() {
    let root = tmp("skips");
    consult(&root, "20260801-050505-a", None, Some(VERDICTS_A)); // no manifest
    consult(
        &root,
        "20260801-060606-b",
        Some("{not json"),
        Some(VERDICTS_A),
    ); // bad manifest
    consult(&root, "20260801-070707-c", Some(MANIFEST_A), None); // no verdicts
    consult(&root, "20260801-080808-d", Some(MANIFEST_A), Some("{oops")); // bad verdicts
                                                                          // not consult-shaped: ignored entirely
    fs::create_dir_all(root.join("scratch-dir")).unwrap();
    fs::write(root.join("loose-file.txt"), "x").unwrap();

    let ledger = Ledger::new(root.join("ledger.jsonl"));
    let rep = collect_councils(&root, &ledger, false).unwrap();
    assert_eq!(rep.consults_seen, 4);
    assert_eq!(rep.skipped_no_manifest, 1);
    assert_eq!(rep.skipped_manifest_bad, 1);
    assert_eq!(rep.skipped_no_verdicts, 1);
    assert_eq!(rep.skipped_verdicts_bad, 1);
    assert_eq!(rep.rows, 0);
}

#[test]
fn seat_missing_from_verdicts_is_counted_not_guessed() {
    let root = tmp("noseat");
    let manifest = r#"{
  "a": {"provider": "p", "model": "m"},
  "b": {"provider": "p", "model": "m2"}
}"#;
    let verdicts = r#"{"a": {"stance": "approve", "verdict": "yes"}}"#;
    consult(&root, "20260801-090909-eps", Some(manifest), Some(verdicts));
    let ledger = Ledger::new(root.join("ledger.jsonl"));
    let rep = collect_councils(&root, &ledger, false).unwrap();
    assert_eq!(rep.rows, 1);
    assert_eq!(
        rep.skipped_seat_no_verdict, 1,
        "no verdict entry = no row, counted"
    );
}

#[test]
fn dry_run_writes_nothing() {
    let root = tmp("dry");
    consult(
        &root,
        "20260801-101010-zeta",
        Some(MANIFEST_A),
        Some(VERDICTS_A),
    );
    let lpath = root.join("ledger.jsonl");
    let rep = collect_councils(&root, &Ledger::new(&lpath), true).unwrap();
    assert!(rep.dry_run);
    assert_eq!(rep.rows, 2, "dry-run still reports what WOULD land");
    assert!(!lpath.exists(), "no ledger file materializes");
}

#[test]
fn chronology_is_dir_name_order_not_fs_order() {
    let root = tmp("chrono");
    let mk = |name: &str, model: &str| {
        let manifest = format!(r#"{{"s": {{"provider": "p", "model": "{model}"}}}}"#);
        let verdicts = r#"{"s": {"stance": "approve", "verdict": "ok"}}"#;
        consult(&root, name, Some(&manifest), Some(verdicts));
    };
    // created in REVERSE chronological order on purpose
    mk("20260802-000002-late", "late-model");
    mk("20260801-000001-early", "early-model");
    let ledger = Ledger::new(root.join("ledger.jsonl"));
    collect_councils(&root, &ledger, false).unwrap();
    let loaded = ledger.load().unwrap();
    let models: Vec<String> = loaded
        .rows
        .iter()
        .filter_map(|pr| match &pr.row {
            Row::Outcome(o) => Some(o.model.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        models,
        ["early-model".to_string(), "late-model".to_string()]
    );
}

#[test]
fn missing_councils_dir_is_a_hard_io_error() {
    let root = tmp("nodir");
    let err = collect_councils(
        &root.join("absent"),
        &Ledger::new(root.join("l.jsonl")),
        false,
    );
    assert!(err.is_err());
}
// ---------------------------------------------------------------------------
// Bee collector (slice 3a)
// ---------------------------------------------------------------------------

const CARDS_A: &str = r#"{
  "law": "fixture",
  "cards": [
    {"id":"bee-alpha","status":"done","assigned":"KAMANĖ","steps":["a","b"],"note":"x"},
    {"id":"bee-beta","status":"done","assigned":"glm-5.2"},
    {"id":"bee-gamma","status":"done","assigned":"bee"},
    {"id":"bee-delta","status":"done","assigned":"BITUTE"},
    {"id":"bee-eps","status":"done","assigned":"2026-08-25T18:07:37Z"},
    {"id":"bee-zeta","status":"done","assigned":"glm-4.7-flash"},
    {"id":"bee-eta","status":"done","assigned":""},
    {"id":"bee-blocked","status":"blocked-sergeant","assigned":"KAMANĖ"},
    {"id":"bee-open","status":"assigned","assigned":"KAMANĖ"},
    {"id":"","status":"done","assigned":"KAMANĖ"}
  ]
}"#;

#[test]
fn bee_lane_ladder_resolves_only_registry_identity() {
    // bee / model / loop spellings all resolve; everything else is None.
    assert_eq!(
        resolve_bee_lane("bee2").unwrap().lane_id,
        "ollama/llama3.2:3b-64k",
        "bee2 is bitute's loop"
    );
    assert_eq!(resolve_bee_lane("kamane").unwrap().lane_id, "zai/glm-5.2");
    assert_eq!(resolve_bee_lane("glm-5.2").unwrap().lane_id, "zai/glm-5.2");
    assert_eq!(resolve_bee_lane("bee").unwrap().lane_id, "zai/glm-5.2");
    assert_eq!(
        resolve_bee_lane("bitute").unwrap().lane_id,
        "ollama/llama3.2:3b-64k"
    );
    assert_eq!(
        resolve_bee_lane("llama3.2:3b-64k").unwrap().lane_id,
        "ollama/llama3.2:3b-64k"
    );
    // Claim-time quirk, empty, unregistered model: never guessed.
    assert!(resolve_bee_lane("2026-08-25T18:07:37Z").is_none());
    assert!(resolve_bee_lane("").is_none());
    assert!(resolve_bee_lane("glm-4.7-flash").is_none());
}

#[test]
fn bee_collect_maps_done_cards_and_counts_every_skip() {
    let root = tmp("bees");
    let cards = root.join("BEE-CARDS.json");
    fs::write(&cards, CARDS_A).unwrap();
    let ledger = Ledger::new(root.join("ledger.jsonl"));
    let rep = collect_bees(&cards, &ledger, false).unwrap();

    assert_eq!(rep.cards_seen, 10);
    assert_eq!(rep.rows, 4, "alpha+beta+gamma (kamane) + delta (bitute)");
    assert_eq!(rep.passes, 4);
    assert_eq!(rep.skipped_not_done, 2, "blocked-sergeant + assigned");
    assert_eq!(rep.skipped_no_id, 1);
    assert_eq!(
        rep.skipped_no_lane, 3,
        "ts-quirk + unregistered model + empty"
    );
    assert_eq!(rep.skipped_already, 0);

    // Wire roundtrip: identity and task class intact.
    let loaded = ledger.load().unwrap();
    let mut lanes = BTreeSet::new();
    let mut ids = BTreeSet::new();
    for pr in &loaded.rows {
        if let Row::Outcome(o) = &pr.row {
            assert_eq!(o.task_class, TASK_CLASS_BEE);
            assert_eq!(o.outcome, Outcome::Pass);
            assert_eq!(o.cost_tokens, 0);
            lanes.insert(o.lane_id.clone());
            ids.insert(o.card_id.clone());
        }
    }
    assert_eq!(
        lanes,
        BTreeSet::from([
            "zai/glm-5.2".to_string(),
            "ollama/llama3.2:3b-64k".to_string()
        ])
    );
    assert_eq!(
        ids,
        BTreeSet::from([
            "bee/bee-alpha".to_string(),
            "bee/bee-beta".to_string(),
            "bee/bee-gamma".to_string(),
            "bee/bee-delta".to_string(),
        ])
    );
}

#[test]
fn bee_rerun_is_a_noop_and_dry_run_writes_nothing() {
    let root = tmp("bees2");
    let cards = root.join("BEE-CARDS.json");
    fs::write(&cards, CARDS_A).unwrap();
    let ledger = Ledger::new(root.join("ledger.jsonl"));

    let dry = collect_bees(&cards, &ledger, true).unwrap();
    assert_eq!(dry.rows, 4);
    assert_eq!(ledger.load().unwrap().rows.len(), 0, "dry-run appends none");

    collect_bees(&cards, &ledger, false).unwrap();
    let again = collect_bees(&cards, &ledger, false).unwrap();
    assert_eq!(again.rows, 0);
    assert_eq!(again.skipped_already, 4);
    assert_eq!(ledger.load().unwrap().rows.len(), 4);
}

#[test]
fn bee_missing_or_malformed_file_is_a_hard_error() {
    let root = tmp("bees3");
    let ledger = Ledger::new(root.join("ledger.jsonl"));
    assert!(collect_bees(&root.join("nope.json"), &ledger, false).is_err());

    let bad = root.join("bad.json");
    fs::write(&bad, "{\"nope\": 1}").unwrap();
    assert!(collect_bees(&bad, &ledger, false).is_err());

    let notarr = root.join("notarr.json");
    fs::write(&notarr, "{\"cards\": 3}").unwrap();
    assert!(collect_bees(&notarr, &ledger, false).is_err());
}

// --- tinyagi trajectory collector (P2R-S3b) ----------------------------------

/// Base timestamp for bracket tests (epoch ms).
const TA_T1: i64 = 1_750_000_000_000;
const TA_T2: i64 = TA_T1 + 10_000_000;
const TA_T3: i64 = TA_T2 + 10_000_000;
const TA_TCUR: i64 = TA_T3 + 10_000_000;

fn tinyagi_home(tag: &str) -> PathBuf {
    let d = tmp(tag).join("home");
    fs::create_dir_all(d.join("trajectories")).unwrap();
    d
}

fn set_mtime(p: &Path, ms: i64) {
    let t = std::time::UNIX_EPOCH + std::time::Duration::from_millis(ms as u64);
    std::fs::File::options()
        .write(true)
        .open(p)
        .unwrap()
        .set_modified(t)
        .unwrap();
}

/// Write one settings-family snapshot with the given agents JSON body.
fn snap(home: &Path, name: &str, agents: &str, mtime_ms: i64) {
    let p = home.join(name);
    fs::write(
        &p,
        format!("{{\"agents\":{agents},\"custom_providers\":{{}}}}"),
    )
    .unwrap();
    set_mtime(&p, mtime_ms);
}

fn run_line(id: &str, agent: &str, created: i64, is_failure: bool, rl: i64) -> String {
    format!(
        "{{\"agent\":\"{agent}\",\"channel\":\"api\",\"created\":{created},\"id\":\"{id}\",\"meta\":{{\"isFailure\":{is_failure},\"responseLength\":{rl}}},\"turns\":[{{\"role\":\"user\",\"content\":\"x\"}}]}}"
    )
}

const R1: &str = "{\"alpha\":{\"provider\":\"prov1\",\"model\":\"m1\"}}";
const R2: &str = "{\"alpha\":{\"provider\":\"prov2\",\"model\":\"m2\"},\"beta\":{\"provider\":\"prov2\",\"model\":\"m2\"}}";
const RC: &str = "{\"alpha\":{\"provider\":\"prov1\",\"model\":\"m1\"},\"beta\":{\"provider\":\"prov2\",\"model\":\"m2\"},\"gamma\":{\"provider\":\"prov3\",\"model\":\"gm3\"}}";

#[test]
fn tinyagi_bracket_provability() {
    let snaps = vec![
        Snapshot {
            mtime_ms: TA_T3,
            roster: parse_roster(&format!("{{\"agents\":{R2}}}")),
            is_current: false,
        },
        Snapshot {
            mtime_ms: TA_T1,
            roster: parse_roster(&format!("{{\"agents\":{R1}}}")),
            is_current: false,
        },
        Snapshot {
            mtime_ms: TA_T2,
            roster: parse_roster("{\"custom_providers\":{}}"), // restore fragment
            is_current: false,
        },
        Snapshot {
            mtime_ms: TA_TCUR,
            roster: parse_roster(&format!("{{\"agents\":{RC}}}")),
            is_current: true,
        },
    ];
    let b = build_brackets(snaps);
    assert_eq!(b.len(), 4);
    // (MIN, T1] roster R1 — provable
    assert_eq!(b[0].start_ms, Some(i64::MIN));
    assert_eq!(b[0].end_ms, TA_T1);
    assert_eq!(b[0].roster.len(), 1);
    // (T1, T2] empty roster (restore window) — provable edge, dark content
    assert_eq!(b[1].start_ms, Some(TA_T1));
    assert_eq!(b[1].end_ms, TA_T2);
    assert!(b[1].roster.is_empty());
    // start NOT provable: previous snapshot carried no roster
    assert_eq!(b[2].start_ms, None);
    assert_eq!(b[2].end_ms, TA_T3);
    // live settings: [TCUR, +inf)
    assert_eq!(b[3].start_ms, Some(TA_TCUR));
    assert_eq!(b[3].end_ms, i64::MAX);
    assert_eq!(b.iter().filter(|x| x.start_ms.is_some()).count(), 3);
}

#[test]
fn tinyagi_collect_rows_and_skips() {
    let home = tinyagi_home("ta1");
    snap(&home, "settings.json.bak-a", R1, TA_T1);
    snap(&home, "settings.json", RC, TA_TCUR);
    let lines = [
        run_line("r_pass", "alpha", TA_T1 - 1000, false, 500), // R1 window -> Pass
        run_line("r_fail", "alpha", TA_T1 - 2000, true, 0),    // R1 window -> Fail
        run_line("r_cur", "gamma", TA_TCUR + 5000, false, 40), // current bracket
        run_line("r_nolane", "ghost", TA_T1 - 3000, false, 10), // agent not in R1
        // no outcome signal: isFailure absent
        "{\"agent\":\"alpha\",\"channel\":\"api\",\"created\":0,\"id\":\"r_noout\",\"meta\":{\"responseLength\":9},\"turns\":[]}".replace(
            "\"created\":0",
            &format!("\"created\":{}", TA_T1 - 4000),
        ),
        "{\"agent\":\"alpha\",\"channel\":\"api\",\"created\":1,\"id\":\"\",\"meta\":{\"isFailure\":false,\"responseLength\":9},\"turns\":[]}".replace(
            "\"created\":1",
            &format!("\"created\":{}", TA_T1 - 5000),
        ), // no id
        "{\"channel\":\"api\",\"created\":2,\"id\":\"r_noagent\",\"meta\":{\"isFailure\":false,\"responseLength\":9},\"turns\":[]}".replace(
            "\"created\":2",
            &format!("\"created\":{}", TA_T1 - 6000),
        ), // no agent
        run_line("r_gap", "alpha", TA_T1 + 5000, false, 10), // between snapshots -> no bracket
        "not json at all".to_string(),                       // torn line
    ];
    fs::write(
        home.join("trajectories").join("runs.jsonl"),
        lines.join("\n") + "\n",
    )
    .unwrap();
    fs::write(
        home.join("trajectories").join("failed.jsonl"),
        [
            run_line("f_row", "alpha", TA_T1 - 7000, true, 0), // provable Fail row
            "{\"channel\":\"api\",\"created\":3,\"id\":\"f_noagent\",\"meta\":{\"isFailure\":true,\"responseLength\":0},\"turns\":[]}".replace(
                "\"created\":3",
                &format!("\"created\":{}", TA_T1 - 8000),
            ),
        ]
        .join("\n")
            + "\n",
    )
    .unwrap();

    let root = tmp("ta1-led");
    let ledger = Ledger::new(root.join("ledger.jsonl"));
    let rep = collect_tinyagi(&home, &ledger, false).unwrap();

    assert_eq!(rep.snapshots_seen, 2);
    assert_eq!(rep.snapshots_roster, 2);
    assert_eq!(rep.brackets_provable, 2);
    assert_eq!(rep.runs_seen, 9);
    assert_eq!(rep.failed_seen, 2);
    assert_eq!(rep.rows, 4);
    assert_eq!(rep.passes, 2);
    assert_eq!(rep.fails, 2);
    assert_eq!(rep.skipped_no_id, 1);
    assert_eq!(rep.skipped_no_agent, 2);
    assert_eq!(rep.skipped_no_bracket, 1);
    assert_eq!(rep.skipped_empty_roster, 0);
    assert_eq!(rep.skipped_no_lane, 1);
    assert_eq!(rep.skipped_no_outcome, 1);
    assert_eq!(rep.skipped_bad_line, 1);
    assert_eq!(rep.skipped_already, 0);

    let rows = &ledger.load().unwrap().rows;
    assert_eq!(rows.len(), 4);
    let lane_of = |cid: &str| {
        rows.iter()
            .find_map(|pr| match &pr.row {
                Row::Outcome(o) if o.card_id == cid => {
                    Some((o.lane_id.clone(), o.model.clone(), o.outcome))
                }
                _ => None,
            })
            .unwrap()
    };
    // R1 window resolved from the BACKUP roster, not the live one.
    assert_eq!(
        lane_of("tinyagi-run/r_pass"),
        ("prov1/m1".into(), "m1".into(), Outcome::Pass)
    );
    assert_eq!(
        lane_of("tinyagi-run/r_fail"),
        ("prov1/m1".into(), "m1".into(), Outcome::Fail)
    );
    assert_eq!(
        lane_of("tinyagi-run/r_cur"),
        ("prov3/gm3".into(), "gm3".into(), Outcome::Pass)
    );
    // failed.jsonl shares the engine: provable identity -> a real Fail row.
    assert_eq!(
        lane_of("tinyagi-run/f_row"),
        ("prov1/m1".into(), "m1".into(), Outcome::Fail)
    );
}

#[test]
fn tinyagi_idempotent() {
    let home = tinyagi_home("ta2");
    snap(&home, "settings.json.bak-a", R1, TA_T1);
    snap(&home, "settings.json", RC, TA_TCUR);
    fs::write(
        home.join("trajectories").join("runs.jsonl"),
        run_line("x1", "alpha", TA_T1 - 1000, false, 10) + "\n",
    )
    .unwrap();
    let root = tmp("ta2-led");
    let ledger = Ledger::new(root.join("ledger.jsonl"));
    let first = collect_tinyagi(&home, &ledger, false).unwrap();
    assert_eq!(first.rows, 1);
    let again = collect_tinyagi(&home, &ledger, false).unwrap();
    assert_eq!(again.rows, 0);
    assert_eq!(again.skipped_already, 1);
    assert_eq!(ledger.load().unwrap().rows.len(), 1);
}

#[test]
fn tinyagi_restore_dark_zone() {
    let home = tinyagi_home("ta3");
    snap(&home, "settings.json.bak-a", R1, TA_T1);
    // restore fragment FIRST (dark change point), full backup AFTER it:
    // the fragment's window carries no roster, and the R2 window that
    // follows loses its provable start.
    let p = home.join("settings.json.bak-frag");
    fs::write(&p, "{\"custom_providers\":{}}").unwrap();
    set_mtime(&p, TA_T2);
    snap(&home, "settings.json.bak-mid", R2, TA_T3);
    snap(&home, "settings.json", RC, TA_TCUR);
    fs::write(
        home.join("trajectories").join("runs.jsonl"),
        [
            run_line("ok", "alpha", TA_T1 - 1000, false, 5), // provable R1
            run_line("dark", "alpha", TA_T1 + 5000, false, 5), // empty-roster window
            run_line("gap", "alpha", TA_T2 + 2_000_000, false, 5), // unprovable start
        ]
        .join("\n")
            + "\n",
    )
    .unwrap();
    let root = tmp("ta3-led");
    let ledger = Ledger::new(root.join("ledger.jsonl"));
    let rep = collect_tinyagi(&home, &ledger, false).unwrap();
    assert_eq!(rep.snapshots_seen, 4);
    assert_eq!(rep.snapshots_roster, 3);
    assert_eq!(rep.brackets_provable, 3);
    assert_eq!(rep.rows, 1);
    assert_eq!(rep.skipped_empty_roster, 1);
    assert_eq!(rep.skipped_no_bracket, 1);
}

#[test]
fn tinyagi_missing_trail_is_clean() {
    let home = tinyagi_home("ta4");
    snap(&home, "settings.json", RC, TA_TCUR);
    let root = tmp("ta4-led");
    let rep = collect_tinyagi(&home, &Ledger::new(root.join("ledger.jsonl")), true).unwrap();
    assert_eq!(rep.runs_seen, 0);
    assert_eq!(rep.failed_seen, 0);
    assert_eq!(rep.rows, 0);
    assert!(rep.dry_run);
}

#[test]
fn tinyagi_roster_parse_degrades() {
    // extra members inside an agent config are ignored; non-object and
    // identity-less entries drop; a missing agents object = empty map.
    let text = "{\"agents\":{\"a\":{\"provider\":\"p\",\"model\":\"m\",\"harness\":\"x\"},\"b\":7,\"c\":{\"provider\":\"\",\"model\":\"m\"}},\"custom_providers\":{}}";
    let r = parse_roster(text);
    assert_eq!(r.len(), 1);
    assert_eq!(r.get("a").unwrap(), &("p".to_string(), "m".to_string()));
    assert!(parse_roster("{\"custom_providers\":{}}").is_empty());
    assert!(parse_roster("not json").is_empty());
}
