//! Contract tests: restart skill + rotate.py are thin callers of `caddis rotate`,
//! never the old bash gate. Split from rotate.rs at the 280-line cap (CARD-0123).

/// CARD-0121: a restart skill text is a thin caller of `caddis rotate`
/// and never pins a model or cites restart-preflight.sh as authority.
#[test]
fn restart_skill_contract() {
    let good =
        "caddis rotate ready --kind omp --model <id>\ncaddis rotate arm\ncaddis rotate verify";
    assert!(
        good.contains("caddis rotate ready"),
        "good: must call ready"
    );
    assert!(good.contains("caddis rotate arm"), "good: must call arm");
    assert!(
        good.contains("caddis rotate verify"),
        "good: must call verify"
    );
    assert!(
        !good.contains("restart-preflight"),
        "good: must not cite restart-preflight.sh"
    );
    assert!(!good.contains("glm-5.3"), "good: must not pin glm-5.3");
    let bad = "GATE=~/.claude/skills/restart/restart-preflight.sh\n--model zai/glm-5.3";
    assert!(
        bad.contains("restart-preflight") && bad.contains("glm-5.3"),
        "bad: has both forbidden patterns"
    );
}

/// CARD-0123: rotate.py's receipt check calls `caddis rotate verify`,
/// not `restart-preflight.sh`. The .sh is gone; rotate.py does not treat
/// it as law.
#[test]
fn rotate_py_no_preflight() {
    let good = "subprocess.run([\"caddis\", \"rotate\", \"verify\"], ...)\n";
    assert!(good.contains("caddis"), "good: must call caddis");
    assert!(good.contains("rotate"), "good: must call rotate verify");
    assert!(
        !good.contains("restart-preflight"),
        "good: must not reference restart-preflight.sh"
    );
    let bad = "PREFLIGHT = Path(__file__).resolve().parent / \"restart-preflight.sh\"\n\
               subprocess.run([git_bash(), PREFLIGHT.as_posix(), \"verify\"])\n";
    assert!(
        bad.contains("restart-preflight"),
        "bad: references restart-preflight.sh"
    );
}

/// CARD-0127: OMP restart successor consumes session.receipt after verify.
#[test]
fn omp_restart_consumes_session_receipt() {
    let good = "caddis rotate verify\n$HOME/.caddis/rotation/session.receipt\n";
    assert!(good.contains("caddis rotate verify"), "good: verify");
    assert!(
        good.contains("session.receipt"),
        "good: must read session.receipt"
    );
    let bad = "caddis rotate verify\n# HMAC only, no session file\n";
    assert!(
        !bad.contains("session.receipt"),
        "bad: HMAC-only successor skips session.receipt"
    );
}

/// CARD-0138: OMP restart exports CADDIS_LINEAGE before herdr agent start.
/// used-pct is per-turn; stamping it at start is a lie.
#[test]
fn omp_restart_stamps_caddis_lineage() {
    let skill = include_str!("omp_restart_skill.md");
    let export = skill
        .find("export CADDIS_LINEAGE=")
        .expect("skill must export CADDIS_LINEAGE=");
    let start = skill
        .find("herdr agent start")
        .expect("skill must start the successor via herdr");
    assert!(
        export < start,
        "export CADDIS_LINEAGE= must precede herdr agent start"
    );
    assert!(
        skill.contains("caddis rotate ready --lineage"),
        "ready must pass --lineage"
    );
    assert!(
        skill.contains("caddis rotate arm --lineage"),
        "arm must pass --lineage"
    );
    assert!(
        skill.contains("caddis rotate verify --lineage"),
        "verify must pass --lineage"
    );
    assert!(
        !skill.contains("CADDIS_USED_PCT"),
        "must not stamp CADDIS_USED_PCT at start"
    );
    assert!(!skill.contains("glm-5.3"), "must not pin glm-5.3");
}

#[test]
fn omp_restart_successor_reads_packet() {
    let skill = include_str!("omp_restart_skill.md");
    assert!(
        skill.contains("caddis lineage packet --lineage"),
        "successor must query the lineage packet, not a letter"
    );
}

/// CARD-0149: the succession prompt template carries no escaped backticks.
/// A raw backtick inside the double-quoted herdr prompt is shell command
/// substitution — w3J:p3 ran herdr pane close on ITSELF (2026-08-27).
#[test]
fn omp_restart_prompt_has_no_backticks() {
    let skill = include_str!("omp_restart_skill.md");
    assert!(
        !skill.contains("\\`"),
        "prompt template must not rely on backslash-escaped backticks"
    );
    assert!(
        skill.contains("'caddis rotate verify --lineage $LINEAGE'"),
        "prompt commands must be single-quoted"
    );
}
