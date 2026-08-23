//! CARD-WARDEN-3 — a steer must deliver a CHECK FINDING, never a paragraph.
//!
//! THE RULING THIS IMPLEMENTS (quorum, 2026-07-27, via ~/.claude/hooks/jit-laws.py):
//!
//! > "Text is the weak form. In the incident that started this work the relevant
//! > doctrine file was LOADED and was used as a reason NOT to act — so injecting
//! > the same words closer to the moment plausibly strengthens the
//! > rationalization rather than breaking it. A check either found the thing or
//! > it did not."
//!
//! v1 of this warden shipped prose. A paragraph about being careful with
//! `git reset --hard` is exactly the loaded-doctrine-as-permission shape: the
//! agent reads it, notes that it has considered the risk, and proceeds.
//! "This discards 3 modified files: a.rs, b.rs, c.rs" cannot be read that way.
//!
//! AND THE PRECISION RULE, which is the half that is easy to drop: A GREEN
//! CHECK EMITS NOTHING. If the tree is clean there is nothing to say, and
//! saying it anyway is the wallpaper that trains the reader to skip the
//! channel — which would cost the findings that DO matter. Silence here is a
//! measured outcome, not an absence.

use caddis_warden::checks;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A real git repo in TEMP. The testfence forbids tests mutating non-temp
/// paths, and a check that shells out to git cannot be honestly tested against
/// a fake one.
struct Repo(PathBuf);

impl Repo {
    fn new(name: &str) -> Self {
        let mut p = std::env::temp_dir();
        p.push(format!("caddis-warden3-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).expect("mkdir");
        let r = Repo(p);
        r.git(&["init", "-q"]);
        r.git(&["config", "user.email", "w@example.invalid"]);
        r.git(&["config", "user.name", "warden test"]);
        r
    }
    fn git(&self, args: &[&str]) {
        let out = Command::new("git")
            .arg("-C")
            .arg(&self.0)
            .args(args)
            .output()
            .expect("git runs");
        assert!(out.status.success(), "git {args:?} failed: {out:?}");
    }
    fn write(&self, name: &str, body: &str) {
        std::fs::write(self.0.join(name), body).expect("write");
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A DIRTY tree must produce a finding that NAMES what would be destroyed.
#[test]
fn a_dirty_tree_names_the_files_that_would_be_discarded() {
    let r = Repo::new("dirty");
    r.write("kept.txt", "one\n");
    r.git(&["add", "kept.txt"]);
    r.git(&["commit", "-qm", "base"]);
    r.write("kept.txt", "one\ntwo\n"); // modified
    r.write("fresh.txt", "new\n"); // untracked

    let finding =
        checks::git_reset_discards(r.path()).expect("a dirty tree must produce a finding");
    assert!(
        finding.contains("kept.txt"),
        "the finding must NAME the modified file, not describe risk: {finding}"
    );
    assert!(
        finding.contains("fresh.txt"),
        "untracked files are destroyed by a hard reset too: {finding}"
    );
}

/// THE PRECISION RULE. A clean tree says NOTHING.
#[test]
fn a_clean_tree_is_silent() {
    let r = Repo::new("clean");
    r.write("kept.txt", "one\n");
    r.git(&["add", "kept.txt"]);
    r.git(&["commit", "-qm", "base"]);

    assert_eq!(
        checks::git_reset_discards(r.path()),
        None,
        "a green check must emit NOTHING — wallpaper trains the reader to skip the channel"
    );
}

/// The finding is a MEASUREMENT, so it carries a count the reader can check
/// against the list. A bare "you have uncommitted changes" is the weak form
/// again, one level down.
#[test]
fn the_finding_is_specific_not_a_warning() {
    let r = Repo::new("specific");
    r.write("a.txt", "1\n");
    r.git(&["add", "a.txt"]);
    r.git(&["commit", "-qm", "base"]);
    r.write("a.txt", "2\n");

    let f = checks::git_reset_discards(r.path()).expect("finding");
    assert!(
        f.contains('1') || f.to_lowercase().contains("one"),
        "the finding must carry the COUNT so the reader can check it: {f}"
    );
    assert!(
        !f.to_lowercase().contains("be careful"),
        "a finding states what IS, never advice: {f}"
    );
}

/// Where git cannot report at all, the check is SILENT: an ERROR is not a
/// finding and must never be dressed as one.
///
/// ⚠ THIS TEST USED A TEMP DIRECTORY AND WAS ENVIRONMENT-DEPENDENT. Caught by a
/// clean-agent audit, which got a FAILURE from the same command that passed for
/// me. Root cause, measured: `std::env::temp_dir()` reads `TMP` BEFORE `TEMP`.
/// With `TMP` set it resolved to `H:\_claude_temp`; with `TMP` unset it fell to
/// `C:\Users\ashpac\AppData\Local\Temp` — which is INSIDE the user's home
/// directory, AND THAT HOME IS ITSELF A GIT REPO. So the "non-repo" fixture was
/// a repo with 20 dirty files, the check correctly reported them, and the test
/// failed. The CHECK was right the whole time; the FIXTURE's premise was false.
///
/// The old test asserted "outside a repo" while never verifying its fixture was
/// outside a repo — the same vacuous-fixture shape as an absence-assertion with
/// no positive control. On a machine where `$HOME` is a repo, NO temp directory
/// can be relied on to be outside one, so the premise is not merely unverified,
/// it is unconstructible.
///
/// A path that DOES NOT EXIST is the one location git can never resolve, on any
/// machine, under any TMP. That is what makes this deterministic.
#[test]
fn a_location_git_cannot_resolve_is_silent_rather_than_inventing_a_finding() {
    let mut p = std::env::temp_dir();
    p.push(format!("caddis-warden3-absent-{}", std::process::id()));
    p.push("no-such-directory-anywhere");
    assert!(!p.exists(), "precondition: the fixture path must NOT exist");

    assert_eq!(
        checks::git_reset_discards(&p),
        None,
        "nothing was measured, so there is nothing to say — and an unmeasurable \
         location must never be reported as a finding"
    );
}

/// The companion the old test was missing: when the directory IS inside a repo,
/// the check reports THAT repo. Proven here against a repo we built, so it holds
/// whatever the machine's TMP happens to be.
#[test]
fn a_subdirectory_inside_a_repo_reports_that_repo() {
    let r = Repo::new("subdir");
    r.write("tracked.txt", "1\n");
    r.git(&["add", "tracked.txt"]);
    r.git(&["commit", "-qm", "base"]);
    r.write("tracked.txt", "2\n");
    let sub = r.path().join("nested");
    std::fs::create_dir_all(&sub).expect("mkdir");

    let f = checks::git_reset_discards(&sub)
        .expect("a dirty enclosing repo must be reported from a subdirectory");
    assert!(f.contains("tracked.txt"), "{f}");
}

/// The registry maps IDs to code. An UNKNOWN id is silent, never a crash and
/// never a fabricated finding.
#[test]
fn an_unknown_check_id_is_silent() {
    let r = Repo::new("registry");
    let ctx = checks::Ctx {
        command: "",
        cwd: r.path(),
    };
    assert_eq!(checks::run("no.such.check", &ctx), None);
}

#[test]
fn the_registry_routes_the_known_id_to_its_check() {
    let r = Repo::new("route");
    r.write("x.txt", "1\n");
    r.git(&["add", "x.txt"]);
    r.git(&["commit", "-qm", "base"]);
    r.write("x.txt", "2\n");

    let ctx = checks::Ctx {
        command: "git reset --hard",
        cwd: r.path(),
    };
    let direct = checks::git_reset_discards(r.path());
    let routed = checks::run("git.reset.discards-uncommitted", &ctx);
    assert!(direct.is_some(), "precondition: the check fires");
    assert_eq!(routed, direct, "the registry must route to the same code");
}

/// THE INTEGRATION: a hard reset in a DIRTY repo steers, and the steer carries
/// the check's finding rather than advice. This is the half that proves the law
/// and the check are actually wired to each other.
#[test]
fn a_hard_reset_in_a_dirty_repo_steers_with_the_finding() {
    use caddis_warden::{decide_in, ToolCall, Verdict};

    let r = Repo::new("integration");
    r.write("doomed.txt", "1\n");
    r.git(&["add", "doomed.txt"]);
    r.git(&["commit", "-qm", "base"]);
    r.write("doomed.txt", "2\n");

    let v = decide_in(
        &ToolCall::new("bash").command("git reset --hard origin/main"),
        r.path(),
    );
    match v {
        Verdict::Steer { law, .. } => {
            assert!(
                law.contains("doomed.txt"),
                "the steer must carry the CHECK'S FINDING, naming the file: {law}"
            );
            assert!(
                !law.to_lowercase().contains("be careful"),
                "advice is the weak form the ruling rejected: {law}"
            );
        }
        other => panic!("expected Steer carrying a finding, got {other:?}"),
    }
}
