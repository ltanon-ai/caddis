//! hops_core.rs — canary hops 2–6, the ones that exercise the caddis-core
//! substrate: envelope, policy, idempotency, ledger append and read-back.
//! Split out of canary.rs under the 280-line law.
//!
//! The seam is WHICH SUBSTRATE a hop proves. Everything here calls into
//! caddis-core and touches at most one scratch file; the organ hops (7–11)
//! live next door in `hops_organs.rs`. Each hop is a function returning its
//! own `Hop`, which is the shape the original file had already chosen for
//! hops 7–9 — this only finishes the job for the hops that were still
//! inline, and that inlining was what pushed `run_canary` to CCN 15.
//!
//! Two hops can ABORT the whole chain (a broken validator or an unopenable
//! ledger makes every later hop meaningless), so they return `Result<_, Hop>`
//! where the `Err` carries the red hop that ends the run.

use std::path::Path;

use caddis_core::envelope::{self, Envelope};
use caddis_core::idempotency::Idempotency;
use caddis_core::ledger::Ledger;
use caddis_core::policy;

use crate::hop::{ok, red, Hop};

/// Hop 2 — envelope: strict form accepts a good frame, rejects a bad one.
/// `Err` = the chain cannot continue without a trustworthy validator.
pub(crate) fn envelope_hop(ts: &str) -> Result<(Hop, Envelope), Hop> {
    let env_ok = envelope::validate(
        1,
        "canary-0001",
        "idem-canary-1",
        "signal/canary.hop",
        "canary",
        "ledger",
        "{\"hop\":2}",
        ts,
    );
    let env_bad = envelope::validate(9, "canary-0001", "k", "t", "a", "b", "x", ts);
    match (env_ok, env_bad) {
        (Ok(e), Err(_)) => Ok((
            ok(
                2,
                "envelope",
                None,
                "accepts strict form, rejects bad version",
            ),
            e,
        )),
        (Ok(_), Ok(_)) => Err(red(2, "envelope", "validator accepted a bad version", None)),
        (Err(e), _) => Err(red(2, "envelope", &format!("{}: {}", e.code, e.why), None)),
    }
}

/// Hop 3 — policy: the LAW is alive when it denies what v0 forbids.
pub(crate) fn policy_hop(env: &Envelope) -> Hop {
    let d = policy::decide(env);
    if !d.allow && d.reason.starts_with("E-POLICY") {
        ok(3, "policy", None, "denies non-admitted type (E-POLICY)")
    } else {
        red(
            3,
            "policy",
            &format!("policy did not deny as v0 law requires: {}", d.reason),
            None,
        )
    }
}

/// Hop 4 — idempotency: first key passes, the replay is caught.
pub(crate) fn idempotency_hop(env: &Envelope) -> Hop {
    let mut idem = Idempotency::new();
    let first = idem.check(&env.idem_key);
    let replay = idem.check(&env.idem_key);
    match (first, replay) {
        (Ok(()), Err(e)) if e.starts_with("E-IDEM") => {
            ok(4, "idempotency", None, "replay detected (E-IDEM)")
        }
        _ => red(4, "idempotency", "replay was not detected", None),
    }
}

/// Hop 5, first half — open the real ledger. `Err` ends the run: hops 5, 6
/// and 7 all read this file, so there is nothing left to prove without it.
pub(crate) fn open_ledger(path: &Path) -> Result<Ledger, Hop> {
    Ledger::open(path).map_err(|e| red(5, "ledger-append", &e.to_string(), None))
}

/// Hop 5 — ledger append on the real substrate.
pub(crate) fn ledger_append_hop(ledger: &mut Ledger, env: &Envelope) -> Hop {
    match ledger.append(env) {
        Ok(seq) if seq >= 1 => ok(5, "ledger-append", None, &format!("seq {seq}")),
        Ok(_) => red(5, "ledger-append", "seq did not advance", None),
        Err(e) => red(5, "ledger-append", &e.to_string(), None),
    }
}

/// Hop 6 — ledger read-back: reopen the file; every row must be intact.
pub(crate) fn ledger_readback_hop(path: &Path) -> Hop {
    match Ledger::open(path) {
        Ok(l) if l.unreadable() == 0 && l.seq() >= 1 => {
            ok(6, "ledger-readback", None, "all rows intact")
        }
        Ok(l) => red(
            6,
            "ledger-readback",
            &format!("unreadable rows: {}", l.unreadable()),
            None,
        ),
        Err(e) => red(6, "ledger-readback", &e.to_string(), None),
    }
}
