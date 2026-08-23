//! checks — laws name a check, the check RUNS, and its FINDING is what reaches
//! the model.
//!
//! WHY THIS EXISTS INSTEAD OF PROSE (quorum ruling 2026-07-27): text is the weak
//! form. In the incident that produced that ruling the relevant doctrine file
//! was LOADED and was used as a reason NOT to act, so injecting the same words
//! nearer the moment plausibly strengthens the rationalization rather than
//! breaking it. A paragraph about being careful with `git reset --hard` reads as
//! "risk considered, proceeding". "This discards 3 files: a.rs, b.rs, c.rs"
//! does not.
//!
//! THE PRECISION RULE: a GREEN check emits NOTHING. If the demand is already
//! satisfied there is nothing to say, and saying it anyway is the wallpaper that
//! trains the reader to skip the channel — which costs the findings that matter.
//! Silence is a measured outcome here, never an absence.
//!
//! AND THE HONEST BOUND, which belongs beside every claim this crate makes: a
//! checker of tool INPUT is a VOCABULARY gate, not an outcome gate. It
//! recognises spellings; it cannot see effects. Someone who wants a forbidden
//! outcome can reach for a spelling nobody enumerated, or bypass the tool
//! entirely. This makes known mistakes loud. It makes nothing safe.

pub mod cmdline;
pub mod git;
pub mod gitgrammar;
pub mod incident_log;
pub mod incidents;
pub(crate) mod lexer;
pub(crate) mod naive;
pub(crate) mod positions;
pub(crate) mod registry;
pub(crate) mod runners;
pub(crate) mod scan;
pub mod shell;
pub mod shell_local;

use std::path::Path;
use std::process::Command;

/// A check's finding. `None` is GREEN and means SILENCE.
pub type Finding = Option<String>;

/// Everything a check is allowed to look at.
///
/// One context type rather than a signature per check: the registry below stores
/// them in a single table, and a table of heterogeneous signatures is a table
/// that cannot exist. It also fixes what a check may READ, which is the point —
/// widening this struct is a deliberate act, not a drift.
pub struct Ctx<'a> {
    pub command: &'a str,
    pub cwd: &'a Path,
}

/// How loudly a red finding should land.
///
/// HARD denies; SOFT steers. The split is inherited from the estate's own
/// registry and it encodes the constraint that outranks coverage: a warden that
/// blocks legitimate work gets switched off, and a switched-off warden protects
/// nothing. A wrong measurement (SOFT) is corrected as well by a finding as by a
/// refusal; an irreversible rewrite (HARD) is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Hard,
    Soft,
}

type CheckFn = fn(&Ctx) -> Finding;

/// ID -> (callable, severity). THE ONE TABLE.
///
/// `run`, `is_registered` and `severity_of` are all derived from this, so there
/// is exactly one place that knows which ids exist. An earlier draft kept a
/// separate `REGISTERED` list beside the dispatch `match`: that is two
/// bookkeepings of one fact, and they rot apart precisely where it matters —
/// `is_registered` would answer yes for an id `run` silently ignored, which is
/// the drift ratchet reporting coverage the crate does not have.
///
/// THE ID -> CALLABLE MAP LIVES IN CODE. Law data names a check; it never
/// supplies one. That is the trust boundary, and it is what allows the shared
/// law table to be regenerated or edited without any of it becoming an execution
/// path.
const REGISTRY: &[(&str, CheckFn, Severity)] = &[
    ("git.reset.discards-uncommitted", c_reset, Severity::Soft),
    ("git.hooks.skipped", c_hooks, Severity::Hard),
    ("git.signing.bypassed", c_signing, Severity::Hard),
    ("git.push.force-to-protected", c_force_push, Severity::Hard),
    ("git.push.into-rewritten-repo", c_rewritten, Severity::Hard),
    (
        "git.stage.blanket-in-shared-worktree",
        c_blanket,
        Severity::Soft,
    ),
    ("shell.skip-ci", c_skip_ci, Severity::Hard),
    ("shell.osv-no-resolve", c_osv, Severity::Hard),
    ("shell.git-show-piped-counter", c_git_show, Severity::Soft),
    (
        "shell.process-query-self-match",
        c_self_match,
        Severity::Soft,
    ),
    // WARDEN-LOCAL: named by no law, triggered from our own registry. The
    // drift ratchet only pins corpus->registry, so registered-but-no-law is
    // expected here and is not drift.
    ("shell.exit-code-through-pipe", c_pipe_rc, Severity::Soft),
    (
        "shell.gate-chained-into-commit",
        c_gate_chain,
        Severity::Soft,
    ),
    ("shell.posix-tmp-across-python", c_posix_tmp, Severity::Soft),
];

fn c_reset(ctx: &Ctx) -> Finding {
    // The trigger belongs to the check. Without it the `git status` measurement
    // below reports a dirty tree for every tool call in a dirty repo, and the
    // warden steers on ordinary work — see `git::is_hard_reset`.
    if !git::is_hard_reset(ctx.command) {
        return None;
    }
    git_reset_discards(ctx.cwd)
}
fn c_hooks(ctx: &Ctx) -> Finding {
    git::skips_hooks(ctx.command)
}
fn c_signing(ctx: &Ctx) -> Finding {
    git::bypasses_signing(ctx.command)
}
fn c_force_push(ctx: &Ctx) -> Finding {
    git::force_push_to_protected(ctx.command)
}
fn c_rewritten(ctx: &Ctx) -> Finding {
    incidents::push_into_rewritten_repo(ctx.command, ctx.cwd)
}
fn c_blanket(ctx: &Ctx) -> Finding {
    git::blanket_stage(ctx.command)
}
fn c_skip_ci(ctx: &Ctx) -> Finding {
    shell::skip_ci_marker(ctx.command)
}
fn c_osv(ctx: &Ctx) -> Finding {
    shell::osv_no_resolve(ctx.command)
}
fn c_git_show(ctx: &Ctx) -> Finding {
    shell::git_show_piped_into_a_counter(ctx.command)
}
fn c_pipe_rc(ctx: &Ctx) -> Finding {
    shell_local::exit_code_through_pipe(ctx.command)
}
fn c_gate_chain(ctx: &Ctx) -> Finding {
    shell_local::gate_chained_into_commit(ctx.command)
}
fn c_posix_tmp(ctx: &Ctx) -> Finding {
    shell_local::posix_tmp_across_python(ctx.command)
}
fn c_self_match(ctx: &Ctx) -> Finding {
    shell::process_query_self_match(ctx.command)
}

/// Run a registered check by ID. An unknown id is SILENT — never a crash, never
/// a fabricated finding.
pub fn run(check_id: &str, ctx: &Ctx) -> Finding {
    REGISTRY
        .iter()
        .find(|(id, _, _)| *id == check_id)
        .and_then(|(_, f, _)| f(ctx))
}

/// Whether this crate answers to an id at all.
///
/// THE REGISTRY IS INTERROGABLE ON PURPOSE. `run` alone cannot answer this — an
/// unknown id and a green check both return `None`, so nothing outside could
/// tell "not implemented" from "nothing to report". That is exactly the
/// distinction the drift ratchet has to make, and a gate that cannot tell
/// absence from silence is this estate's most reproducible defect.
pub fn is_registered(check_id: &str) -> bool {
    REGISTRY.iter().any(|(id, _, _)| *id == check_id)
}

pub fn severity_of(check_id: &str) -> Option<Severity> {
    REGISTRY
        .iter()
        .find(|(id, _, _)| *id == check_id)
        .map(|(_, _, sev)| *sev)
}

/// Every registered id, in registry order.
pub fn registered_ids() -> Vec<&'static str> {
    REGISTRY.iter().map(|(id, _, _)| *id).collect()
}

/// Run every registered check and return the findings that fired, in registry
/// order, each with its severity.
pub fn run_all(ctx: &Ctx) -> Vec<(&'static str, Severity, String)> {
    REGISTRY
        .iter()
        .filter_map(|(id, f, sev)| f(ctx).map(|finding| (*id, *sev, finding)))
        .collect()
}

/// What a `git reset --hard` in this directory would destroy, by name.
///
/// Errors are SILENT, deliberately: outside a repo, or with git unavailable,
/// nothing was measured — and reporting "could not measure" as a FINDING would
/// be inventing one. The estate's recurring failure is absence rendering as
/// success; this is the one place where silence is the honest reading, because
/// the caller only ever acts on a present finding.
pub fn git_reset_discards(cwd: &Path) -> Finding {
    let out = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["status", "--porcelain"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let items = doomed_paths(&text);
    if items.is_empty() {
        return None;
    }
    Some(describe(&items))
}

/// The paths a hard reset would take with it: every modified, staged or
/// untracked entry. `git status --porcelain` lines are `XY <path>`, and the path
/// starts at column 3.
fn doomed_paths(porcelain: &str) -> Vec<String> {
    porcelain
        .lines()
        .filter(|l| l.len() > 3)
        .map(|l| l[3..].trim().trim_matches('"').to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

/// A finding STATES WHAT IS. It carries the count so the reader can check it
/// against the list, and it gives no advice — advice is the weak form again, one
/// level down.
fn describe(items: &[String]) -> String {
    const SHOWN: usize = 12;
    let n = items.len();
    let head: Vec<&str> = items.iter().take(SHOWN).map(String::as_str).collect();
    let mut s = format!(
        "a hard reset here discards {} uncommitted item{}: {}",
        n,
        if n == 1 { "" } else { "s" },
        head.join(", ")
    );
    if n > SHOWN {
        s.push_str(&format!(", and {} more", n - SHOWN));
    }
    s
}
