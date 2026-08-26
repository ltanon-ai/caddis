//! Direct tests for the allowlist matcher (CARD-0111).
//!
//! These are the pure halves — normalizing, matching, exemptions. The verdicts
//! themselves are driven through the real binary in `tests/card_gate.rs`,
//! because a gate that behaves differently in-process than it does when spawned
//! would be worse than no gate.

use super::*;

fn decl(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_string()).collect()
}

#[test]
fn an_exact_declared_path_matches_and_a_sibling_does_not() {
    let d = decl(&["src/a.rs"]);
    assert!(declared_covers(&d, &normalize("src/a.rs")));
    assert!(!declared_covers(&d, &normalize("src/b.rs")));
    // A prefix that is not a path boundary must not match.
    assert!(!declared_covers(&d, &normalize("src/a.rs.bak")));
}

#[test]
fn a_trailing_slash_declares_a_subtree_and_nothing_else_does() {
    let subtree = decl(&["src/"]);
    assert!(declared_covers(&subtree, &normalize("src/a.rs")));
    assert!(declared_covers(&subtree, &normalize("src/deep/b.rs")));
    assert!(declared_covers(&subtree, &normalize("src")));
    // The neighbouring directory is NOT inside it.
    assert!(!declared_covers(&subtree, &normalize("srcx/a.rs")));

    // Without the slash it is one exact file, never a subtree.
    let exact = decl(&["src"]);
    assert!(!declared_covers(&exact, &normalize("src/a.rs")));
}

#[test]
fn globs_are_not_honoured_because_a_declaration_must_be_readable() {
    // A glob in an allowlist is a promise nobody can check by reading it, and
    // the card law wants the exact editable paths. `src/*` matches literally
    // nothing rather than silently everything.
    let d = decl(&["src/*", "*.rs"]);
    assert!(!declared_covers(&d, &normalize("src/a.rs")));
    assert!(!declared_covers(&d, &normalize("a.rs")));
}

#[test]
fn a_dotdot_escape_never_matches() {
    // A declaration cannot reach outward; otherwise `../../etc/x` is declarable.
    let d = decl(&["../outside.rs", "src/../src/a.rs"]);
    assert!(!declared_covers(&d, &normalize("../outside.rs")));
    assert!(!declared_covers(&d, &normalize("src/a.rs")));
}

#[test]
fn backslashes_fold_so_the_same_file_matches_written_either_way() {
    let d = decl(&["src/a.rs"]);
    assert!(declared_covers(&d, &normalize("src\\a.rs")));
    let win = decl(&["src\\a.rs"]);
    assert!(declared_covers(&win, &normalize("src/a.rs")));
}

#[test]
fn case_folds_only_where_the_filesystem_does() {
    let d = decl(&["src/A.rs"]);
    // On Windows two spellings are one file; on Linux they are two, and folding
    // there would let a declaration cover a file it never named.
    assert_eq!(declared_covers(&d, &normalize("src/a.rs")), cfg!(windows));
}

#[test]
fn a_path_with_stray_whitespace_still_matches_what_it_declares() {
    // A frame with a miscounted length hands the warden `src/a.rs\n`. Without
    // trimming, a DECLARED file is denied and shows up in the attest bundle's
    // OUTSIDE list — safe, but a gate that refuses the file you declared is a
    // gate people switch off. Found by an end-to-end drive, not by a fixture.
    let d = decl(&["src/a.rs"]);
    assert!(declared_covers(&d, &normalize("src/a.rs\n")));
    assert!(declared_covers(&d, &normalize("  src/a.rs  ")));
    assert!(declared_covers(&d, &normalize("src/a.rs\r\n")));
    // And a declaration written with trailing space works the same way.
    assert!(declared_covers(
        &decl(&["src/a.rs "]),
        &normalize("src/a.rs")
    ));
}

#[test]
fn an_empty_declaration_entry_matches_nothing() {
    assert!(!declared_covers(&decl(&["", "  "]), &normalize("src/a.rs")));
    assert!(!declared_covers(&decl(&[]), &normalize("src/a.rs")));
}

#[test]
fn a_path_inside_the_working_directory_is_compared_relatively() {
    let cwd = std::path::Path::new("C:/w/repo");
    assert_eq!(
        relative_to("C:/w/repo/src/a.rs", cwd),
        normalize("src/a.rs")
    );
    assert_eq!(relative_to("src/a.rs", cwd), normalize("src/a.rs"));
    // Outside the working directory it keeps its own shape rather than being
    // forced into a relative one it does not have.
    //
    // ⛔ THE EXPECTATION MUST GO THROUGH `normalize` WITH THE SAME CASE AS THE
    // INPUT. Spelling it `d:` hard-coded a WINDOWS artifact: `normalize`
    // lowercases only under `cfg!(windows)`, so the assertion could only ever
    // hold on Windows and failed on ubuntu and macos the first time this code
    // reached CI. Comparing `normalize(X)` against `relative_to(X, ..)` tests
    // the real property — an outside path keeps its own shape — on every
    // platform, because both sides fold or do not fold together.
    assert_eq!(
        relative_to("D:/other/x.rs", cwd),
        normalize("D:/other/x.rs")
    );
}

#[test]
fn the_ledger_and_its_lock_are_exempt_whatever_a_card_declares() {
    // A gate that can stop the warden recording its own verdicts breaks the
    // audit trail the whole trust argument rests on.
    let led = "C:/Users/x/.caddis/warden-ledger.jsonl";
    assert!(is_exempt(&normalize(led), led));
    assert!(is_exempt(
        &normalize("C:/Users/x/.caddis/warden-ledger.lock"),
        led
    ));
}

#[test]
fn build_and_vcs_machinery_is_exempt_at_any_depth() {
    let led = "C:/led.jsonl";
    for p in [
        "target/debug/x",
        "crates/a/target/debug/x",
        "node_modules/pkg/i.js",
        ".git/HEAD",
        "sub/.git/HEAD",
    ] {
        assert!(is_exempt(&normalize(p), led), "{p} should be exempt");
    }
    // And an ordinary source file is NOT exempt, or the exemption is a hole.
    assert!(!is_exempt(&normalize("src/a.rs"), led));
    assert!(!is_exempt(&normalize("targeted/a.rs"), led));
}
