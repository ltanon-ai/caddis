//! store.rs — the lease store over the append-only `pool/leases.jsonl`
//! journal (CARD-BITYNAS-1).
//!
//! SINGLE WRITER: `&mut self` serializes every check-then-act — the journal
//! buys crash recovery, not cross-process exclusion. Two processes claiming
//! through two `LeaseStore`s on one file can lose claims; the bitynas
//! daemon organ (a later unit) is the designated single writer. Do not
//! "fix" this here — promoting caddis-core's lock law is a later card.
//!
//! APPEND LAW: every mutation first decides on the in-memory index, then
//! journals the row(s) with ONE `write_all` of the complete `\n`-terminated
//! line, then returns. The file is created lazily on first append.
//!
//! # Panics
//!
//! Any mutation panics if the journal append fails (disk full, permissions):
//! the on-disk state is then unreliable for a single-writer store and the
//! process must not keep leasing. The in-memory index is only ever updated
//! AFTER a successful append, so a panicked store is stale, never wrong.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use caddis_organs::util_time::iso8601_now;

use crate::journal::{self, Row};
use crate::lease::{BusyError, LeaseOwner, LeaseRecord, PeremptionEvent, DEFAULT_TTL_S};

/// The one preemption cause this unit knows.
const CAUSE_TTL: &str = "ttl_expired";

pub struct LeaseStore {
    path: PathBuf,
    index: BTreeMap<String, LeaseRecord>,
    pending: Vec<PeremptionEvent>,
    unreadable: usize,
}

impl LeaseStore {
    /// Open the journal at `path` (created lazily on first append) and
    /// rebuild the in-memory index by folding its rows in order. Torn or
    /// unknown rows are counted in [`unreadable`](Self::unreadable) and
    /// skipped — never fatal, never a panic.
    pub fn open(path: &Path) -> std::io::Result<Self> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let mut store = LeaseStore {
            path: path.to_path_buf(),
            index: BTreeMap::new(),
            pending: Vec::new(),
            unreadable: 0,
        };
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let mut rows = Vec::new();
                for line in text.lines() {
                    if line.trim().is_empty() {
                        continue;
                    }
                    match journal::parse(line) {
                        Some(row) => rows.push(row),
                        None => store.unreadable += 1,
                    }
                }
                store.index = journal::fold(rows.into_iter());
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        Ok(store)
    }

    /// Rows skipped at open (torn tail, unknown op) — a health signal for
    /// the daemon, not an error: the journal is append-only and skipped
    /// rows are already lost to the crash that tore them.
    pub fn unreadable(&self) -> usize {
        self.unreadable
    }

    /// The live record for `slot_id`, if held.
    pub fn held(&self, slot_id: &str) -> Option<&LeaseRecord> {
        self.index.get(slot_id)
    }

    /// Atomically claim `slot_id` for `owner`.
    ///
    /// Free (or never taken) → a new lease with `ttl_s = DEFAULT_TTL_S`.
    /// Held and fresh → `Err(BusyError)` carrying the holder's full
    /// identity. Held but stale → the old lease is preempted: a reclaim
    /// row plus a fresh claim row are journaled, the [`PeremptionEvent`]
    /// is queued for [`events()`](Self::events) — perėmimas NIEKADA
    /// tylus, a reclaim is never silent — and the new lease is returned.
    ///
    /// # Panics
    ///
    /// On an empty (or whitespace-only) `slot_id`/`lane`: an empty id is a
    /// caller bug, not a busy slot, and `BusyError` cannot say so. Plus
    /// the type-level append law.
    pub fn claim(
        &mut self,
        slot_id: &str,
        lane: &str,
        owner: LeaseOwner,
    ) -> Result<LeaseRecord, BusyError> {
        assert!(
            !slot_id.trim().is_empty() && !lane.trim().is_empty(),
            "bitynas: claim needs a non-empty slot_id and lane"
        );
        let now = iso8601_now();
        let held = self.index.get(slot_id).cloned();
        match held {
            None => {
                let rec = fresh_record(slot_id, lane, &owner, DEFAULT_TTL_S);
                self.append(&journal::line(&Row::Claim { record: rec.clone() }));
                self.index.insert(slot_id.to_string(), rec.clone());
                Ok(rec)
            }
            Some(held) if held.is_stale(&now, &held.heartbeat_at_utc) => {
                self.append(&journal::line(&Row::Reclaim {
                    slot_id: slot_id.to_string(),
                    at_utc: now.clone(),
                    cause: CAUSE_TTL.to_string(),
                    new_owner: Some(owner.clone()),
                    previous: held.clone(),
                }));
                let rec = fresh_record(slot_id, lane, &owner, DEFAULT_TTL_S);
                self.append(&journal::line(&Row::Claim { record: rec.clone() }));
                self.pending.push(PeremptionEvent {
                    slot_id: slot_id.to_string(),
                    lane: held.lane.clone(),
                    previous: held,
                    new_owner: Some(owner),
                    at_utc: now.clone(),
                    cause: CAUSE_TTL.to_string(),
                });
                self.index.insert(slot_id.to_string(), rec.clone());
                Ok(rec)
            }
            Some(held) => Err(BusyError { holder: held }),
        }
    }

    /// Release the lease. Absent slot → `Ok(())` (idempotent, POSIX unlink
    /// semantics). Held by someone else → `Err(BusyError)` naming them.
    pub fn release(&mut self, slot_id: &str, owner: &LeaseOwner) -> Result<(), BusyError> {
        match self.index.get(slot_id).cloned() {
            None => Ok(()),
            Some(held) if held.owner() == *owner => {
                let now = iso8601_now();
                self.append(&journal::line(&Row::Release {
                    slot_id: slot_id.to_string(),
                    at_utc: now,
                }));
                self.index.remove(slot_id);
                Ok(())
            }
            Some(held) => Err(BusyError { holder: held }),
        }
    }

    /// Refresh the lease heartbeat. Absent → `Ok(None)` (the caller sees
    /// the lease is gone). Wrong owner → `Err(BusyError)`. Right owner →
    /// the refreshed record.
    pub fn heartbeat(
        &mut self,
        slot_id: &str,
        owner: &LeaseOwner,
    ) -> Result<Option<LeaseRecord>, BusyError> {
        match self.index.get(slot_id).cloned() {
            None => Ok(None),
            Some(held) if held.owner() == *owner => {
                let now = iso8601_now();
                self.append(&journal::line(&Row::Heartbeat {
                    slot_id: slot_id.to_string(),
                    at_utc: now.clone(),
                }));
                let mut rec = held;
                rec.heartbeat_at_utc = now;
                self.index.insert(slot_id.to_string(), rec.clone());
                Ok(Some(rec))
            }
            Some(held) => Err(BusyError { holder: held }),
        }
    }

    /// Free every lease whose heartbeat is older than its TTL, returning
    /// the [`PeremptionEvent`]s DIRECTLY — they are not also queued for
    /// [`events()`](Self::events): a preemption is never silent, but it is
    /// also never reported twice.
    pub fn sweep(&mut self, now_utc: &str) -> Vec<PeremptionEvent> {
        let stale: Vec<String> = self
            .index
            .values()
            .filter(|r| r.is_stale(now_utc, &r.heartbeat_at_utc))
            .map(|r| r.slot_id.clone())
            .collect();
        let mut events = Vec::new();
        for slot_id in stale {
            if let Some(previous) = self.index.get(&slot_id).cloned() {
                self.append(&journal::line(&Row::Reclaim {
                    slot_id: slot_id.clone(),
                    at_utc: now_utc.to_string(),
                    cause: CAUSE_TTL.to_string(),
                    new_owner: None,
                    previous: previous.clone(),
                }));
                self.index.remove(&slot_id);
                events.push(PeremptionEvent {
                    slot_id,
                    lane: previous.lane.clone(),
                    previous,
                    new_owner: None,
                    at_utc: now_utc.to_string(),
                    cause: CAUSE_TTL.to_string(),
                });
            }
        }
        events
    }

    /// Drain the pending [`PeremptionEvent`]s (claim-preemptions that
    /// happened since the last drain).
    pub fn events(&mut self) -> Vec<PeremptionEvent> {
        std::mem::take(&mut self.pending)
    }

    /// One complete `\n`-terminated line, one `write_all` — atomic per
    /// syscall on Windows `FILE_APPEND_DATA` (the CARD-0108 law; never
    /// `writeln!`, which can tear).
    fn append(&self, row_line: &str) {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .unwrap_or_else(|e| panic!("bitynas: journal open failed ({}): {e}", self.path.display()));
        f.write_all(row_line.as_bytes())
            .unwrap_or_else(|e| panic!("bitynas: journal write failed ({}): {e}", self.path.display()));
    }
}

/// A brand-new lease stamped `now` for both clocks.
fn fresh_record(slot_id: &str, lane: &str, owner: &LeaseOwner, ttl_s: u64) -> LeaseRecord {
    let now = iso8601_now();
    LeaseRecord {
        slot_id: slot_id.to_string(),
        lane: lane.to_string(),
        session_id: owner.session_id.clone(),
        host: owner.host.clone(),
        pid: owner.pid,
        repo: None,
        card: None,
        taken_at_utc: now.clone(),
        ttl_s,
        heartbeat_at_utc: now,
    }
}
