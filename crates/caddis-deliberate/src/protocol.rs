//! protocol.rs — P0 slice 2: [`Protocol`] / [`Convening`] / [`Verdict`] and
//! the PROTOCOL PIN (F3).
//!
//! Ruling provenance:
//!
//! - **F3** the protocol is PINNED at convening: [`Protocol::pin`] is the
//!   sha256 of the protocol's canonical bytes, [`Convening`] stores it at
//!   [`Convening::open`] time (never caller-supplied), and
//!   [`Convening::verify_pin`] recomputes and REJECTS on mismatch. The P3
//!   executor refuses to dispatch an unpinned/mismatched convening — the
//!   mid-flight-edit → pause → re-dispatch choreography (F11) is P2/P3 work
//!   layered ON this seam, never instead of it.
//! - **House law** (plan P2): changing a protocol = new card version +
//!   quorum sign-off. The substrate expresses this as DATA: `version` is a
//!   protocol field inside the canonical bytes, so ANY edit — even one that
//!   forgets the version bump — still flips the pin and fails
//!   [`Convening::verify_pin`]. The pin covers every behavioral field.
//! - **F9/floors tie-in**: the panel a convening seats must satisfy the
//!   floors of the very protocol it pins — [`Convening::open`] refuses a
//!   floor-violating panel (fail-closed; silent degradation is dead).
//!   Quorum-pool disjointness vs the council panel (F9 STRICT, overlap =
//!   error) is the slice-3 check, parameterized by nothing — it is a law,
//!   not a knob.
//! - **Provenance law** (brief lesson): model identity comes from TRANSPORT
//!   records, never self-report. [`ProvenanceRow::transport_served_model`]
//!   is named for where its value must come from; there is deliberately no
//!   self-report field or constructor.
//! - **Zero runtime deps**: [`Protocol::canonical`] is hand-rolled (router
//!   `encode_registry` / `esc` precedent); serde stays dev-only (P0
//!   round-trip proof).

use crate::sha256;
use crate::{Panel, PanelErr};

/// Which deliberation shape this protocol prescribes (P2 authors the two
/// v1 cards; the substrate carries the vocabulary as DATA).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(test, derive(serde::Serialize, serde::Deserialize))]
pub enum ProtocolKind {
    Council,
    Quorum,
}

impl ProtocolKind {
    /// Canonical token used in [`Protocol::canonical`].
    pub fn as_str(self) -> &'static str {
        match self {
            ProtocolKind::Council => "council",
            ProtocolKind::Quorum => "quorum",
        }
    }
}

/// A deliberation protocol — VERSIONED DATA (F3): kind, ordered stages,
/// and the panel floors it prescribes. Stages are the named pipeline steps
/// in execution order (the plan's P2 cards: convene → panel → dispatch →
/// collect → integrate → verdict → ledger); the substrate pins the
/// vocabulary, P2 attaches mechanics per stage.
///
/// Changing ANY field is a protocol change: new card version + quorum
/// sign-off (house law), and the pin flips either way.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(test, derive(serde::Serialize, serde::Deserialize))]
pub struct Protocol {
    pub version: u32,
    pub kind: ProtocolKind,
    pub stages: Vec<String>,
    pub floors: crate::Floors,
}

impl Protocol {
    /// Deterministic canonical byte form — hand-rolled (serde is dev-only),
    /// fixed key order, strings escaped, so equal protocols always encode
    /// to equal bytes and any behavioral difference always encodes
    /// differently. Injective by construction; never parsed back in P0.
    pub fn canonical(&self) -> String {
        let stages = self
            .stages
            .iter()
            .map(|s| format!("\"{}\"", esc(s)))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            concat!(
                "{{\"version\":{},\"kind\":\"{}\",\"stages\":[{}],",
                "\"floors\":{{\"panel_size\":{},\"min_families\":{},\"min_non_chinese\":{}}}}}"
            ),
            self.version,
            self.kind.as_str(),
            stages,
            self.floors.panel_size,
            self.floors.min_families,
            self.floors.min_non_chinese,
        )
    }

    /// The PIN (F3): sha256-hex of the canonical bytes. 64 lowercase hex
    /// chars. Stored by [`Convening::open`], checked by
    /// [`Convening::verify_pin`].
    pub fn pin(&self) -> String {
        sha256::hex(self.canonical().as_bytes())
    }
}

/// One dispatch-log row: what left, to which seat, at which stage, digest
/// of the exact payload. P3 fills it as it dispatches; P0 carries the
/// shape so a convening is auditable from birth.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(test, derive(serde::Serialize, serde::Deserialize))]
pub struct DispatchEntry {
    pub stage: String,
    pub lane_id: String,
    /// sha256-hex of the dispatched payload (integrity seam for P3).
    pub payload_digest: String,
}

/// One deliberation event under a pinned protocol.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(test, derive(serde::Serialize, serde::Deserialize))]
pub struct Convening {
    pub id: String,
    pub task: String,
    /// sha256 of the protocol AT CONVENING TIME (F3). Computed by
    /// [`Convening::open`] — never caller-supplied, never mutated.
    pub pinned_protocol: String,
    pub panel: Panel,
    pub dispatch_log: Vec<DispatchEntry>,
}

/// Convening construction refusals. Fail-closed: a floor-violating panel
/// or a size mismatch is a REFUSAL, never a degraded convening.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConveningErr {
    /// The seated panel violates the pinned protocol's floors.
    Floor(PanelErr),
    /// `panel.seats.len() != protocol.floors.panel_size`.
    PanelSizeMismatch { have: usize, want: usize },
}

impl std::fmt::Display for ConveningErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConveningErr::Floor(e) => write!(f, "panel violates pinned protocol floors: {e}"),
            ConveningErr::PanelSizeMismatch { have, want } => {
                write!(
                    f,
                    "panel size mismatch: have {have} seats, protocol wants {want}"
                )
            }
        }
    }
}

impl std::error::Error for ConveningErr {}

impl Convening {
    /// Open a convening: pin the protocol (F3) and refuse any panel that
    /// does not satisfy the protocol's own floors. The dispatch log starts
    /// empty; P3 appends as it dispatches.
    pub fn open(
        id: impl Into<String>,
        task: impl Into<String>,
        protocol: &Protocol,
        panel: Panel,
    ) -> Result<Convening, ConveningErr> {
        let want = protocol.floors.panel_size;
        let have = panel.seats.len();
        if have != want {
            return Err(ConveningErr::PanelSizeMismatch { have, want });
        }
        if let Err(e) = panel.check_floors(&protocol.floors) {
            return Err(ConveningErr::Floor(e));
        }
        Ok(Convening {
            id: id.into(),
            task: task.into(),
            pinned_protocol: protocol.pin(),
            panel,
            dispatch_log: Vec::new(),
        })
    }

    /// F3: recompute the protocol's pin and compare against the one stored
    /// at open time. `Ok(())` = this is still the exact protocol the
    /// convening was opened under; `Err` = the protocol moved (mid-flight
    /// edit, version bump, floor change — ANY difference) and dispatch
    /// under it must be refused. The F11 pause→re-dispatch choreography
    /// (P2/P3) starts FROM this error, never bypasses it.
    pub fn verify_pin(&self, protocol: &Protocol) -> Result<(), PinMismatch> {
        let actual = protocol.pin();
        if actual == self.pinned_protocol {
            Ok(())
        } else {
            Err(PinMismatch {
                pinned: self.pinned_protocol.clone(),
                actual,
            })
        }
    }
}

/// F3 refusal: the protocol under review no longer hashes to the pinned
/// value. Carries both hashes — the mismatch is the evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinMismatch {
    pub pinned: String,
    pub actual: String,
}

impl std::fmt::Display for PinMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "protocol pin mismatch: convened under {}, protocol now hashes to {} — refuse dispatch (F3)",
            self.pinned, self.actual
        )
    }
}

impl std::error::Error for PinMismatch {}

/// Per-seat provenance for a verdict: model identity from TRANSPORT
/// records only (brief lesson). The field is NAMED for its source; there
/// is deliberately no self-report path anywhere in this type.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(test, derive(serde::Serialize, serde::Deserialize))]
pub struct ProvenanceRow {
    pub lane_id: String,
    pub lane_type: crate::LaneType,
    /// The model the TRANSPORT says it served for this reply. Never a
    /// model the seat claims about itself.
    pub transport_served_model: String,
}

/// A structured ruling with provenance. P2 builds the verdict TABLE
/// (disagreement mapping, never averaging; quorum floor 2/3); P0 pins the
/// shape: which convening, what was ruled, who actually served it, and
/// whether the convening ran degraded (the quorum asterisk seam).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(test, derive(serde::Serialize, serde::Deserialize))]
pub struct Verdict {
    pub convening_id: String,
    pub ruling: String,
    pub provenance: Vec<ProvenanceRow>,
    /// Degradation marker — the quorum "asterisk under degradation"
    /// (plan P2). False unless a recorded degradation happened.
    pub degraded: bool,
}

/// JSON-style string escaping for [`Protocol::canonical`] — vendored by
/// organ law from caddis-router's ledger `esc` (zero runtime deps). A raw
/// `"`/`\`/control char inside a stage name must never change the
/// canonical framing, or two different protocols could share bytes.
pub(crate) fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod protocol_tests;
