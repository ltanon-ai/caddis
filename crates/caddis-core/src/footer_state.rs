//! footer-state.rs — FOOTER-STATE v0 (CARD-0007; C2: duomenys gimsta R1a, TUI R3).
//! Tipizuota momentinė "upės būsenos" išraiška, kurią kada nors pieš Riverbed'as (29).
//! v0 laukai: organų būsenos iš ledger+persistence šaltinių, kuriuos core jau turi.
use crate::ledger::Ledger;
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct OrganState {
    pub node_id: String,
    pub state: String, // born | active | dormant | degraded | retired
}

/// CARD-P1d: the live session fields join the v0 ones — turns, folds, cold
/// bodies and the assembled prompt size the status bar draws.
///
/// This stays a DUMB DATA STRUCT and `caddis-core` stays the TCB: the crate's
/// manifest states zero runtime dependencies and it sits at the BOTTOM of the
/// workspace graph, so it cannot depend on `caddis-surface` to learn what an
/// event is. The DERIVATION — the fold from events to this state — lives in
/// `caddis-cli::footer`, which already depends on both crates. Moving the fold
/// down here would invert the graph to buy nothing.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FooterState {
    pub warden: String,
    pub organs: Vec<OrganState>,
    pub ledger_seq: u64,
    pub cards_done: u64,
    /// User turns taken in this session.
    pub turns: u64,
    /// Compactions performed (ledger baseline + this session's folds).
    pub folds: u64,
    /// Turn bodies paged out to the cold store.
    pub cold: u64,
    /// Characters in the last assembled prompt — the true context %.
    pub used_chars: u64,
    /// CARD-LASTCALL-1: unix seconds of the newest `model_call.completed`.
    ///
    /// The TIMESTAMP is stored and the AGE is computed at render, because an
    /// age is wrong the moment it is written down: a footer that redraws four
    /// times a second would need someone to remember to refresh a stored one.
    /// `None` = no model call has completed yet, which every surface must show
    /// as unknown and never as zero.
    pub last_completed_unix: Option<u64>,
    /// CARD-LASTCALL-1: the in-flight attempt number; `0` = no call in flight.
    ///
    /// `1` is what a perfectly healthy call reports, so only `> 1` means the
    /// state worth alarming about: alive, working, and starved of answers.
    pub attempt: u64,
}

impl FooterState {
    /// Surenka būseną iš turimų šaltinių (ledger + paprastas organų sąrašas).
    /// v0: organų sąrašas iš žinomų crate vardų (statinis); ledger_seq tikras.
    pub fn snapshot(ledger_path: &Path) -> FooterState {
        let seq = Ledger::open(ledger_path).map(|l| l.seq()).unwrap_or(0);
        FooterState {
            warden: "active".into(),
            organs: vec![
                OrganState {
                    node_id: "caddis-core".into(),
                    state: "active".into(),
                },
                OrganState {
                    node_id: "caddis-store".into(),
                    state: "active".into(),
                },
                OrganState {
                    node_id: "caddis-card".into(),
                    state: "active".into(),
                },
                OrganState {
                    node_id: "caddis-bee".into(),
                    state: "active".into(),
                },
                OrganState {
                    node_id: "caddis-cli".into(),
                    state: "active".into(),
                },
            ],
            ledger_seq: seq,
            cards_done: 0,
            // CARD-P1d: the live session fields are FOLDED FROM EVENTS by
            // caddis-cli::footer, never read here — a disk snapshot has no
            // session to describe.
            ..Default::default()
        }
    }

    /// Vienos eilutės plain-words atvaizdavimas (PLAIN-WORDS įstatymas):
    pub fn render_plain(&self) -> String {
        let alive = self.organs.iter().filter(|o| o.state == "active").count();
        format!(
            "warden {} | organs {}/5 active | ledger seq {}",
            self.warden, alive, self.ledger_seq
        )
    }
}
