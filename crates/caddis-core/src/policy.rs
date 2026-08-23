//! policy.rs — decide(env) (CARD-0001 step 3): v0 leidžia murmur/*, kita E-POLICY.
use crate::envelope::{validate, Envelope};

#[derive(Debug, PartialEq)]
pub struct Decision {
    pub allow: bool,
    pub reason: String,
}

pub fn decide(e: &Envelope) -> Decision {
    // D2 (r2-council 2026-08-16): murmur = telemetry STREAM, NOT envelope-family
    // (C-arch-1 ruling; arch: caddis-core never emits telemetry; rails emit it).
    // Envelope srityje murmur neegzistuoja — allow taisyklė pašalinta RED-FIRST.
    // Pasirašytiems mazgo-būsenos įrašams naudoti signal/node.* (ateina su schema v2).
    Decision {
        allow: false,
        reason: format!(
            "E-POLICY: type not allowed in v0: {} (signed node-state: signal/node.*)",
            e.r#type
        ),
    }
}

/// Pilnas įėjimo taškas (validate + rule) — naudoti kanale.
#[allow(clippy::too_many_arguments)] // mirrors envelope::validate — the same 8 schema wire fields pass through unchanged
pub fn admit(
    v: u8,
    id: &str,
    idem_key: &str,
    typ: &str,
    from: &str,
    to: &str,
    body: &str,
    ts: &str,
) -> Result<Envelope, String> {
    let env = validate(v, id, idem_key, typ, from, to, body, ts)
        .map_err(|e| format!("{}: {}", e.code, e.why))?;
    let d = decide(&env);
    if d.allow {
        Ok(env)
    } else {
        Err(d.reason)
    }
}
