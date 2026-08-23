//! checks_v2_shell.rs — the shell-shaped checks, the incident latch, and the
//! registry's own self-consistency.
//!
//! Split from `checks_v2_git.rs` under the repo's 280-line file law. The seam is
//! the one the source already has: `checks/git.rs` against `checks/shell.rs` and
//! `checks/incidents.rs`. Splitting a test file anywhere else makes the tests
//! stop mirroring the thing they test, and then nobody can tell which half is
//! missing coverage.
//!
//! ⭐ EACH CHECK GETS A RED CASE **AND** A GREEN CASE. A check that fires on
//! everything is indistinguishable from a check that fires on the right thing if
//! you only ever test the red — and it is the more expensive failure, because it
//! trains the reader to skip the channel and takes every OTHER finding with it.

use caddis_warden::checks::{git, incidents, shell};

// ---------------------------------------------------------- shell.skip-ci

#[test]
fn a_ci_skipping_commit_message_is_found() {
    let marker = format!("[{} {}]", "skip", "ci");
    let f = shell::skip_ci_marker(&format!("git commit -m \"wip {marker}\"")).expect("finding");
    assert!(f.contains("quality control"), "{f}");
}

#[test]
fn an_ordinary_bracketed_word_is_not_a_ci_skip() {
    assert_eq!(
        shell::skip_ci_marker("git commit -m \"[refactor] tidy\""),
        None
    );
}

// ---------------------------------------------------- shell.osv-no-resolve

#[test]
fn suppressing_dependency_resolution_is_found() {
    let f = shell::osv_no_resolve("osv-scanner --no-resolve -r .").expect("finding");
    assert!(f.contains("never examined"), "{f}");
}

#[test]
fn an_honest_osv_scan_is_silent() {
    assert_eq!(shell::osv_no_resolve("osv-scanner -r ."), None);
}

#[test]
fn the_flag_on_an_unrelated_tool_is_not_an_osv_finding() {
    assert_eq!(shell::osv_no_resolve("somethingelse --no-resolve"), None);
}

// --------------------------------------------- shell.git-show-piped-counter

#[test]
fn a_git_show_piped_into_a_counter_is_found() {
    let f =
        shell::git_show_piped_into_a_counter("git show HEAD:src/lib.rs | wc -l").expect("finding");
    assert!(f.contains("128"), "the finding states the mechanism: {f}");
}

#[test]
fn a_git_show_that_is_not_piped_into_a_counter_is_silent() {
    assert_eq!(
        shell::git_show_piped_into_a_counter("git show HEAD:src/lib.rs > out.txt"),
        None
    );
}

#[test]
fn a_counter_after_an_and_operator_is_not_a_pipeline() {
    // `&&` runs wc on something else entirely; flagging it would be noise, and
    // noise is what kills the channel.
    assert_eq!(
        shell::git_show_piped_into_a_counter("git show HEAD:a.rs && wc -l other.txt"),
        None
    );
}

// ------------------------------------------ shell.process-query-self-match

#[test]
fn a_self_matching_windows_process_query_is_found() {
    let cmd = "Get-CimInstance Win32_Process | ? { $_.CommandLine -match 'supervisor' }";
    let f = shell::process_query_self_match(cmd).expect("finding");
    assert!(f.contains("matches itself"), "{f}");
}

#[test]
fn a_windows_process_query_that_excludes_itself_is_silent() {
    let cmd = "Get-CimInstance Win32_Process | ? { $_.CommandLine -match 'sup' -and \
               $_.ProcessId -ne $PID }";
    assert_eq!(shell::process_query_self_match(cmd), None);
}

#[test]
fn a_bare_ps_grep_is_found_and_an_inverted_one_is_not() {
    assert!(shell::process_query_self_match("ps aux | grep supervisor").is_some());
    assert_eq!(
        shell::process_query_self_match("ps aux | grep -v grep | grep supervisor"),
        None
    );
}

// ----------------------------------------- git.push.into-rewritten-repo

fn incident(repo: &str) -> incidents::Incident {
    incidents::Incident {
        repo: repo.to_string(),
        reference: "refs/remotes/origin/main".to_string(),
        old: "655f64d2aaaa".to_string(),
        new: "0000000000".to_string(),
    }
}

#[test]
fn pushing_into_a_repo_with_an_open_rewrite_incident_is_found() {
    let inc = vec![incident("E:\\ClaudeToolbox\\_worktrees\\bee-build-laisvas")];
    let f = incidents::push_into_rewritten_repo_with(
        "cd E:/ClaudeToolbox/_worktrees/bee-build-laisvas && git push origin main",
        &inc,
    )
    .expect("finding");
    assert!(f.contains("UNRESOLVED HISTORY REWRITE"), "{f}");
}

#[test]
fn a_local_commit_in_that_repo_is_not_blocked() {
    // Reading, testing and committing locally are safe and are often exactly
    // what the recovery needs. Only PUBLISHING more history deepens the damage.
    let inc = vec![incident("E:\\ClaudeToolbox\\_worktrees\\bee-build-laisvas")];
    assert_eq!(
        incidents::push_into_rewritten_repo_with(
            "cd E:/ClaudeToolbox/_worktrees/bee-build-laisvas && git commit -m x",
            &inc
        ),
        None
    );
}

#[test]
fn a_short_repo_leaf_does_not_match_an_unrelated_command() {
    // THE REGRESSION THIS EXISTS FOR: a real incident repo ends in `/wt`, and a
    // bare substring test makes those two letters match `newt`, `swt`, or any
    // command that happens to contain them — on a check that DENIES a push.
    let inc = vec![incident(
        "H:\\_claude_temp\\pytest-4000\\test_a_protected0\\wt",
    )];
    assert_eq!(
        incidents::push_into_rewritten_repo_with("git push origin newt-feature", &inc),
        None,
        "a two-letter leaf must not deny an unrelated push"
    );
    assert!(
        incidents::push_into_rewritten_repo_with(
            "git -C H:/_claude_temp/pytest-4000/test_a_protected0/wt push origin main",
            &inc
        )
        .is_some(),
        "but the real repo must still be caught"
    );
}

#[test]
fn no_open_incident_means_perfect_silence() {
    assert_eq!(
        incidents::push_into_rewritten_repo_with("git push origin main", &[]),
        None
    );
}

#[test]
fn the_log_parser_reads_windows_paths_and_honours_resolved() {
    let log = concat!(
        "{\"repo\": \"E:\\\\Tool\\\\repo-a\", \"ref\": \"refs/heads/main\", ",
        "\"old\": \"abc123def\", \"new\": \"999\"}\n",
        "{\"repo\": \"E:\\\\Tool\\\\repo-b\", \"resolved\": true}\n",
        "not json at all\n"
    );
    let got = incidents::open_incidents_from(log);
    assert_eq!(got.len(), 1, "resolved rows and junk lines are skipped");
    assert_eq!(
        got[0].repo, "E:\\Tool\\repo-a",
        "the JSON escape must be undone or no path ever matches"
    );
}

// ------------------------------------------ git.reset.discards-uncommitted

#[test]
fn the_reset_check_only_triggers_on_an_actual_hard_reset() {
    // THE REGRESSION THIS EXISTS FOR: the finding is measured with `git status`,
    // which reports a dirty tree no matter what the command was. A check with no
    // trigger therefore steers on EVERY call in a dirty repo — ordinary pushes,
    // ordinary writes — which is the wallpaper the precision rule forbids.
    assert!(git::is_hard_reset("git reset --hard origin/main"));
    assert!(git::is_hard_reset("cd repo && git reset --hard"));
    assert!(
        git::is_hard_reset("git -C /some/repo reset --hard"),
        "a -C reset is still a reset"
    );
    assert!(!git::is_hard_reset("git push origin main"));
    assert!(!git::is_hard_reset("git reset --soft HEAD~1"));
    assert!(!git::is_hard_reset("echo git reset --hard"));
}

// --------------------------- the -C hole, closed on every git-shaped check

#[test]
fn a_dash_c_path_does_not_hide_the_subcommand_from_any_check() {
    // The estate's Python takes the first non-dash token after `git`, which for
    // `git -C /repo push ...` is `/repo` — so the whole force-push gate is
    // evaded by a spelling this estate uses constantly.
    assert!(
        git::force_push_to_protected("git -C /repo push --force origin main").is_some(),
        "a -C force-push to master must still be caught"
    );
    assert!(git::skips_hooks(&format!(
        "git -C /repo commit {} -m x",
        ["--no", "-verify"].concat()
    ))
    .is_some());
    assert!(git::blanket_stage("git -C /repo add -A").is_some());
}

#[test]
fn a_dash_c_push_still_reads_its_refspec_correctly() {
    // `-C <path>` adds a positional, so counting positionals from the start of
    // the line reads the remote as the refspec and the branch as a remote.
    assert_eq!(
        git::force_push_to_protected("git -C /repo push --force origin feature-x"),
        None,
        "the remote must not be mistaken for the destination branch"
    );
}

// ------------------------------------------------------------- the registry

#[test]
fn the_registry_answers_for_every_id_it_claims() {
    // `is_registered` and `run` are derived from ONE table; this pins that they
    // cannot disagree, which is the failure that would make the drift ratchet
    // report coverage the crate does not have.
    let cwd = std::path::Path::new(".");
    for id in caddis_warden::checks::registered_ids() {
        assert!(
            caddis_warden::checks::is_registered(id),
            "{id} is enumerated but not registered"
        );
        assert!(
            caddis_warden::checks::severity_of(id).is_some(),
            "{id} has no severity"
        );
        let ctx = caddis_warden::checks::Ctx { command: "", cwd };
        // An empty command must not panic any check.
        caddis_warden::checks::run(id, &ctx);
    }
}
