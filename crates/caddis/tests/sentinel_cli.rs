//! sentinel_cli.rs — CARD-0331/0332. The audit organ in Rust: argument-
//! compatible with the bee's sentinel call, slot-compatible with the
//! push gate, fail-closed (a failed audit never destroys a prior
//! verdict). Hermetic: stub grok, stub sentinel home, temp repo + runs.

mod sentinel_common;
use sentinel_common::*;

#[test]
fn clear_report_writes_pass_slot() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let w = World::new("clear", CLEAR_ENV);
    let (o, e, c) = w.run(&[
        "sentinel",
        "audit",
        "--target",
        "lib.rs",
        "audit this patch",
    ]);
    assert_eq!(c, 0, "audit: {o}{e}");
    let slot = fs::read_to_string(w.slot()).expect("slot written");
    assert!(
        slot.contains("\"verdict\":\"pass\""),
        "pass verdict: {slot}"
    );
    assert!(
        slot.contains("\"audit_file\""),
        "cites its evidence: {slot}"
    );
    let head = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&w.repo)
        .output()
        .unwrap();
    let sha = String::from_utf8_lossy(&head.stdout).trim().to_string();
    assert!(slot.contains(&sha), "slot pins the exact head: {slot}");
    let audit_path = slot
        .split("\"audit_file\":\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();
    let audit = fs::read_to_string(audit_path.replace('\\', "/")).expect("evidence on disk");
    assert!(audit.contains("CLEAR"), "evidence says CLEAR: {audit}");
}

#[test]
fn findings_report_writes_fail_slot() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let w = World::new("find", FINDINGS_ENV);
    let (o, e, c) = w.run(&[
        "sentinel",
        "audit",
        "--target",
        "lib.rs",
        "audit this patch",
    ]);
    assert_eq!(c, 0, "the audit itself succeeded: {o}{e}");
    let slot = fs::read_to_string(w.slot()).expect("slot written");
    assert!(
        slot.contains("\"verdict\":\"fail\""),
        "findings fail: {slot}"
    );
}

#[test]
fn broken_engine_writes_nothing() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let w = World::new("broken", "{\"type\":\"error\",\"message\":\"boom\"}");
    let (o, e, c) = w.run(&[
        "sentinel",
        "audit",
        "--target",
        "lib.rs",
        "audit this patch",
    ]);
    assert_ne!(c, 0, "engine error must fail: {o}{e}");
    assert!(
        !w.slot().exists(),
        "a failed audit NEVER destroys a prior verdict"
    );
}

#[test]
fn non_audit_mode_refuses_by_name() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let w = World::new("mode", CLEAR_ENV);
    let (o, e, c) = w.run(&["sentinel", "patrol", "sweep"]);
    assert_ne!(c, 0, "refused: {o}{e}");
    assert!(
        format!("{o}{e}").contains("audit"),
        "names the v1 mode: {o}{e}"
    );
}

#[test]
fn timeout_flag_parsed_inert() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let w = World::new("inert", CLEAR_ENV);
    let (o, e, c) = w.run(&[
        "sentinel",
        "audit",
        "--timeout",
        "5",
        "--target",
        "lib.rs",
        "audit this patch",
    ]);
    assert_eq!(c, 0, "parsed, never enforced on the agent: {o}{e}");
    assert!(w.slot().is_file(), "the verdict landed");
}

#[test]
fn model_set_persists_and_reads_back() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let w = World::new("model", CLEAR_ENV);
    let (o, e, c) = w.run(&["sentinel", "model", "--set", "grok-4.7-fast"]);
    assert_eq!(c, 0, "set: {o}{e}");
    let state = fs::read_to_string(w.home.join(".caddis/sentinel.json")).expect("state written");
    assert!(
        state.contains("\"model\":\"grok-4.7-fast\""),
        "persisted: {state}"
    );
    let (o, e, c) = w.run(&["sentinel", "model"]);
    assert_eq!(c, 0, "show: {o}{e}");
    assert!(o.contains("grok-4.7-fast"), "reads back: {o}{e}");
}

#[test]
fn audit_updates_the_warden_room_state() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let w = World::new("state", CLEAR_ENV);
    let (o, e, c) = w.run(&[
        "sentinel",
        "audit",
        "--target",
        "lib.rs",
        "audit this patch",
    ]);
    assert_eq!(c, 0, "audit: {o}{e}");
    let state = fs::read_to_string(w.home.join(".caddis/sentinel.json")).expect("state written");
    assert!(
        state.contains("\"model\":\"grok-4.6\""),
        "default model: {state}"
    );
    assert!(
        state.contains("\"verdict\":\"CLEAR\""),
        "last audit truth: {state}"
    );
    assert!(
        state.contains("last-verify-caddis-workshop"),
        "names its slot: {state}"
    );
}

#[test]
fn text_fallback_parses_the_real_grok_envelope() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let envelope = "{\"text\":\"analysis prose... {\\\"verdict\\\":\\\"CLEAR\\\",\\\"summary\\\":\\\"clean\\\",\\\"findings\\\":[],\\\"cannot_verify\\\":[]}\",\"stopReason\":\"end_turn\"}";
    let w = World::new("textfb", envelope);
    let (o, e, c) = w.run(&[
        "sentinel",
        "audit",
        "--target",
        "lib.rs",
        "audit this patch",
    ]);
    assert_eq!(c, 0, "audit: {o}{e}");
    let slot = fs::read_to_string(w.slot()).expect("slot written");
    assert!(
        slot.contains("\"verdict\":\"pass\""),
        "text-shape verdict parsed: {slot}"
    );
}

/// CARD-0332 row 1: a flag VALUE that is itself a mode word must never
/// reroute the mode — the live audit's finding 0.
#[test]
fn mode_word_value_stays_audit() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let w = World::new("modeval", CLEAR_ENV);
    let (o, e, c) = w.run(&["sentinel", "audit", "--target", "verify", "probe"]);
    assert_eq!(
        c, 0,
        "the VALUE 'verify' must not overwrite the mode: {o}{e}"
    );
    assert!(w.slot().is_file(), "the audit ran in audit mode");
}

/// CARD-0332 row 2: `model --set` preserves the last-audit truth.
#[test]
fn model_set_keeps_last() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let w = World::new("keeplast", CLEAR_ENV);
    w.run(&[
        "sentinel",
        "audit",
        "--target",
        "lib.rs",
        "audit this patch",
    ]);
    let (o, e, c) = w.run(&["sentinel", "model", "--set", "grok-4.7-fast"]);
    assert_eq!(c, 0, "set: {o}{e}");
    let state = fs::read_to_string(w.home.join(".caddis/sentinel.json")).expect("state");
    assert!(
        state.contains("\"model\":\"grok-4.7-fast\""),
        "new model: {state}"
    );
    assert!(
        state.contains("\"verdict\":\"CLEAR\""),
        "last audit KEPT: {state}"
    );
}

/// CARD-0332 evidence row: findings count is TOP-LEVEL objects.
#[test]
fn findings_count_is_top_level() {
    let env = "{\"text\":\"{\\\"verdict\\\":\\\"FINDINGS\\\",\\\"findings\\\":[{\\\"severity\\\":\\\"high\\\",\\\"meta\\\":{\\\"a\\\":1,\\\"b\\\":{\\\"c\\\":2}}}],\\\"cannot_verify\\\":[]}\",\"stopReason\":\"end_turn\"}";
    let w = World::new("count", env);
    let (o, e, c) = w.run(&["sentinel", "audit", "--target", "lib.rs", "probe"]);
    assert_eq!(c, 0, "audit: {o}{e}");
    assert!(
        o.contains("findings=1"),
        "ONE finding, nested braces not counted: {o}{e}"
    );
    let slot = fs::read_to_string(w.slot()).expect("slot written");
    assert!(
        slot.contains("\"verdict\":\"fail\""),
        "a finding fails: {slot}"
    );
}
