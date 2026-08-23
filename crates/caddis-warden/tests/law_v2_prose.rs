//! CARD-WARDEN-4 — a content guard must exempt PROSE, or it forbids
//! documenting its own rules.
//!
//! FOUND BY PROBING THE FALSE-POSITIVE SURFACE, not by a failing test. Six of
//! seven probes were already correct; the seventh was this: a `.md` file
//! containing the sentence "never write the nosec marker in source" was DENIED.
//! The rule forbidding suppression made it impossible to WRITE DOWN the rule
//! forbidding suppression — and the mission document describing this very crate
//! would have been blocked by it.
//!
//! The same minute, banking the lesson about it was refused by the estate's OWN
//! jit-law, because the explanatory text quoted a CI-skip marker verbatim. Two
//! guards, different authors, identical failure. That is a CLASS, not a
//! coincidence, and it is why this card exists rather than a one-line patch.
//!
//! THE MISSING DISTINCTION IS "CAN THIS FILE ACTUALLY SUPPRESS ANYTHING":
//!   - prose (.md/.txt/.rst/.adoc) — a marker there is INERT. No scanner reads
//!     it. Exempting prose removes a case the rule never had jurisdiction over;
//!     it is not a weakening.
//!   - CI CONFIG (.yml/.yaml/.toml) — a skip-marker or an allow-failure does
//!     its damage precisely here. STAYS FULLY IN SCOPE. The exemption must NOT
//!     generalise to "non-code", which is the tempting and wrong version.
//!
//! A suite that only tests what should be DENIED cannot see over-blocking. This
//! file tests the other direction on purpose.

use caddis_warden::{decide, ToolCall};

fn nosec() -> String {
    ["# no", "sec"].concat()
}
fn skip_ci() -> String {
    ["[skip ", "ci]"].concat()
}
fn allow_failure() -> String {
    ["allow_", "failure"].concat()
}

/// Documentation ABOUT a marker must be writable. This is the defect.
#[test]
fn a_markdown_doc_explaining_a_marker_is_not_denied() {
    let body = format!("## Suppression\n\nNever write `{}` in source.\n", nosec());
    for path in [
        "docs/why-we-forbid.md",
        "NOTES.txt",
        "docs/policy.rst",
        "docs/guide.adoc",
    ] {
        let v = decide(&ToolCall::new("write").path(path).content(&body));
        assert!(
            !v.is_deny(),
            "prose cannot suppress anything, so `{path}` must be writable: {v:?}"
        );
    }
}

/// The guard's own incident report, postmortem and lesson bank are ALWAYS prose
/// about markers. A guard without this exemption eventually blocks its own
/// postmortem — which is the moment you most need to write.
#[test]
fn the_wardens_own_postmortem_is_writable() {
    let body = format!(
        "# Incident\n\nThe agent tried `{}` and was blocked. It then tried `{}`.\n",
        nosec(),
        skip_ci()
    );
    let v = decide(
        &ToolCall::new("write")
            .path("E:/ClaudeToolbox/_handoffs/INCIDENT-2026-08-23.md")
            .content(&body),
    );
    assert!(!v.is_deny(), "a postmortem must be writable: {v:?}");
}

// ── AND THE HALF THAT MUST NOT MOVE ─────────────────────────────────────────
// If the exemption leaked into config, the rule would lose the ground it
// actually defends. These are the positive controls for the exemption itself.

#[test]
fn a_ci_config_with_a_skip_marker_is_still_denied() {
    let body = format!("script:\n  - git commit -m \"wip {}\"\n", skip_ci());
    for path in [".gitlab-ci.yml", ".github/workflows/ci.yaml"] {
        let v = decide(&ToolCall::new("write").path(path).content(&body));
        assert!(
            v.is_deny(),
            "CI config is exactly where a skip marker does its damage: `{path}` {v:?}"
        );
    }
}

#[test]
fn a_ci_config_with_allow_failure_is_still_denied() {
    let body = format!("test:\n  {}: true\n", allow_failure());
    let v = decide(&ToolCall::new("write").path(".gitlab-ci.yml").content(&body));
    assert!(v.is_deny(), "{v:?}");
}

#[test]
fn source_code_is_still_fully_in_scope() {
    let body = format!("token = load()  {}\n", nosec());
    for path in ["src/auth.py", "src/app.ts", "crates/x/src/lib.rs"] {
        let v = decide(&ToolCall::new("write").path(path).content(&body));
        assert!(v.is_deny(), "`{path}` must stay in scope: {v:?}");
    }
}

/// A file with NO extension, or an unknown one, is treated as code. The default
/// must be the strict side: an exemption should be granted by recognition, not
/// by failure to recognise.
#[test]
fn an_unknown_extension_defaults_to_in_scope() {
    let body = format!("token = load()  {}\n", nosec());
    for path in ["Makefile", "scripts/deploy", "weird.qqq"] {
        let v = decide(&ToolCall::new("write").path(path).content(&body));
        assert!(
            v.is_deny(),
            "unrecognised must default to STRICT, never to exempt: `{path}` {v:?}"
        );
    }
}

/// A `.md` is prose for the SUPPRESSION rule only. It is not a free pass: a
/// secret written into a markdown file is still a secret in the repo forever.
#[test]
fn the_prose_exemption_does_not_extend_to_secrets() {
    let prefix: String = [115u8, 107u8, 45u8].iter().map(|b| *b as char).collect();
    let body = format!("Example key: {prefix}A1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6\n");
    let v = decide(&ToolCall::new("write").path("docs/setup.md").content(&body));
    assert!(
        v.is_deny(),
        "a secret in a .md is still a secret in git history forever: {v:?}"
    );
}
