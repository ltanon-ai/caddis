//! shell_local.rs — checks the warden enforces that NO law names.
//!
//! WHY A SECOND SHELL FILE. `shell.rs` holds the checks whose ids appear in the
//! estate's shared `laws.json`; the drift ratchet pins that set. These three are
//! WARDEN-LOCAL: promoted straight from the lesson bank, triggered from our own
//! registry, named by no law. Keeping them apart means a reader can tell at a
//! glance which checks the shared corpus is entitled to expect — and the ratchet
//! stays honest about covering only the corpus.
//!
//! ⛔ THE SELECTION RULE DISQUALIFIED MOST CANDIDATES, which is the point. The
//! corpus holds 134 laws; only 9 name a check, and of the 125 prose-only ones
//! exactly THREE have a trigger tight enough to be a check: the trigger must be
//! nearly ALWAYS wrong. Rejected on that ground, and named here so nobody
//! re-derives the analysis: `recorded-pid-is-not-identity` (fires on any process
//! listing, most of which are fine), `substring-search-is-not-membership-test`
//! (`grep -c` is correct constantly), and `run-a-falsifier-or-mutation-probe`
//! (its trigger is the DECLARATION, i.e. it fires when you are doing the right
//! thing).
//!
//! ALL THREE ARE SOFT. Each is a wrong MEASUREMENT or a process slip, never an
//! irreversible act, and a denial would block work the operator legitimately
//! asked for. Every one of them fired on the session that wrote them, and was
//! right each time — which is the only field evidence that matters here.

use super::Finding;
use crate::checks::cmdline::{segments, segments_detailed};
use crate::checks::gitgrammar::is_git_subcommand;

/// RED when a pipeline's exit status is then read with `$?`.
///
/// `cmd | tail -5; echo rc=$?` reports TAIL's status, so a command that exited
/// 127 reads as success — a false GREEN, the polarity nobody goes back to
/// investigate. Provenance: the estate's `pipe-eats-the-exit-code` law, which
/// fired on the session that wrote this check and was right.
///
/// GREEN, and these matter more than the red: `${PIPESTATUS[0]}` and
/// `set -o pipefail` are the prescribed fixes, and a check that fired on them
/// would punish the remedy it exists to encourage.
pub fn exit_code_through_pipe(command: &str) -> Finding {
    let lowered = command.to_lowercase();
    if lowered.contains("pipestatus") || lowered.contains("pipefail") {
        return None;
    }
    let mut after_a_pipe = false;
    for segment in segments_detailed(command) {
        if after_a_pipe && segment.tokens.iter().any(|t| t.contains("$?")) {
            return Some(format!(
                "`$?` is read after a pipeline, so it reports the LAST process rather than the one \
                 being judged: `{}`. A command that exited 127 reads as success here. Capture the \
                 status directly (`cmd > out 2>&1; rc=$?`) or use `${{PIPESTATUS[0]}}`.",
                segment.tokens.join(" ")
            ));
        }
        // ASSIGNED, never latched: `$?` reads the command IMMEDIATELY
        // before it, so being "after a pipeline" is a fact about ONE
        // segment, not a state the rest of the line inherits. Latching it
        // condemned `a | b; c > out 2>&1; rc=$?` — the very shape the
        // message above prescribes as the remedy.
        after_a_pipe = segment.sep_before.as_deref() == Some("|");
    }
    None
}

/// Commands whose exit code is the whole point of running them.
///
/// A NAMED, SMALL list on purpose. An open-ended "any command" version would
/// fire on `cd repo && git add file`, which is correct and constant — and a
/// check that fires on the ordinary case is the wallpaper this crate keeps
/// warning about.
const GATE_CMDS: &[&str] = &[
    "pytest",
    "tox",
    "ruff",
    "mypy",
    "eslint",
    "lizard",
    "codespell",
    "typos",
    "gitleaks",
    "shellcheck",
];

fn is_gate_segment(tokens: &[String]) -> bool {
    if tokens.iter().any(|t| t.contains("gate.py")) {
        return true;
    }
    let head = match tokens.first() {
        Some(h) => h.rsplit(['/', '\\']).next().unwrap_or(h),
        None => return false,
    };
    let head = head.trim_end_matches(".exe");
    if GATE_CMDS.contains(&head) {
        return true;
    }
    let sub = tokens.get(1).map(String::as_str).unwrap_or("");
    match head {
        "cargo" => matches!(sub, "test" | "clippy" | "fmt"),
        "go" => sub == "test",
        "npm" | "pnpm" | "yarn" => matches!(sub, "test" | "run"),
        "make" => matches!(sub, "test" | "check" | "lint"),
        _ => false,
    }
}

/// RED when a gate or test run is chained by `&&` into `git commit`/`git add`.
///
/// The chain swallows the gate's exit code and commits on red. Provenance: the
/// estate's `gate-then-commit-never-chain` law — run the gate, READ its output,
/// then commit as a separate call.
///
/// Order matters and is checked: a gate AFTER the commit is not this hazard, so
/// `git commit -m x && pytest` stays silent.
pub fn gate_chained_into_commit(command: &str) -> Finding {
    let all = segments_detailed(command);
    let mut seen_gate: Option<String> = None;
    for segment in &all {
        if is_gate_segment(&segment.tokens) {
            seen_gate = Some(segment.tokens.join(" "));
            continue;
        }
        let chained = segment.sep_before.as_deref() == Some("&&");
        let commits = is_git_subcommand(&segment.tokens, "commit")
            || is_git_subcommand(&segment.tokens, "add");
        if let (true, true, Some(gate)) = (chained, commits, seen_gate.clone()) {
            return Some(format!(
                "`{gate}` is chained with `&&` into `{}`: the chain swallows the gate's exit code \
                 path and commits on red the moment the shell's short-circuit is misread. Run the \
                 gate, READ its output, then commit as a separate command.",
                segment.tokens.join(" ")
            ));
        }
    }
    None
}

/// RED when a POSIX `/tmp` path is handed across the bash-to-Windows-Python
/// boundary inside ONE command.
///
/// On this harness bash resolves `/tmp` to the MSYS mount while Windows Python
/// resolves it drive-relative to `C:\tmp`, so the write and the read touch
/// different files. **The dangerous variant is not the crash:** if anything
/// already exists at the Python-side path from an earlier run, it is read and
/// answered confidently — a silent wrong result where the loud one at least
/// stops you.
pub fn posix_tmp_across_python(command: &str) -> Finding {
    posix_tmp_across_python_on(command, cfg!(windows))
}

/// The platform is a PARAMETER so the check is testable in both worlds.
///
/// ⚠ GATED ON PURPOSE. This hazard exists because two interpreters disagree
/// about `/tmp`; on a POSIX host they agree and the finding would describe a
/// hazard that does not exist. A check that fires where its premise is false is
/// the false positive that gets a mechanism switched off.
pub fn posix_tmp_across_python_on(command: &str, windows: bool) -> Finding {
    if !windows || !command.contains("/tmp/") {
        return None;
    }
    let invokes_python = segments(command).iter().any(|tokens| {
        tokens.first().is_some_and(|head| {
            let base = head.rsplit(['/', '\\']).next().unwrap_or(head);
            let base = base.trim_end_matches(".exe");
            base == "python" || base == "python3" || base == "py"
        })
    });
    if !invokes_python {
        return None;
    }
    Some(
        "a POSIX `/tmp/...` path is used in the same command as a Windows Python: bash resolves \
         `/tmp` to the MSYS mount, Python resolves it drive-relative, so the write and the read \
         touch DIFFERENT files. The crash is the lucky case — a stale file at the Python-side path \
         is read and answered confidently. Use one drive-qualified path on both sides."
            .to_string(),
    )
}
