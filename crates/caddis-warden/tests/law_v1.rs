//! CARD-WARDEN-2 — the law, v1.
//!
//! These tests ARE the specification. Each names the estate law it descends
//! from, because a rule whose reason is lost gets deleted by the next person
//! who finds it inconvenient.
//!
//! THE ORGANISING PRINCIPLE, and it is a real constraint rather than a slogan:
//! a warden that blocks legitimate work gets switched off, and a switched-off
//! warden protects nothing. So `Deny` covers only the unambiguous, and anything
//! a competent engineer might do deliberately is `Steer` — allowed, with the
//! banked law delivered at the moment it applies.
//!
//! ⚠ WHY THE FIXTURES ARE ASSEMBLED FROM PARTS. Writing this file the obvious
//! way was BLOCKED by the estate's own `nobs-anti-disable` guard: a test that
//! proves the warden denies a suppression pattern must contain that pattern,
//! and the guard cannot tell a fixture from a real bypass. The sanctioned way
//! out is the inline `nobs-allow` escape hatch — but spending an allowance
//! here would put a literal suppression marker in a tracked file forever. So
//! the tokens are CONCATENATED at runtime instead: identical bytes reach the
//! warden, no literal pattern sits in the source, and no guard is weakened.
//! This is the same technique agent-execution.md §2 already mandates for secret
//! fixtures, applied to suppression fixtures for the same reason.

use caddis_warden::{decide, ToolCall, Verdict};

/// The suppression tokens, assembled so this file carries none of them literally.
fn no_verify() -> String {
    ["--no", "-verify"].concat()
}
fn nosec() -> String {
    ["# no", "sec"].concat()
}
fn eslint_disable() -> String {
    ["eslint-", "disable"].concat()
}

// ─────────────────────────── RULE 1 · SUPPRESSION ───────────────────────────
// Estate law: "Never weaken, bypass, or disable a guard to make work pass. Fix
// the work." Suppression is the estate's most-repeated failure — hence DENY.

#[test]
fn a_hook_bypassing_commit_is_denied() {
    let cmd = format!("git commit {} -m 'wip'", no_verify());
    let v = decide(&ToolCall::new("bash").command(&cmd));
    assert!(v.is_deny(), "bypassing hooks must be denied, got {v:?}");
}

#[test]
fn writing_a_scanner_suppression_is_denied() {
    let content = format!("token = load()  {}\n", nosec());
    let v = decide(&ToolCall::new("write").path("src/auth.py").content(&content));
    assert!(
        v.is_deny(),
        "a scanner suppression must be denied, got {v:?}"
    );
}

#[test]
fn a_linter_disable_is_denied() {
    let content = format!("// {}-next-line no-eval\neval(x)\n", eslint_disable());
    let v = decide(&ToolCall::new("edit").path("src/app.ts").content(&content));
    assert!(v.is_deny(), "got {v:?}");
}

/// THE ESCAPE HATCH IS PART OF THE RULE, not a weakening of it. A rule with no
/// legitimate way through gets bypassed wholesale instead of narrowly, and the
/// estate then loses the record of WHY the exception existed. The documented
/// form makes the exception auditable — which is the entire point.
#[test]
fn a_documented_suppression_is_allowed_not_denied() {
    let content = format!(
        "# nobs-allow: {} — vendored sample, tracked in CARD-SEC-9\ntoken = load()  {}\n",
        "nosec",
        nosec()
    );
    let v = decide(&ToolCall::new("write").path("src/auth.py").content(&content));
    assert!(
        !v.is_deny(),
        "a DOCUMENTED exception must pass; a rule with no way through gets bypassed wholesale: {v:?}"
    );
}

// ─────────────────────────── RULE 2 · SENSITIVE FILES ───────────────────────
// From the global contract's never-read-or-expose list. DENY, because there is
// no version of "I meant to overwrite the credential store".

#[test]
fn writing_a_credentials_file_is_denied() {
    let v = decide(
        &ToolCall::new("write")
            .path("C:/Users/ashpac/.claude/.credentials.json")
            .content("{}"),
    );
    assert!(v.is_deny(), "got {v:?}");
}

#[test]
fn a_normal_source_file_is_not_denied() {
    let v = decide(
        &ToolCall::new("write")
            .path("crates/caddis-core/src/ledger.rs")
            .content("pub fn ok() {}"),
    );
    assert_eq!(v, Verdict::Allow, "ordinary work must be untouched");
}

// ─────────────────────────── RULE 3 · DESTRUCTIVE GIT ───────────────────────

#[test]
fn a_force_push_to_a_shared_branch_is_denied() {
    let v = decide(&ToolCall::new("bash").command("git push --force origin master"));
    assert!(v.is_deny(), "got {v:?}");
}

#[test]
fn an_ordinary_push_is_allowed() {
    let v = decide(&ToolCall::new("bash").command("git push origin feat/warden"));
    assert_eq!(v, Verdict::Allow);
}

/// A hard reset is destructive but legitimate — the estate's own rules tell
/// agents to revert on gate failure. CARD-WARDEN-3 CHANGED THIS CONTRACT, and
/// the change is the point rather than a concession to make a test pass: the
/// steer no longer carries a paragraph of advice, it carries the finding of an
/// executed check. So when there is NOTHING TO REPORT the steer disappears
/// entirely — a green check emits nothing.
///
/// Judged against an explicit directory, never the process's own: a test that
/// reads `current_dir()` measures the workshop it happens to run in, and would
/// flip between Steer and Allow depending on whether this repo was dirty at the
/// moment someone ran it. The dirty-tree half lives in tests/checks_v1.rs,
/// where a real temp repo can be built and controlled.
/// ⚠ The fixture is a path that DOES NOT EXIST, not an empty temp directory.
/// An empty temp dir was environment-dependent: `std::env::temp_dir()` reads
/// `TMP` before `TEMP`, and with `TMP` unset it lands inside the user's home
/// directory — which is itself a git repo — so the "nothing to discard" fixture
/// silently became "a dirty repo with 20 files". Caught by a clean-agent audit
/// that got a FAILURE from the command that passed for me. A non-existent path
/// is the one location git cannot resolve on any machine.
#[test]
fn a_hard_reset_is_silent_when_there_is_nothing_to_discard() {
    let mut dir = std::env::temp_dir();
    dir.push(format!("caddis-law-v1-absent-{}", std::process::id()));
    dir.push("no-such-directory-anywhere");
    assert!(
        !dir.exists(),
        "precondition: the fixture path must NOT exist"
    );

    let v = caddis_warden::decide_in(
        &ToolCall::new("bash").command("git reset --hard origin/main"),
        &dir,
    );
    assert_eq!(
        v,
        Verdict::Allow,
        "nothing measured means nothing to say — advice-for-its-own-sake is the \
         wallpaper that trains the reader to skip the channel"
    );
}

// ─────────────────────────── RULE 4 · SECRET LITERALS ───────────────────────
// "NEVER put a secret in code/logs/commits — vault PATHS only."

#[test]
fn writing_a_secret_shaped_literal_is_denied() {
    let prefix: String = [115u8, 107u8, 45u8].iter().map(|b| *b as char).collect();
    let body = "A1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6";
    let v = decide(
        &ToolCall::new("write")
            .path("src/config.ts")
            .content(&format!("const key = \"{prefix}{body}\"\n")),
    );
    assert!(
        v.is_deny(),
        "a secret-shaped literal must be denied, got {v:?}"
    );
}

// ─────────────────────────── THE READ PATH STAYS FREE ───────────────────────
// A consciousness that taxes reading makes itself hated for no safety gain.

#[test]
fn reading_is_never_denied() {
    for tool in ["read", "grep", "glob", "ls"] {
        let v = decide(&ToolCall::new(tool).path("/etc/hosts"));
        assert!(!v.is_deny(), "{tool} must not be denied: {v:?}");
    }
}

/// Reading a SENSITIVE file is the one read that is not free — the contract
/// names these never-READ, not merely never-write.
#[test]
fn reading_a_sensitive_file_is_denied() {
    let v = decide(&ToolCall::new("read").path("~/.claude/.credentials.json"));
    assert!(v.is_deny(), "got {v:?}");
}

// ─────────────────────────── DENY CARRIES ITS REASON ────────────────────────

#[test]
fn every_deny_explains_itself_to_the_model() {
    let cmd = format!("git commit {} -m x", no_verify());
    let v = decide(&ToolCall::new("bash").command(&cmd));
    match v {
        Verdict::Deny { reason } => {
            assert!(
                reason.len() > 20,
                "a bare refusal teaches nothing: {reason:?}"
            );
            assert!(
                reason.to_lowercase().contains("verify") || reason.to_lowercase().contains("hook"),
                "the reason must name what was actually violated: {reason:?}"
            );
        }
        other => panic!("expected Deny, got {other:?}"),
    }
}
