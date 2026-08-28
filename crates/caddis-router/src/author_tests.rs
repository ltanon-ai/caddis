//! author_tests.rs — P5 phase (a) gates (brief §9): encode_registry
//! round-trips the corpus; self-validation refuses malformed candidates;
//! crash order (bak → rename → journal) each proven; the refusal family;
//! journal seq law (max+1 over PARSED rows, never the line count).

use std::fs;
use std::path::PathBuf;

use crate::lane::LaneTier;
use crate::{
    author_commit, author_prepare, journal_load, AuthorErr, AuthorOp, JournalRow, LaneEntry,
    LaneRegistry,
};

fn tmpdir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("rtr-author-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

fn lane(id: &str, family: &str, tier: LaneTier, cost: f64) -> LaneEntry {
    LaneEntry {
        id: id.to_string(),
        family: family.to_string(),
        tier,
        cost_per_task_usd: cost,
    }
}

fn upsert(id: &str, family: &str, tier: LaneTier, cost: f64) -> AuthorOp {
    AuthorOp::LanesUpsert {
        id: id.to_string(),
        family: family.to_string(),
        tier,
        cost,
    }
}

// --- encode_registry round-trip (the audit==obey law for lanes) -------------

fn assert_round_trip(text: &str) {
    let reg = crate::parse_registry(text).unwrap();
    let enc = crate::encode_registry(&reg);
    let back = crate::parse_registry(&enc).unwrap();
    assert_eq!(back, reg, "round-trip broke on:\n{text}\n->\n{enc}");
}

#[test]
fn encode_round_trips_multi_lane_corpus() {
    // The registry test corpus shapes: several lanes, mixed tiers, costs
    // across magnitudes (shortest-round-trip f64 display included).
    assert_round_trip(
        "{\"id\":\"gemini\",\"family\":\"google\",\"tier\":\"free\",\"cost_per_task_usd\":0}\n\
         {\"id\":\"groq\",\"family\":\"groq\",\"tier\":\"mid\",\"cost_per_task_usd\":0.0004}\n\
         {\"id\":\"local-qwen\",\"family\":\"ollama\",\"tier\":\"local\",\"cost_per_task_usd\":0}\n\
         {\"id\":\"opus\",\"family\":\"anthropic\",\"tier\":\"premium\",\"cost_per_task_usd\":12.5}\n",
    );
}

#[test]
fn encode_round_trips_escape_heavy_ids() {
    // Free text through the two-character escaping discipline: quotes,
    // backslashes, control-adjacent punctuation, non-ASCII.
    assert_round_trip(
        "{\"id\":\"a\\\"b\",\"family\":\"f\\\\g\",\"tier\":\"free\",\"cost_per_task_usd\":1}\n\
         {\"id\":\"kämanė-ė\",\"family\":\"šeima-ų\",\"tier\":\"local\",\"cost_per_task_usd\":0.5}\n",
    );
}

#[test]
fn encode_is_the_only_shape_the_loader_accepts() {
    // What the author WRITES must be the canonical wire form: fixed key
    // order, sorted ids, one object per line.
    let reg = crate::parse_registry(
        "{\"id\":\"zz\",\"family\":\"z\",\"tier\":\"free\",\"cost_per_task_usd\":1}\n\
         {\"id\":\"aa\",\"family\":\"a\",\"tier\":\"local\",\"cost_per_task_usd\":0}\n",
    )
    .unwrap();
    let enc = crate::encode_registry(&reg);
    let lines: Vec<&str> = enc.lines().collect();
    assert_eq!(lines.len(), 2);
    assert!(
        lines[0].starts_with("{\"id\":\"aa\","),
        "sorted by id: {enc}"
    );
    assert!(lines[0].contains("\"family\":\"a\",\"tier\":\"local\",\"cost_per_task_usd\":0"));
    assert!(
        enc.ends_with('\n'),
        "trailing newline frames the last append"
    );
}

#[test]
fn registry_upsert_remove_semantics() {
    let mut reg = crate::parse_registry(
        "{\"id\":\"a\",\"family\":\"x\",\"tier\":\"free\",\"cost_per_task_usd\":1}\n\
         {\"id\":\"b\",\"family\":\"y\",\"tier\":\"mid\",\"cost_per_task_usd\":2}\n",
    )
    .unwrap();
    assert!(!reg.upsert(lane("c", "z", LaneTier::Local, 0.0)), "added");
    assert!(
        reg.upsert(lane("a", "x2", LaneTier::Premium, 9.0)),
        "replaced"
    );
    let re = crate::parse_registry(&crate::encode_registry(&reg)).unwrap();
    let a = re.entries().iter().find(|e| e.id == "a").unwrap();
    assert_eq!(a.family, "x2");
    assert_eq!(a.tier, LaneTier::Premium);
    assert_eq!(a.cost_per_task_usd, 9.0);
    assert!(re.entries().iter().any(|e| e.id == "c"));
    assert!(reg.remove("b"));
    assert!(!reg.remove("b"), "second remove: absent");
    assert!(LaneRegistry::from_entries(vec![]).is_none());
}

// --- prepare: the dry-run IS steps 1-6 ---------------------------------------

#[test]
fn prepare_upsert_on_missing_file_is_the_empty_ruling() {
    let d = tmpdir("prep-missing");
    let target = d.join("lanes.jsonl");
    let plan = author_prepare(
        upsert("gemini", "google", LaneTier::Free, 0.0),
        &target,
        None,
    )
    .expect("prepare ok");
    assert_eq!(plan.prior_hash, None);
    assert_eq!(plan.bak, None);
    assert!(plan.candidate.contains("\"id\":\"gemini\""));
    assert_eq!(plan.journal, d.join("author.jsonl"));
    assert!(plan.summary.contains("added lane gemini"));
    // Nothing written by a prepare.
    assert!(!target.exists());
    assert!(!plan.journal.exists());
}

#[test]
fn prepare_refuses_stale_expect_prior() {
    let d = tmpdir("stale");
    let target = d.join("lanes.jsonl");
    fs::write(
        &target,
        "{\"id\":\"a\",\"family\":\"f\",\"tier\":\"free\",\"cost_per_task_usd\":1}\n",
    )
    .unwrap();
    let err = author_prepare(
        upsert("b", "g", LaneTier::Local, 0.0),
        &target,
        Some("deadbeef"),
    )
    .unwrap_err();
    assert!(err.is_refusal(), "stale proposal is a refusal: {err:?}");
    assert!(err.message().contains("stale proposal"));
    // The honest hash16 passes (case-insensitive).
    let plan = author_prepare(upsert("b", "g", LaneTier::Local, 0.0), &target, None).unwrap();
    let h16 = &plan.prior_hash.clone().unwrap()[..16];
    author_prepare(
        upsert("b", "g", LaneTier::Local, 0.0),
        &target,
        Some(&h16.to_uppercase()),
    )
    .expect("matching prior commits");
}

#[test]
fn prepare_expect_prior_absent_sentinel() {
    let d = tmpdir("absent");
    let target = d.join("lanes.jsonl");
    author_prepare(
        upsert("a", "f", LaneTier::Free, 0.0),
        &target,
        Some("absent"),
    )
    .expect("absent sentinel matches a missing file");
    let err = author_prepare(
        upsert("a", "f", LaneTier::Free, 0.0),
        &target,
        Some("00112233"),
    )
    .unwrap_err();
    assert!(err.is_refusal());
}

#[test]
fn prepare_noop_refusal_after_commit() {
    let d = tmpdir("noop");
    let target = d.join("lanes.jsonl");
    let op = upsert("a", "f", LaneTier::Free, 1.0);
    let plan = author_prepare(op.clone(), &target, None).unwrap();
    author_commit(&plan, "test", "terminal").unwrap();
    let err = author_prepare(op, &target, None).unwrap_err();
    assert!(err.is_refusal(), "{err:?}");
    assert!(err.message().contains("no-op"));
}

#[test]
fn prepare_refuses_last_lane_remove() {
    let d = tmpdir("last-lane");
    let target = d.join("lanes.jsonl");
    fs::write(
        &target,
        "{\"id\":\"a\",\"family\":\"f\",\"tier\":\"free\",\"cost_per_task_usd\":1}\n",
    )
    .unwrap();
    let err = author_prepare(AuthorOp::LanesRemove { id: "a".into() }, &target, None).unwrap_err();
    assert!(err.is_refusal());
    assert!(err.message().contains("LAST lane"));
}

#[test]
fn prepare_refuses_remove_absent_lane() {
    let d = tmpdir("rm-absent");
    let target = d.join("lanes.jsonl");
    fs::write(
        &target,
        "{\"id\":\"a\",\"family\":\"f\",\"tier\":\"free\",\"cost_per_task_usd\":1}\n",
    )
    .unwrap();
    let err = author_prepare(AuthorOp::LanesRemove { id: "zz".into() }, &target, None).unwrap_err();
    assert!(err.is_refusal());
    assert!(err.message().contains("nothing to remove"));
}

#[test]
fn prepare_never_writes_past_a_malformed_current_file() {
    for (name, content) in [
        (
            "lanes.jsonl",
            "{\"id\":\"a\",\"family\":\"f\",\"tier\":\"droid\",\"cost_per_task_usd\":1}\n",
        ),
        ("policy.json", "{\"floor.chair\":0.8,\"unknown.key\":1}\n"),
    ] {
        let d = tmpdir(&format!("malformed-{name}"));
        let target = d.join(name);
        fs::write(&target, content).unwrap();
        let op = if name.starts_with("lanes") {
            upsert("b", "g", LaneTier::Local, 0.0)
        } else {
            AuthorOp::policy_set("floor.skeptic", "0.9").unwrap()
        };
        let err = author_prepare(op, &target, None).unwrap_err();
        assert!(
            !err.is_refusal(),
            "malformed current file is a DEFECT: {err:?}"
        );
        assert!(
            err.message().contains("never writes past a defect"),
            "{err:?}"
        );
    }
}

// --- policy: file-is-whole-policy law ---------------------------------------

#[test]
fn policy_first_set_writes_exactly_the_ruled_keys() {
    let d = tmpdir("first-policy");
    let target = d.join("policy.json");
    let plan = author_prepare(
        AuthorOp::policy_set("tier.secret", "local").unwrap(),
        &target,
        None,
    )
    .unwrap();
    // The candidate rules tier.secret; min_samples is the one converged
    // constant an omitted ruling keeps; NO builtin floors/ceilings seeded.
    assert!(plan.candidate.contains("\"tier.secret\":\"local\""));
    assert!(plan.candidate.contains("\"min_samples\""));
    assert!(!plan.candidate.contains("floor."));
    assert!(!plan.candidate.contains("ceiling."));
    assert!(!plan.candidate.contains("tier.public"));
    assert_eq!(plan.prior_hash, None);
    // It loads (self-validation ran inside prepare).
    crate::parse_policy(&plan.candidate).unwrap();
}

#[test]
fn policy_first_set_without_a_tier_is_refused_by_self_validation() {
    // A file that rules no data class rules nothing routable — the
    // re-parse gate refuses the candidate BEFORE it can exist on disk.
    let d = tmpdir("first-floor");
    let target = d.join("policy.json");
    let err = author_prepare(
        AuthorOp::policy_set("floor.chair", "0.8").unwrap(),
        &target,
        None,
    )
    .unwrap_err();
    assert!(!err.is_refusal(), "{err:?}");
    assert!(
        err.message().contains("no tier.<data_class> ruled"),
        "self-validation must name the law: {err:?}"
    );
}

#[test]
fn policy_set_floor_invalid_range_is_a_defect() {
    let d = tmpdir("bad-floor");
    let target = d.join("policy.json");
    fs::write(&target, "{\"min_samples\":5,\"tier.secret\":\"local\"}\n").unwrap();
    let err = author_prepare(
        AuthorOp::policy_set("floor.chair", "1.5").unwrap(),
        &target,
        None,
    )
    .unwrap_err();
    assert!(!err.is_refusal());
    assert!(err.message().contains("invalid"), "{err:?}");
}

#[test]
fn policy_value_grammar() {
    assert!(AuthorOp::policy_set("droid.lane", "1").is_err());
    assert!(AuthorOp::policy_set("floor.", "0.5").is_err());
    assert!(AuthorOp::policy_set("floor.chair", "zero").is_err());
    assert!(AuthorOp::policy_set("tier.droid", "local").is_err());
    assert!(AuthorOp::policy_set("tier.secret", "local,droid").is_err());
    assert!(AuthorOp::policy_set("tier.secret", "").is_err());
    assert!(AuthorOp::policy_set("min_samples", "0").is_err());
    assert!(AuthorOp::policy_set("min_samples", "x").is_err());
    assert!(AuthorOp::policy_unset("floor.chair").is_ok());
    assert!(AuthorOp::policy_unset("nope.key").is_err());
}

#[test]
fn policy_unset_laws() {
    let d = tmpdir("unset");
    let target = d.join("policy.json");
    // Bootstrap: tier first, then a floor.
    let p1 = author_prepare(
        AuthorOp::policy_set("tier.secret", "local").unwrap(),
        &target,
        None,
    )
    .unwrap();
    author_commit(&p1, "test", "terminal").unwrap();
    let p2 = author_prepare(
        AuthorOp::policy_set("floor.chair", "0.8").unwrap(),
        &target,
        None,
    )
    .unwrap();
    author_commit(&p2, "test", "terminal").unwrap();

    // Absent key: refusal, not a defect.
    let err = author_prepare(
        AuthorOp::policy_unset("floor.skeptic").unwrap(),
        &target,
        None,
    )
    .unwrap_err();
    assert!(err.is_refusal());
    assert!(err.message().contains("nothing to unset"));

    // Present key: unsets; the ruled tier and min_samples survive.
    let p3 = author_prepare(
        AuthorOp::policy_unset("floor.chair").unwrap(),
        &target,
        None,
    )
    .unwrap();
    let seq = author_commit(&p3, "test", "terminal").unwrap();
    assert_eq!(seq, 3);
    let text = fs::read_to_string(&target).unwrap();
    assert!(!text.contains("floor.chair"));
    assert!(text.contains("tier.secret"));
    assert!(text.contains("min_samples"));
    crate::parse_policy(&text).unwrap();

    // Unsetting the LAST tier is refused by the re-parse gate (a policy
    // file must rule at least one data class).
    let err = author_prepare(
        AuthorOp::policy_unset("tier.secret").unwrap(),
        &target,
        None,
    )
    .unwrap_err();
    assert!(!err.is_refusal());
    assert!(
        err.message().contains("no tier.<data_class> ruled"),
        "{err:?}"
    );
}

#[test]
fn policy_unset_min_samples_equal_to_default_is_a_noop() {
    // Omitted min_samples keeps DEFAULT on the next load — unsetting a
    // default-valued ruling changes nothing, and a non-change is refused.
    let d = tmpdir("ms-noop");
    let target = d.join("policy.json");
    let p1 = author_prepare(
        AuthorOp::policy_set("tier.public", "local,free").unwrap(),
        &target,
        None,
    )
    .unwrap();
    author_commit(&p1, "test", "terminal").unwrap();
    let err = author_prepare(
        AuthorOp::policy_unset("min_samples").unwrap(),
        &target,
        None,
    )
    .unwrap_err();
    assert!(err.is_refusal(), "{err:?}");
    // But unsetting a NON-default ruling is a real change.
    let p2 = author_prepare(
        AuthorOp::policy_set("min_samples", "9").unwrap(),
        &target,
        None,
    )
    .unwrap();
    author_commit(&p2, "test", "terminal").unwrap();
    let p3 = author_prepare(
        AuthorOp::policy_unset("min_samples").unwrap(),
        &target,
        None,
    )
    .unwrap();
    author_commit(&p3, "test", "terminal").unwrap();
    assert!(fs::read_to_string(&target)
        .unwrap()
        .contains("\"min_samples\":5"));
}

// --- commit: crash order + journal ------------------------------------------

#[test]
fn commit_order_bak_then_rename_then_journal() {
    let d = tmpdir("commit");
    let target = d.join("lanes.jsonl");
    let prior = "{\"id\":\"a\",\"family\":\"f\",\"tier\":\"free\",\"cost_per_task_usd\":1}\n";
    fs::write(&target, prior).unwrap();

    let plan = author_prepare(upsert("b", "g", LaneTier::Local, 0.0), &target, None).unwrap();
    let h16 = plan.prior_hash.as_ref().unwrap()[..16].to_string();
    let bak_name = format!("lanes.jsonl.bak.{h16}");
    assert_eq!(plan.bak.as_deref(), Some(bak_name.as_str()));

    let seq = author_commit(&plan, "sergeant", "terminal").unwrap();
    assert_eq!(seq, 1);

    // Rename landed: the file holds the candidate, old id + new id present.
    let after = fs::read_to_string(&target).unwrap();
    assert!(after.contains("\"id\":\"a\"") && after.contains("\"id\":\"b\""));
    // Bak landed FIRST and holds the PRIOR bytes, immutable.
    let bak = fs::read_to_string(d.join(&bak_name)).unwrap();
    assert_eq!(bak, prior);
    // Journal row landed LAST with matching hashes.
    let j = journal_load(&d.join("author.jsonl"));
    assert_eq!(j.bad.len(), 0);
    assert_eq!(j.rows.len(), 1);
    let r = &j.rows[0];
    assert_eq!(r.seq, 1);
    assert_eq!(r.actor, "sergeant");
    assert_eq!(r.actor_kind, "terminal");
    assert_eq!(r.op, "lanes-upsert");
    assert_eq!(r.target, "lanes.jsonl");
    assert_eq!(r.prior_hash, plan.prior_hash);
    assert_eq!(r.next_hash, plan.next_hash);
    assert_eq!(r.bak.as_deref(), Some(bak_name.as_str()));
    // The tmp file is gone (rename moved it).
    assert!(!d.join("lanes.jsonl.tmp").exists());
}

#[test]
fn commit_journal_seq_is_monotonic() {
    let d = tmpdir("seq");
    let target = d.join("lanes.jsonl");
    for id in ["a", "b", "c"] {
        let plan = author_prepare(upsert(id, "f", LaneTier::Free, 0.0), &target, None).unwrap();
        let seq = author_commit(&plan, "t", "terminal").unwrap();
        assert!(seq >= 1);
    }
    let j = journal_load(&d.join("author.jsonl"));
    let seqs: Vec<u64> = j.rows.iter().map(|r| r.seq).collect();
    assert_eq!(seqs, vec![1, 2, 3], "seq must be max+1 each append");
}

#[test]
fn journal_seq_survives_a_hand_forked_file() {
    // Model-voice law: seq comes from max over PARSED rows, never the line
    // count — a hand-edited journal with gaps/dups must not re-fork.
    let d = tmpdir("fork");
    let target = d.join("lanes.jsonl");
    fs::write(
        &target,
        "{\"id\":\"a\",\"family\":\"f\",\"tier\":\"free\",\"cost_per_task_usd\":1}\n",
    )
    .unwrap();
    let journal = d.join("author.jsonl");
    fs::write(
        &journal,
        concat!(
            "{\"seq\":1,\"ts\":\"2026-08-28T00:00:00Z\",\"actor\":\"x\",\"actor_kind\":\"terminal\",\"op\":\"lanes-upsert\",\"target\":\"lanes.jsonl\",\"prior_hash\":null,\"next_hash\":\"aa\",\"bak\":null}\n",
            "this line is garbage\n",
            "{\"seq\":7,\"ts\":\"2026-08-28T00:00:01Z\",\"actor\":\"x\",\"actor_kind\":\"terminal\",\"op\":\"lanes-upsert\",\"target\":\"lanes.jsonl\",\"prior_hash\":null,\"next_hash\":\"bb\",\"bak\":null}\n",
        ),
    )
    .unwrap();
    let j = journal_load(&journal);
    assert_eq!(j.rows.len(), 2);
    assert_eq!(j.bad.len(), 1, "garbage line is honest, not hidden");
    let plan = author_prepare(upsert("b", "g", LaneTier::Local, 0.0), &target, None).unwrap();
    let seq = author_commit(&plan, "t", "terminal").unwrap();
    assert_eq!(seq, 8, "max parsed seq is 7 — never the line count");
}

#[test]
fn commit_keeps_an_existing_bak_immutable() {
    // A .bak named by content-hash already exists (an interrupted earlier
    // attempt): create_new fails, the organ keeps the old bytes (provably
    // identical by name), and the write still lands.
    let d = tmpdir("bak-immutable");
    let target = d.join("lanes.jsonl");
    let prior = "{\"id\":\"a\",\"family\":\"f\",\"tier\":\"free\",\"cost_per_task_usd\":1}\n";
    fs::write(&target, prior).unwrap();
    let plan = author_prepare(upsert("b", "g", LaneTier::Local, 0.0), &target, None).unwrap();
    let bak = d.join(plan.bak.as_ref().unwrap());
    fs::write(&bak, "sentinel-from-an-interrupted-attempt\n").unwrap();
    author_commit(&plan, "t", "terminal").unwrap();
    assert_eq!(
        fs::read_to_string(&bak).unwrap(),
        "sentinel-from-an-interrupted-attempt\n",
        "a bak is never overwritten"
    );
    assert!(fs::read_to_string(&target)
        .unwrap()
        .contains("\"id\":\"b\""));
}

#[test]
fn commit_without_prior_writes_no_bak() {
    let d = tmpdir("no-bak");
    let target = d.join("policy.json");
    let plan = author_prepare(
        AuthorOp::policy_set("tier.internal", "local,free,mid").unwrap(),
        &target,
        None,
    )
    .unwrap();
    author_commit(&plan, "t", "terminal").unwrap();
    assert_eq!(
        fs::read_dir(&d).unwrap().count(),
        2,
        "policy.json + author.jsonl only"
    );
    let j = journal_load(&d.join("author.jsonl"));
    assert_eq!(j.rows[0].prior_hash, None);
    assert_eq!(j.rows[0].bak, None);
}

#[test]
fn expect_prior_gates_a_real_commit_race() {
    // The propose→confirm contract: a plan rendered against content X must
    // not commit after the file moved past X — the STALE check happens at
    // prepare, so the panel re-renders instead of blind-writing.
    let d = tmpdir("race");
    let target = d.join("lanes.jsonl");
    let plan = author_prepare(upsert("a", "f", LaneTier::Free, 0.0), &target, None).unwrap();
    // Someone else writes between propose and confirm.
    fs::write(
        &target,
        "{\"id\":\"z\",\"family\":\"z\",\"tier\":\"local\",\"cost_per_task_usd\":0}\n",
    )
    .unwrap();
    let err = author_prepare(plan.op.clone(), &target, Some("absent")).unwrap_err();
    assert!(err.is_refusal());
    assert!(err.message().contains("stale proposal"));
}

#[test]
fn journal_row_encodes_flat_and_escapes() {
    // The wire form is the flat no-nesting subset with the two-character
    // escaping discipline — an actor carrying a quote must survive.
    let d = tmpdir("esc");
    let target = d.join("lanes.jsonl");
    let plan = author_prepare(upsert("a", "f", LaneTier::Free, 0.0), &target, None).unwrap();
    author_commit(&plan, "actor\"quote", "terminal").unwrap();
    let j = journal_load(&d.join("author.jsonl"));
    assert_eq!(j.rows[0].actor, "actor\"quote");
    assert_eq!(j.bad.len(), 0);
}

// --- self-validation gate (direct) -------------------------------------------

#[test]
fn self_validation_is_a_gate_not_an_assumption() {
    // The re-parse law is structural in prepare; prove the LOADER half by
    // feeding the encoder's output shape with an injected defect: parse
    // must refuse what the write path would never emit.
    let injected = "{\"id\":\"a\",\"family\":\"f\",\"tier\":\"droid\",\"cost_per_task_usd\":1}\n";
    assert!(crate::parse_registry(injected).is_err());
    let injected_policy = "{\"floor.chair\":0.8}\n";
    assert!(crate::parse_policy(injected_policy).is_err());
}

#[test]
fn usage_helpers_agree_with_the_op_words() {
    assert_eq!(upsert("a", "f", LaneTier::Free, 0.0).word(), "lanes-upsert");
    assert_eq!(
        AuthorOp::LanesRemove { id: "a".into() }.word(),
        "lanes-remove"
    );
    assert_eq!(
        AuthorOp::policy_set("min_samples", "5").unwrap().word(),
        "policy-set"
    );
    assert_eq!(
        AuthorOp::policy_unset("min_samples").unwrap().word(),
        "policy-unset"
    );
    assert!(AuthorOp::policy_set("tier.secret", "local")
        .unwrap()
        .targets_policy());
    assert!(!upsert("a", "f", LaneTier::Free, 0.0).targets_policy());
}

#[test]
fn journal_row_partial_defaults() {
    let r = JournalRow {
        line: 0,
        seq: 1,
        ts: "2026-08-28T00:00:00Z".into(),
        actor: "t".into(),
        actor_kind: "terminal".into(),
        op: "policy-set".into(),
        target: "policy.json".into(),
        prior_hash: None,
        next_hash: "ab".repeat(32),
        bak: None,
    };
    // The error taxonomy round-trips its words.
    assert!(AuthorErr::Refusal("x".into()).is_refusal());
    assert!(!AuthorErr::Defect("x".into()).is_refusal());
    assert_eq!(r.op, "policy-set");
}
