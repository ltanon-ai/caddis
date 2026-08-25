//! caddis-core — the nervous kernel v0 (CARD-0001; LADDER R1a; WORKSPACE C3: sync TCB).
//! Channel: envelope -> policy -> idempotency -> ledger (same contracts as the
//! node prototype in the seed; that prototype was the proof, THIS is the organ).
//! Modifiable-by-design (CD-021): every pub item cites its canon origin.

pub mod envelope;
pub mod footer_state;
pub mod idempotency;
pub mod ledger;
// The ledger's mutual exclusion, split under the 280-line law (CARD-0108).
// Private: callers get exclusion as a property of `append`, never as a knob.
mod ledger_lock;
pub mod policy;

pub const VERSION: &str = "0.1.0";
