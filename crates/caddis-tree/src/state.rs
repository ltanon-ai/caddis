//! state.rs — tree-state: the append-only event log and every invariant
//! the quorum pinned. Writes are atomic (whole log to temp, rename over);
//! seq is monotonic and a mismatched log is refused at load; one writer
//! per log (the orchestrating session) — a second session is refused;
//! attempt and cost caps are GLOBAL per goal and checked PROSPECTIVELY
//! (used + incoming > cap refuses); the in-memory view is ONLY ever
//! rebuilt from the file, which is what makes kill-mid-tree resume
//! possible. A leaf is pinned DONE by a PASSED gate or a strong close —
//! a failed gate leaves the leaf retryable.

use crate::codec::{event_line, parse_line};
use crate::event::TreeEvent;
pub use crate::event::{Caps, EventKind, Lane, StateErr};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

pub struct TreeState {
    log: PathBuf,
    writer: String,
    caps: Caps,
    events: Vec<TreeEvent>,
    attempts_used: u32,
    cost_used: u64,
    parent_of: HashMap<String, String>,
    children_of: HashMap<String, Vec<String>>,
    live: HashSet<String>,
    gated: HashSet<String>,
    strong_closed: HashSet<String>,
}

impl TreeState {
    /// Fresh log for `writer` (single-writer law: the orchestrating
    /// session). Refuses to adopt an existing non-empty log — that is
    /// `load`'s job, never a silent overwrite.
    pub fn new(log: impl AsRef<Path>, writer: &str, caps: Caps) -> Result<Self, StateErr> {
        let log = log.as_ref().to_path_buf();
        if log.is_file() && fs::metadata(&log).map(|m| m.len() > 0).unwrap_or(false) {
            return Err(StateErr::WriterConflict);
        }
        Ok(Self::replay(log, writer.to_string(), caps, Vec::new()))
    }

    /// Resume: rebuild the WHOLE view from the file alone; the writer is
    /// whoever wrote the log.
    pub fn load(log: impl AsRef<Path>, caps: Caps) -> Result<Self, StateErr> {
        Self::load_as(log, "", caps)
    }

    /// Resume but only as `writer` — any other session is refused.
    pub fn load_as(log: impl AsRef<Path>, writer: &str, caps: Caps) -> Result<Self, StateErr> {
        let log = log.as_ref().to_path_buf();
        let text = fs::read_to_string(&log).map_err(|e| StateErr::Io(e.to_string()))?;
        let mut events = Vec::new();
        let mut last = 0u64;
        for line in text.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let ev = parse_line(line)?;
            if ev.seq != last + 1 {
                return Err(StateErr::SeqMismatch);
            }
            last = ev.seq;
            events.push(ev);
        }
        let logged = events.first().map(|e| e.writer.clone()).unwrap_or_default();
        if !writer.is_empty() && logged != writer {
            return Err(StateErr::WriterConflict);
        }
        let mut st = Self::replay(log, logged, caps, events);
        st.recount();
        Ok(st)
    }

    fn replay(log: PathBuf, writer: String, caps: Caps, events: Vec<TreeEvent>) -> Self {
        let mut st = Self {
            log,
            writer,
            caps,
            attempts_used: 0,
            cost_used: 0,
            parent_of: HashMap::new(),
            children_of: HashMap::new(),
            live: HashSet::new(),
            gated: HashSet::new(),
            strong_closed: HashSet::new(),
            events: Vec::new(),
        };
        for ev in events {
            st.fold(&ev.kind);
            st.events.push(ev);
        }
        st
    }

    /// Rebuild scalar totals (load path only; append keeps them current).
    fn recount(&mut self) {
        self.attempts_used = 0;
        self.cost_used = 0;
        for ev in &self.events {
            if let EventKind::LeafDispatch { cost, .. } = &ev.kind {
                self.attempts_used += 1;
                self.cost_used += cost;
            }
        }
    }

    fn fold(&mut self, kind: &EventKind) {
        match kind {
            EventKind::PlanAccepted { plan, children } => {
                self.children_of.insert(plan.clone(), children.clone());
                for c in children {
                    self.parent_of.insert(c.clone(), plan.clone());
                }
            }
            EventKind::SubtreeLive { parent } => {
                self.live.insert(parent.clone());
            }
            EventKind::SubtreeClosed { parent } => {
                self.live.remove(parent);
            }
            EventKind::LeafGated { card, pass: true } => {
                self.gated.insert(card.clone());
            }
            EventKind::StrongClose { card } => {
                self.strong_closed.insert(card.clone());
            }
            _ => {}
        }
    }

    /// Append with every invariant enforced, then persist atomically.
    /// Cost honesty: the dispatch cost is spent before this call (the
    /// executor already ran); `can_dispatch` exists so the walker can
    /// refuse BEFORE spending, and this refusal is the authoritative one.
    pub fn append(&mut self, kind: EventKind) -> Result<(), StateErr> {
        self.check(&kind)?;
        let ev = TreeEvent {
            seq: self.events.len() as u64 + 1,
            writer: self.writer.clone(),
            kind,
        };
        self.fold(&ev.kind);
        if let EventKind::LeafDispatch { cost, .. } = &ev.kind {
            self.attempts_used += 1;
            self.cost_used += cost;
        }
        self.events.push(ev);
        self.persist()
    }

    fn check(&self, kind: &EventKind) -> Result<(), StateErr> {
        let intaked = self
            .events
            .iter()
            .any(|e| matches!(e.kind, EventKind::GoalIntake { .. }));
        match kind {
            EventKind::GoalIntake { .. } if intaked => Err(StateErr::AlreadyIntaked),
            EventKind::PlanAccepted { .. } | EventKind::LeafDispatch { .. } if !intaked => {
                Err(StateErr::OrphanCard)
            }
            EventKind::LeafDispatch {
                card, lane, cost, ..
            } => self.check_dispatch(card, lane, *cost),
            _ => Ok(()),
        }
    }

    fn check_dispatch(&self, card: &str, lane: &Lane, cost: u64) -> Result<(), StateErr> {
        let parent = self.parent_of.get(card);
        if parent.is_none() {
            return Err(StateErr::OrphanCard);
        }
        if self.gated.contains(card) || self.strong_closed.contains(card) {
            return Err(StateErr::AlreadyDone);
        }
        if let Some(p) = parent {
            if matches!(lane, Lane::Strong) && self.live.contains(p) {
                return Err(StateErr::StrongUnderLive);
            }
        }
        if self.attempts_used + 1 > self.caps.max_attempts {
            return Err(StateErr::CapAttempts);
        }
        if self.cost_used + cost > self.caps.max_cost {
            return Err(StateErr::CapCost);
        }
        Ok(())
    }

    /// Pre-flight for the walker: refuse BEFORE the executor spends.
    pub fn can_dispatch(&self) -> Result<(), StateErr> {
        let intaked = self
            .events
            .iter()
            .any(|e| matches!(e.kind, EventKind::GoalIntake { .. }));
        if !intaked {
            return Err(StateErr::OrphanCard);
        }
        if self.attempts_used >= self.caps.max_attempts {
            return Err(StateErr::CapAttempts);
        }
        if self.cost_used >= self.caps.max_cost {
            return Err(StateErr::CapCost);
        }
        Ok(())
    }

    /// Atomic persist: whole log to a temp sibling, then rename over. A
    /// crash mid-write never leaves a torn line in the real log.
    fn persist(&self) -> Result<(), StateErr> {
        let tmp = self.log.with_extension("jsonl.tmp");
        let mut body = String::new();
        for ev in &self.events {
            body.push_str(&event_line(ev));
        }
        let mut f = fs::File::create(&tmp).map_err(|e| StateErr::Io(e.to_string()))?;
        f.write_all(body.as_bytes())
            .map_err(|e| StateErr::Io(e.to_string()))?;
        drop(f);
        fs::rename(&tmp, &self.log).map_err(|e| StateErr::Io(e.to_string()))?;
        Ok(())
    }

    // ── read accessors (the walker's whole world-view) ─────────────────
    pub fn writer(&self) -> &str {
        &self.writer
    }

    pub fn seq(&self) -> u64 {
        self.events.len() as u64
    }

    pub fn events(&self) -> &[TreeEvent] {
        &self.events
    }

    pub fn dispatched(&self, card: &str) -> u32 {
        self.events
            .iter()
            .filter(|e| matches!(&e.kind, EventKind::LeafDispatch { card: c, .. } if c == card))
            .count() as u32
    }

    pub fn gated_ok(&self, card: &str) -> bool {
        self.gated.contains(card)
    }

    pub fn parent_of(&self, card: &str) -> Option<&String> {
        self.parent_of.get(card)
    }

    pub fn is_live(&self, parent: &str) -> bool {
        self.live.contains(parent)
    }
}
