//! event.rs — the tree event vocabulary (BC3). Serialization lives in
//! codec.rs, split under the 280-line law.

/// Per-goal GLOBAL caps: total attempts across every leaf, and cumulative
/// cost units — both refuse further dispatch once reached.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Caps {
    pub max_attempts: u32,
    pub max_cost: u64,
}

/// Who acted. The strong lane closes; only the weak lanes dispatch under a
/// LIVE subtree (BC3 law).
#[derive(Debug, Clone, PartialEq)]
pub enum Lane {
    Strong,
    Weak(String),
}

impl Lane {
    pub(crate) fn tag(&self) -> String {
        match self {
            Lane::Strong => "strong".into(),
            Lane::Weak(m) => format!("weak:{m}"),
        }
    }

    pub(crate) fn from_tag(v: &str) -> Lane {
        match v.strip_prefix("weak:") {
            Some(m) => Lane::Weak(m.to_string()),
            None => Lane::Strong,
        }
    }
}

/// The append-only event vocabulary (BC3 failure map included).
#[derive(Debug, Clone, PartialEq)]
pub enum EventKind {
    GoalIntake {
        root_red: String,
    },
    PlanAccepted {
        plan: String,
        children: Vec<String>,
    },
    SubtreeLive {
        parent: String,
    },
    SubtreeClosed {
        parent: String,
    },
    LeafDispatch {
        card: String,
        attempt: u32,
        cost: u64,
        lane: Lane,
    },
    LeafGated {
        card: String,
        pass: bool,
    },
    BubbleUp {
        from: String,
        to: String,
    },
    ReplanParent {
        parent: String,
        reason: String,
    },
    StrongClose {
        card: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct TreeEvent {
    pub seq: u64,
    pub writer: String,
    pub kind: EventKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StateErr {
    /// A parsed event's seq is not last+1 — the log is torn or tampered.
    SeqMismatch,
    /// A second session tried to own a log that already has a writer.
    WriterConflict,
    CapAttempts,
    CapCost,
    AlreadyIntaked,
    /// No accepted plan names this card — nothing to dispatch.
    OrphanCard,
    AlreadyDone,
    StrongUnderLive,
    /// Goal intake without an authored root integration RED.
    NoRootRed,
    PlanGates(String),
    Io(String),
}
