//! caddis-warden — THE CONSCIOUSNESS (sąmonė) for omp.
//!
//! omp owns the MECHANISM: a `tool_call` hook that fires before the approval
//! gate and can block a tool from executing. It has no opinion about WHAT
//! should be blocked. This crate is that opinion — the estate's law, enforced
//! mechanically at the tool boundary instead of remembered by a tired session.
//!
//! THREE VERDICTS, and the middle one is the reason this is a consciousness and
//! not a firewall:
//! - `Deny` — an unambiguous violation. The tool never runs; the model is told
//!   why, in its own error channel.
//! - `Steer` — the action is allowed, AND a banked law is delivered at the
//!   moment it applies. This is the jit-laws mechanism: doctrine arrives when
//!   it is relevant, not at session start where it is read once and forgotten.
//! - `Allow` — nothing to say.
//!
//! EVERY verdict is recorded. The ledger is append-only and is the artifact the
//! whole trust argument rests on; a warden that decides without recording is
//! just a mood.
//!
//! DESIGN CONSTRAINT THAT OUTRANKS COVERAGE: a warden that blocks legitimate
//! work gets switched off, and a switched-off warden protects nothing. So
//! `Deny` is reserved for what is unambiguous, and everything a reasonable
//! engineer might do on purpose is `Steer` instead. Being ignorable is the
//! failure mode; being obstructive is how you get ignored.

pub mod allowlist;
pub mod attest;
pub mod attest_verify;
pub mod card;
pub mod card_state;
pub mod checks;
pub mod cli;
pub mod identity;
pub mod law;
pub mod laws;
pub mod propose;
pub mod receipt;
pub mod receipt_report;
pub mod replay;
pub mod report;
pub mod wire;

// Rendering for `replay`, and the ONE ledger-row parser both `replay` and
// `report` read through. Neither is public API: the binary reaches the ledger
// through `replay::run` and `report::run`, and `rows` stays `pub(crate)`
// deliberately (rows.rs:1-4) so a second copy of the row parser cannot appear.
// Locating a card's window in the ledger. Internal: callers reach it through
// `attest`, and it speaks in `Row`, which stays pub(crate) by design.
mod attest_window;
// The bundle field readers `attest --verify` stands on. Internal by design.
mod json_read;
mod replay_report;
mod rows;

/// What the warden decided about one tool call.
#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    /// Run it. Nothing to say.
    Allow,
    /// Run it, but deliver this law to the model at this moment.
    Steer { law: String, why: String },
    /// Do not run it. `reason` reaches the model as a tool error.
    Deny { reason: String },
}

impl Verdict {
    pub fn is_deny(&self) -> bool {
        matches!(self, Verdict::Deny { .. })
    }

    /// The ledger's one-word name for this verdict.
    pub fn tag(&self) -> &'static str {
        match self {
            Verdict::Allow => "allow",
            Verdict::Steer { .. } => "steer",
            Verdict::Deny { .. } => "deny",
        }
    }
}

/// One tool call, as omp hands it over.
#[derive(Debug, Clone)]
pub struct ToolCall {
    /// omp's tool name: `bash`, `edit`, `write`, `read`, ...
    pub tool: String,
    /// The command line, for `bash`.
    pub command: String,
    /// The target path, for file tools.
    pub path: String,
    /// The content being written, for file tools.
    pub content: String,
}

impl ToolCall {
    pub fn new(tool: &str) -> Self {
        Self {
            tool: tool.to_string(),
            command: String::new(),
            path: String::new(),
            content: String::new(),
        }
    }
    pub fn command(mut self, c: &str) -> Self {
        self.command = c.to_string();
        self
    }
    pub fn path(mut self, p: &str) -> Self {
        self.path = p.to_string();
        self
    }
    pub fn content(mut self, c: &str) -> Self {
        self.content = c.to_string();
        self
    }

    /// Everything this call would put into the world, as one searchable text.
    /// A rule that scans "the payload" must not have to remember which field a
    /// given tool happens to use — `bash` hides its payload in `command`,
    /// `write` in `content`, and a rule that checks only one of them is a rule
    /// with a hole shaped like the other.
    pub fn payload(&self) -> String {
        format!("{}\n{}", self.command, self.content)
    }
}

/// The whole decision for one tool call, judged against the process's current
/// directory — which is omp's working directory, because the warden is spawned
/// by omp's own hook.
pub fn decide(call: &ToolCall) -> Verdict {
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    decide_in(call, &cwd)
}

/// The same decision against an EXPLICIT directory. Checks shell out to git, so
/// a test that cannot choose the repo it is judging is testing the workshop it
/// happens to run in rather than the law.
pub fn decide_in(call: &ToolCall, cwd: &std::path::Path) -> Verdict {
    law::apply(call, cwd)
}
