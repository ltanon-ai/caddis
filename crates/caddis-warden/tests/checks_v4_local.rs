//! checks_v4_local.rs — the three warden-local checks, driven both ways.
//!
//! Each of these fired on the session that wrote them and was correct each time.
//! That is the field evidence for the SELECTION RULE (a lesson becomes a check
//! only when the trigger is nearly always wrong) — and it is why the GREEN cases
//! below carry as much weight as the RED ones. A check that fires on the correct
//! idiom is worse than no check: it trains the reader to skip the channel.

use caddis_warden::checks::shell_local as sl;

// ------------------------------------------- shell.exit-code-through-pipe

#[test]
fn reading_a_dollar_question_after_a_pipe_is_found() {
    let f = sl::exit_code_through_pipe("cargo test 2>&1 | tail -5; echo rc=$?")
        .expect("a piped exit-code read must produce a finding");
    assert!(
        f.contains("last process") || f.contains("pipe"),
        "the finding states the mechanism: {f}"
    );
}

#[test]
fn the_pipestatus_idiom_is_silent() {
    // The correct form the estate was told to adopt. Firing here would punish
    // the fix and kill the channel.
    assert_eq!(
        sl::exit_code_through_pipe("cargo test | tail -5; echo rc=${PIPESTATUS[0]}"),
        None
    );
}

#[test]
fn pipefail_is_silent() {
    assert_eq!(
        sl::exit_code_through_pipe("set -o pipefail; cargo test | tail -5; echo rc=$?"),
        None
    );
}

#[test]
fn an_exit_code_read_with_no_pipe_at_all_is_silent() {
    assert_eq!(
        sl::exit_code_through_pipe("cargo test > out.txt 2>&1; echo rc=$?"),
        None,
        "capturing directly and then reading $? is the RECOMMENDED shape"
    );
}

#[test]
fn a_later_unpiped_command_does_not_inherit_an_earlier_pipeline() {
    // `$?` reads the command IMMEDIATELY before it. A pipeline earlier on the
    // line is already settled, so condemning a later direct capture flags the
    // very shape this finding's own message prescribes as the remedy.
    assert_eq!(
        sl::exit_code_through_pipe("ls | head -5; echo done; foo > out 2>&1; rc=$?"),
        None,
        "an earlier pipeline must not condemn a later direct capture"
    );
}

#[test]
fn a_pipeline_with_no_exit_code_read_is_silent() {
    assert_eq!(sl::exit_code_through_pipe("cargo test | tail -5"), None);
}

// --------------------------------------- shell.gate-chained-into-commit

#[test]
fn chaining_a_gate_into_a_commit_is_found() {
    let f = sl::gate_chained_into_commit("python tools/gate.py --deep && git commit -m x")
        .expect("a gate chained into a commit must produce a finding");
    assert!(f.contains("exit code") || f.contains("red"), "{f}");
}

#[test]
fn chaining_a_test_runner_into_a_stage_is_found() {
    assert!(sl::gate_chained_into_commit("pytest && git add -A").is_some());
    assert!(sl::gate_chained_into_commit("cargo test && git commit -am wip").is_some());
}

#[test]
fn an_ordinary_cd_before_a_commit_is_silent() {
    // `cd` is not a gate. Denying this shape would fire on almost every commit
    // anyone makes.
    assert_eq!(
        sl::gate_chained_into_commit("cd repo && git add src/lib.rs"),
        None
    );
}

#[test]
fn a_gate_run_as_its_own_command_is_silent() {
    assert_eq!(
        sl::gate_chained_into_commit("python tools/gate.py --deep"),
        None
    );
    assert_eq!(sl::gate_chained_into_commit("git commit -m x"), None);
}

#[test]
fn a_gate_after_the_commit_is_not_the_hazard() {
    // The defect is the gate's exit code being swallowed BEFORE the commit.
    assert_eq!(
        sl::gate_chained_into_commit("git commit -m x && pytest"),
        None
    );
}

// ------------------------------------- shell.posix-tmp-across-python

#[test]
fn a_posix_tmp_path_crossing_into_windows_python_is_found() {
    let f = sl::posix_tmp_across_python_on(
        "curl -s http://x > /tmp/x.json && python -c \"import json;json.load(open('/tmp/x.json'))\"",
        true,
    )
    .expect("a /tmp path handed across the bash-to-Windows-Python boundary must be found");
    assert!(f.contains("/tmp"), "{f}");
}

#[test]
fn the_same_command_is_silent_where_the_premise_is_false() {
    // On a POSIX host both interpreters agree; firing there is pure noise, and
    // the finding would be describing a hazard that does not exist.
    assert_eq!(
        sl::posix_tmp_across_python_on(
            "curl -s http://x > /tmp/x.json && python -c \"open('/tmp/x.json')\"",
            false
        ),
        None
    );
}

#[test]
fn a_drive_qualified_path_is_silent() {
    assert_eq!(
        sl::posix_tmp_across_python_on(
            "curl -s http://x > H:/scratch/x.json && python -c \"open('H:/scratch/x.json')\"",
            true
        ),
        None
    );
}

#[test]
fn a_tmp_path_with_no_python_is_silent() {
    assert_eq!(
        sl::posix_tmp_across_python_on("cat /tmp/x.json | wc -l", true),
        None
    );
}

#[test]
fn python_with_no_tmp_path_is_silent() {
    assert_eq!(
        sl::posix_tmp_across_python_on("python -c \"print(1)\"", true),
        None
    );
}

// ------------------------------------------------------------- registry

#[test]
fn all_three_are_registered_and_soft() {
    use caddis_warden::checks::{severity_of, Severity};
    for id in [
        "shell.exit-code-through-pipe",
        "shell.gate-chained-into-commit",
        "shell.posix-tmp-across-python",
    ] {
        assert_eq!(
            severity_of(id),
            Some(Severity::Soft),
            "{id} must be registered as SOFT — each is a wrong measurement or a \
             process slip, never an irreversible act, and denying would block \
             work the operator legitimately asked for"
        );
    }
}
