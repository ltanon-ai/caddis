//! executor_tests.rs — P3 slice 1 executor tests. Plan P3 Done-When
//! slices proven here: sandbox convening end-to-end against stub
//! http/bridge/cli lanes; cap enforcement (Ruling 7) and
//! parallel-behind-raised-cap (F4) observed through a bounded rendezvous;
//! session card rows written; provenance records the transport-served
//! model; F3 pin-moved → F11 re-dispatch through the executor.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// std-only crate law (crates.io deps banned) + no-unwrapped-lock rule:
/// recover the guard from poisoning directly — the call site shows
/// locking, never error handling.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

use crate::council::{self, Position, Stakes, COUNCIL_STAGES};
use crate::executor::{run_council, ExecErr, Executed, Lane, LaneErr, LaneOutput, LaneSet};
use crate::protocol::{Protocol, ProtocolKind};
use crate::registry::{ProviderCard, Registry, SeatCard};
use crate::sessions::{self, SessionRow};
use crate::{CostClass, Floors, LaneType, Seat, SeatState};

// --- fixtures (council_tests precedent) ------------------------------------

const ACTOR: &str = "terminal.ashpac";

fn warden_row(typ: &str, from: &str, body_text: &str) -> String {
    format!(
        "{{\"seq\":1,\"v\":1,\"id\":\"x\",\"idem_key\":\"k\",\"type\":\"{typ}\",\
         \"from\":\"{from}\",\"to\":\"warden\",\"body\":\"{body_text}\",\"ts\":\"1\"}}\n"
    )
}

fn gate_open() -> String {
    warden_row(
        "card.open",
        ACTOR,
        &caddis_warden::card_state::body("open", "CARD-0007", "_card_x.md", "deadbeef"),
    )
}

fn seat(id: &str, family: &str, lt: LaneType, caps: u32) -> Seat {
    Seat {
        lane_id: id.to_string(),
        lane_type: lt,
        family: family.to_string(),
        provider: family.to_string(),
        model: format!("{family}-registered-model"),
        cost_class: CostClass::Free,
        state: SeatState::Live,
        caps,
        last_probe: Some(std::time::SystemTime::UNIX_EPOCH),
    }
}

fn card(version: u32, floors: Floors) -> Protocol {
    Protocol {
        version,
        kind: ProtocolKind::Council,
        stages: COUNCIL_STAGES.iter().map(|s| s.to_string()).collect(),
        floors,
    }
}

/// Registry fixture: providers with caps, seats with caps; seat ids must
/// prefix-match nothing in particular but the seat row's provider field
/// names its provider.
fn registry_with_caps(
    providers: &[(&str, u32, LaneType)],
    seats: &[(&str, &str, u32, LaneType)],
) -> Registry {
    Registry {
        providers: providers
            .iter()
            .map(|(id, caps, lt)| {
                (
                    id.to_string(),
                    ProviderCard {
                        id: id.to_string(),
                        lane_type: *lt,
                        base_url: String::new(),
                        auth_path: String::new(),
                        caps: *caps,
                        source: "test".to_string(),
                    },
                )
            })
            .collect(),
        seats: seats
            .iter()
            .map(|(id, provider, caps, lt)| {
                (
                    id.to_string(),
                    SeatCard {
                        id: id.to_string(),
                        provider: provider.to_string(),
                        family: provider.to_string(),
                        model: format!("{provider}-registered-model"),
                        lane_type: *lt,
                        cost_class: CostClass::Free,
                        state: SeatState::Live,
                        since_epoch_s: 0,
                        caps: *caps,
                        cost_in_usd_per_mtok: 0.0,
                        cost_out_usd_per_mtok: 0.0,
                        context_window: 0,
                        max_tokens: 0,
                        source: "test".to_string(),
                    },
                )
            })
            .collect(),
    }
}

// --- stub lanes --------------------------------------------------------------

/// Bounded rendezvous with IN-FLIGHT semantics: a lane marks itself
/// co-resident on entry, waits (max 250 ms) until `expected` lanes are
/// co-resident SIMULTANEOUSLY, and unmarks itself on exit. `met` is TRUE
/// only for real overlap — a later serialized wave can never "meet" the
/// ghost of an earlier one. No hang: a serialized lane times out alone.
#[derive(Default)]
struct RendezvousState {
    /// Lane ids currently co-resident inside `arrive`.
    inflight: Vec<String>,
    /// Per-lane meeting record (unique per lane per run).
    met: Vec<(String, bool)>,
}

#[derive(Default)]
struct Rendezvous {
    expected: usize,
    state: Mutex<RendezvousState>,
}

impl Rendezvous {
    fn new(expected: usize) -> Arc<Rendezvous> {
        Arc::new(Rendezvous {
            expected,
            state: Mutex::new(RendezvousState::default()),
        })
    }

    fn arrive(&self, id: &str) {
        lock(&self.state).inflight.push(id.to_string());
        let deadline = Instant::now() + Duration::from_millis(250);
        loop {
            let (met, done) = {
                let st = lock(&self.state);
                (
                    st.inflight.len() >= self.expected,
                    st.met.iter().any(|(l, _)| l == id),
                )
            };
            if met || done || Instant::now() >= deadline {
                let mut st = lock(&self.state);
                if st.met.iter().any(|(l, _)| l == id) {
                    return; // a peer already recorded our meeting
                }
                if met {
                    // Everyone co-resident RIGHT NOW met — that is the
                    // overlap — recorded atomically for all of them.
                    let ids: Vec<String> = st.inflight.clone();
                    for i in ids {
                        st.met.push((i, true));
                    }
                    st.inflight.clear();
                } else {
                    st.inflight.retain(|l| l != id);
                    st.met.push((id.to_string(), false));
                }
                return;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn met(&self, id: &str) -> bool {
        lock(&self.state)
            .met
            .iter()
            .find(|(l, _)| l == id)
            .map(|(_, m)| *m)
            .unwrap_or(false)
    }
}

/// Stub lane fixture: serves a model that DELIBERATELY differs from the
/// seat's registered model (provenance law proof), optional failure,
/// optional rendezvous, optional mid-run side effect (the F11 hook).
struct StubLane {
    id: String,
    lt: LaneType,
    served_model: String,
    position: Position,
    fail: bool,
    rendezvous: Option<Arc<Rendezvous>>,
    on_invoke: Option<Box<dyn Fn() + Send + Sync>>,
}

impl StubLane {
    fn stub(id: &str, lt: LaneType, position: Position) -> StubLane {
        StubLane {
            id: id.to_string(),
            lt,
            served_model: format!("{id}/transport-served"),
            position,
            fail: false,
            rendezvous: None,
            on_invoke: None,
        }
    }

    fn with_rendezvous(mut self, r: Arc<Rendezvous>) -> Self {
        self.rendezvous = Some(r);
        self
    }

    fn failing(mut self) -> Self {
        self.fail = true;
        self
    }

    fn on_invoke(mut self, f: Box<dyn Fn() + Send + Sync>) -> Self {
        self.on_invoke = Some(f);
        self
    }

    fn into_lane(self) -> Arc<dyn Lane> {
        Arc::new(self)
    }
}

impl Lane for StubLane {
    fn lane_id(&self) -> &str {
        &self.id
    }

    fn lane_type(&self) -> LaneType {
        self.lt
    }

    fn invoke(&self, _task: &str) -> Result<LaneOutput, LaneErr> {
        if let Some(f) = &self.on_invoke {
            f();
        }
        if let Some(r) = &self.rendezvous {
            r.arrive(&self.id);
        }
        if self.fail {
            return Err(LaneErr {
                lane_id: self.id.clone(),
                reason: "transport exploded (stub)".to_string(),
            });
        }
        Ok(LaneOutput {
            transport_served_model: self.served_model.clone(),
            position: self.position,
            tokens_in: 100,
            tokens_out: 200,
        })
    }
}

fn tmp_stream(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("caddis-exec-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("sessions.jsonl")
}

fn read_rows(path: &std::path::Path) -> Vec<SessionRow> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    sessions::parse_rows(&text).unwrap()
}

// --- the happy sandbox: 4 families, mixed lane types, mixed positions ---

fn happy_run(tag: &str) -> (Executed, std::path::PathBuf) {
    let candidates = vec![
        seat("groq/llama", "groq", LaneType::Http, 1),
        seat("anthropic/claude", "anthropic", LaneType::Bridge, 1),
        seat("zai/glm", "zai", LaneType::Cli, 1),
        seat("nvidia/nem", "nvidia", LaneType::Http, 1),
    ];
    let reg = registry_with_caps(
        &[
            ("groq", 1, LaneType::Http),
            ("anthropic", 1, LaneType::Bridge),
            ("zai", 1, LaneType::Cli),
            ("nvidia", 1, LaneType::Http),
        ],
        &[
            ("groq/llama", "groq", 1, LaneType::Http),
            ("anthropic/claude", "anthropic", 1, LaneType::Bridge),
            ("zai/glm", "zai", 1, LaneType::Cli),
            ("nvidia/nem", "nvidia", 1, LaneType::Http),
        ],
    );
    let lanes = LaneSet::new()
        .with(StubLane::stub("groq/llama", LaneType::Http, Position::Ship).into_lane())
        .with(
            StubLane::stub(
                "anthropic/claude",
                LaneType::Bridge,
                Position::ShipWithChanges,
            )
            .into_lane(),
        )
        .with(StubLane::stub("zai/glm", LaneType::Cli, Position::Ship).into_lane())
        .with(StubLane::stub("nvidia/nem", LaneType::Http, Position::DoNotShip).into_lane());
    let session = council::convene(
        "c1",
        "rule on the slice",
        Stakes::Medium,
        &card(1, Floors::default()),
        &candidates,
        ACTOR,
        &gate_open(),
    )
    .unwrap();
    let path = tmp_stream(tag);
    let _ = std::fs::remove_file(&path);
    let v1 = card(1, Floors::default());
    let executed = run_council(session, move || v1.clone(), &reg, &lanes, &path).unwrap();
    (executed, path)
}

#[test]
fn sandbox_council_runs_end_to_end_over_stub_lanes() {
    let (executed, path) = happy_run("e2e");

    // Verdict: full panel answered, never degraded.
    assert!(!executed.verdict.degraded);
    assert_eq!(executed.verdict.convening_id, "c1");
    assert_eq!(executed.bundled.replies.len(), 4);

    // Provenance: TRANSPORT-served models, never the registered ones.
    for p in &executed.verdict.provenance {
        assert!(p.transport_served_model.ends_with("/transport-served"));
        assert!(!p.transport_served_model.contains("registered"));
    }

    // Ledger row parses through the ONE parser; counts match the map.
    let row = council::parse_ledger_row(&executed.ledger_row).unwrap();
    assert_eq!(row.conv, "c1");
    assert_eq!(row.ship, 2);
    assert_eq!(row.ship_with_changes, 1);
    assert_eq!(row.do_not_ship, 1);

    // Dispatch log: one entry per answered leg (P0 seam filled).
    assert_eq!(executed.session.convening.dispatch_log.len(), 4);
    for e in &executed.session.convening.dispatch_log {
        assert_eq!(e.stage, "dispatch");
    }

    // Session cards: open + 4 usage + close, in order.
    let rows = read_rows(&path);
    assert_eq!(rows.len(), 6);
    assert!(matches!(rows[0], SessionRow::Open(_)));
    assert!(matches!(rows[4], SessionRow::Usage(_)));
    assert!(matches!(rows[5], SessionRow::Close(_)));
    assert_eq!(executed.session_rows.len(), 6);
}

#[test]
fn session_usage_rows_carry_transport_models_and_token_counts() {
    let (_, path) = happy_run("usage");
    let rows = read_rows(&path);
    let usages: Vec<_> = rows
        .iter()
        .filter_map(|r| match r {
            SessionRow::Usage(u) => Some(u),
            _ => None,
        })
        .collect();
    assert_eq!(usages.len(), 4);
    for u in &usages {
        assert!(u.model.ends_with("/transport-served"));
        assert_eq!(u.tokens_in, 100);
        assert_eq!(u.tokens_out, 200);
        assert_eq!(u.cost_class, CostClass::Free);
        assert_eq!(u.conv, "c1");
    }
    // The close row's digest links to the ledger row's own digest.
    let close = match rows.last().unwrap() {
        SessionRow::Close(c) => c,
        other => panic!("{other:?}"),
    };
    assert_eq!(close.verdict_digest.len(), 64);
    assert_eq!(close.ship, 2);
    assert_eq!(close.ship_with_changes, 1);
    assert_eq!(close.do_not_ship, 1);
}

// --- cap enforcement + parallel-behind-raised-cap (F4 / Ruling 7) ---------

fn two_seat_world(provider_caps: u32, seat_caps: u32) -> (Vec<Seat>, Registry) {
    let candidates = vec![
        seat("anthropic/a", "anthropic", LaneType::Http, seat_caps),
        seat("anthropic/b", "anthropic", LaneType::Http, seat_caps),
    ];
    let reg = registry_with_caps(
        &[("anthropic", provider_caps, LaneType::Http)],
        &[
            ("anthropic/a", "anthropic", seat_caps, LaneType::Http),
            ("anthropic/b", "anthropic", seat_caps, LaneType::Http),
        ],
    );
    (candidates, reg)
}

fn small_floors() -> Floors {
    Floors {
        panel_size: 2,
        min_families: 1,
        min_non_chinese: 1,
    }
}

fn convened_two_seat(candidates: &[Seat]) -> council::CouncilSession {
    council::convene(
        "w1",
        "wave law",
        Stakes::Small,
        &card(1, small_floors()),
        candidates,
        ACTOR,
        &gate_open(),
    )
    .unwrap()
}

#[test]
fn capped_provider_serializes_across_separate_waves() {
    // Ruling 7 default: provider caps 1, seat caps 1 → effective 1 → the
    // two seats NEVER share a wave and never co-reside in a lane.
    let (candidates, reg) = two_seat_world(1, 1);
    let session = convened_two_seat(&candidates);
    let waves = council::dispatch_plan(&session, &reg).unwrap();
    assert_eq!(
        waves,
        vec![
            vec!["anthropic/a".to_string()],
            vec!["anthropic/b".to_string()]
        ]
    );

    let r = Rendezvous::new(2);
    let lanes = LaneSet::new()
        .with(
            StubLane::stub("anthropic/a", LaneType::Http, Position::Ship)
                .with_rendezvous(r.clone())
                .into_lane(),
        )
        .with(
            StubLane::stub("anthropic/b", LaneType::Http, Position::Ship)
                .with_rendezvous(r.clone())
                .into_lane(),
        );
    let path = tmp_stream("cap1");
    let _ = std::fs::remove_file(&path);
    let v1 = card(1, small_floors());
    let executed = run_council(session, move || v1.clone(), &reg, &lanes, &path).unwrap();
    assert_eq!(executed.bundled.replies.len(), 2);
    assert!(!r.met("anthropic/a"), "serialized lanes never co-reside");
    assert!(!r.met("anthropic/b"), "serialized lanes never co-reside");
}

#[test]
fn raised_cap_lets_one_provider_share_a_wave() {
    // F4's parallel lane: the (warden-gated, in life) raised cap — here a
    // fixture — is the ONLY thing that puts two seats of one provider
    // into one concurrent wave.
    let (candidates, reg) = two_seat_world(2, 2);
    let session = convened_two_seat(&candidates);
    let waves = council::dispatch_plan(&session, &reg).unwrap();
    assert_eq!(
        waves,
        vec![vec!["anthropic/a".into(), "anthropic/b".to_string()]]
    );

    let r = Rendezvous::new(2);
    let lanes = LaneSet::new()
        .with(
            StubLane::stub("anthropic/a", LaneType::Http, Position::Ship)
                .with_rendezvous(r.clone())
                .into_lane(),
        )
        .with(
            StubLane::stub("anthropic/b", LaneType::Http, Position::Ship)
                .with_rendezvous(r.clone())
                .into_lane(),
        );
    let path = tmp_stream("cap2");
    let _ = std::fs::remove_file(&path);
    let v1 = card(1, small_floors());
    run_council(session, move || v1.clone(), &reg, &lanes, &path).unwrap();
    assert!(r.met("anthropic/a"), "same-wave lanes co-reside");
    assert!(r.met("anthropic/b"), "same-wave lanes co-reside");
}

#[test]
fn different_providers_share_one_wave_under_default_caps() {
    // Serialized-by-default is PER PROVIDER: two providers, each within
    // its own cap, legally share a wave and run concurrently.
    let candidates = vec![
        seat("groq/llama", "groq", LaneType::Http, 1),
        seat("anthropic/claude", "anthropic", LaneType::Http, 1),
    ];
    let reg = registry_with_caps(
        &[
            ("groq", 1, LaneType::Http),
            ("anthropic", 1, LaneType::Http),
        ],
        &[
            ("groq/llama", "groq", 1, LaneType::Http),
            ("anthropic/claude", "anthropic", 1, LaneType::Http),
        ],
    );
    let floors = Floors {
        panel_size: 2,
        min_families: 2,
        min_non_chinese: 1,
    };
    let session = council::convene(
        "w2",
        "cross-provider waves",
        Stakes::Small,
        &card(1, floors),
        &candidates,
        ACTOR,
        &gate_open(),
    )
    .unwrap();
    let waves = council::dispatch_plan(&session, &reg).unwrap();
    // Panel order = selection_key (free-first, lane_id ties):
    // anthropic/claude seats before groq/llama — the law, not fixture
    // listing order.
    assert_eq!(
        waves,
        vec![vec![
            "anthropic/claude".to_string(),
            "groq/llama".to_string()
        ]]
    );

    let r = Rendezvous::new(2);
    let lanes = LaneSet::new()
        .with(
            StubLane::stub("groq/llama", LaneType::Http, Position::Ship)
                .with_rendezvous(r.clone())
                .into_lane(),
        )
        .with(
            StubLane::stub("anthropic/claude", LaneType::Http, Position::Ship)
                .with_rendezvous(r.clone())
                .into_lane(),
        );
    let path = tmp_stream("cross");
    let _ = std::fs::remove_file(&path);
    run_council(session, || card(1, floors), &reg, &lanes, &path).unwrap();
    assert!(r.met("groq/llama"));
    assert!(r.met("anthropic/claude"));
}

// --- F3 / F11 through the executor ------------------------------------------

#[test]
fn pin_moved_before_any_dispatch_writes_nothing_and_returns_the_session() {
    let (candidates, reg) = two_seat_world(1, 1);
    let session = convened_two_seat(&candidates);
    let lanes = LaneSet::new()
        .with(StubLane::stub("anthropic/a", LaneType::Http, Position::Ship).into_lane())
        .with(StubLane::stub("anthropic/b", LaneType::Http, Position::Ship).into_lane());
    let path = tmp_stream("pin0");
    let _ = std::fs::remove_file(&path);
    let v2 = card(2, small_floors());
    let err = run_council(session, move || v2.clone(), &reg, &lanes, &path).unwrap_err();
    assert!(err.is_refusal());
    match err {
        ExecErr::PinMoved { session, mismatch } => {
            assert_eq!(session.convening.id, "w1");
            assert_ne!(mismatch.pinned, mismatch.actual);
        }
        other => panic!("{other:?}"),
    }
    // F3 refused before the open row: no stream was ever created.
    assert!(!path.exists());
}

#[test]
fn mid_flight_edit_pauses_then_re_dispatches_end_to_end() {
    // Two waves (capped provider). Wave 1's lane flips the shared card to
    // v2 mid-run; the executor's per-wave re-read catches the moved pin
    // before wave 2; F11 re-dispatches and the re-run completes.
    let (candidates, reg) = two_seat_world(1, 1);
    let session = convened_two_seat(&candidates);

    let shared = Arc::new(Mutex::new(card(1, small_floors())));
    let flip = {
        let shared = shared.clone();
        Box::new(move || {
            let mut c = lock(&shared);
            *c = card(2, small_floors());
        })
    };
    let lanes = LaneSet::new()
        .with(
            StubLane::stub("anthropic/a", LaneType::Http, Position::Ship)
                .on_invoke(flip)
                .into_lane(),
        )
        .with(StubLane::stub("anthropic/b", LaneType::Http, Position::Ship).into_lane());

    let path = tmp_stream("f11");
    let _ = std::fs::remove_file(&path);
    let reader = shared.clone();
    let err = run_council(session, move || lock(&reader).clone(), &reg, &lanes, &path).unwrap_err();

    let (archived, mismatch) = match err {
        ExecErr::PinMoved { session, mismatch } => (*session, mismatch),
        other => panic!("expected PinMoved, got {other:?}"),
    };
    // Crash-honest partial: open + wave-1 usage, no close.
    let rows = read_rows(&path);
    assert_eq!(rows.len(), 2);
    assert!(matches!(rows[0], SessionRow::Open(_)));
    assert!(matches!(rows[1], SessionRow::Usage(_)));
    assert_ne!(mismatch.pinned, mismatch.actual);

    // F11: archive the original under v1, re-dispatch under v2.
    let v1 = card(1, small_floors());
    let v2 = card(2, small_floors());
    let paused =
        council::pause_and_re_dispatch(archived, &v1, &v2, &candidates, ACTOR, &gate_open())
            .unwrap();
    assert_eq!(paused.re_dispatched.convening.id, "w1#r2");
    assert_eq!(paused.re_dispatched.rerun_of.as_deref(), Some("w1"));

    let fresh = LaneSet::new()
        .with(StubLane::stub("anthropic/a", LaneType::Http, Position::Ship).into_lane())
        .with(StubLane::stub("anthropic/b", LaneType::Http, Position::Ship).into_lane());
    let path2 = tmp_stream("f11b");
    let _ = std::fs::remove_file(&path2);
    let executed = run_council(
        paused.re_dispatched,
        move || v2.clone(),
        &reg,
        &fresh,
        &path2,
    )
    .unwrap();
    assert_eq!(executed.bundled.replies.len(), 2);
    let row = council::parse_ledger_row(&executed.ledger_row).unwrap();
    assert_eq!(row.conv, "w1#r2");
    assert_eq!(row.rerun_of, "w1");

    // The re-run's open row carries rerun_of (auditable lineage).
    let rows2 = read_rows(&path2);
    match &rows2[0] {
        SessionRow::Open(o) => assert_eq!(o.rerun_of, "w1"),
        other => panic!("{other:?}"),
    }
}

// --- defects & refusals ------------------------------------------------------

#[test]
fn lane_type_crossing_is_a_defect() {
    let (candidates, reg) = two_seat_world(1, 1);
    let session = convened_two_seat(&candidates);
    // Panel seats BOTH as http; one lane lies about being a bridge.
    let lanes = LaneSet::new()
        .with(StubLane::stub("anthropic/a", LaneType::Bridge, Position::Ship).into_lane())
        .with(StubLane::stub("anthropic/b", LaneType::Http, Position::Ship).into_lane());
    let path = tmp_stream("cross-lane");
    let _ = std::fs::remove_file(&path);
    let v1 = card(1, small_floors());
    let err = run_council(session, move || v1.clone(), &reg, &lanes, &path).unwrap_err();
    assert!(!err.is_refusal());
    match err {
        ExecErr::Defect(m) => assert!(m.contains("identity crossing")),
        other => panic!("{other:?}"),
    }
    assert!(!path.exists());
}

#[test]
fn missing_lane_is_a_defect() {
    let (candidates, reg) = two_seat_world(1, 1);
    let session = convened_two_seat(&candidates);
    let lanes = LaneSet::new()
        .with(StubLane::stub("anthropic/a", LaneType::Http, Position::Ship).into_lane());
    let path = tmp_stream("missing");
    let _ = std::fs::remove_file(&path);
    let v1 = card(1, small_floors());
    let err = run_council(session, move || v1.clone(), &reg, &lanes, &path).unwrap_err();
    match err {
        ExecErr::Defect(m) => assert!(m.contains("no such lane")),
        other => panic!("{other:?}"),
    }
}

#[test]
fn lane_transport_failure_refuses_with_partial_session_rows() {
    // Fail the SECOND wave's lane: the run leaves open + wave-1 usage and
    // NO close — an auditable hole, never a silent one.
    let (candidates, reg) = two_seat_world(1, 1);
    let session = convened_two_seat(&candidates);
    let lanes = LaneSet::new()
        .with(StubLane::stub("anthropic/a", LaneType::Http, Position::Ship).into_lane())
        .with(
            StubLane::stub("anthropic/b", LaneType::Http, Position::Ship)
                .failing()
                .into_lane(),
        );
    let path = tmp_stream("fail");
    let _ = std::fs::remove_file(&path);
    let v1 = card(1, small_floors());
    let err = run_council(session, move || v1.clone(), &reg, &lanes, &path).unwrap_err();
    match &err {
        ExecErr::LaneRefused { lane_id, reason } => {
            assert_eq!(lane_id, "anthropic/b");
            assert!(reason.contains("stub"));
        }
        other => panic!("{other:?}"),
    }
    assert!(err.is_refusal());
    let rows = read_rows(&path);
    assert_eq!(rows.len(), 2); // open + the answered wave-1 usage
    assert!(!rows.iter().any(|r| matches!(r, SessionRow::Close(_))));
}

#[test]
fn blank_transport_model_is_refused_by_the_collect_law() {
    // Provenance has no blank form: a lane whose transport record carries
    // an empty model never reaches a verdict.
    let (candidates, reg) = two_seat_world(2, 2); // one wave, both answer
    let session = convened_two_seat(&candidates);
    let mut blank = StubLane::stub("anthropic/a", LaneType::Http, Position::Ship);
    blank.served_model = String::new();
    let lanes = LaneSet::new()
        .with(blank.into_lane())
        .with(StubLane::stub("anthropic/b", LaneType::Http, Position::Ship).into_lane());
    let path = tmp_stream("blank");
    let _ = std::fs::remove_file(&path);
    let v1 = card(1, small_floors());
    let err = run_council(session, move || v1.clone(), &reg, &lanes, &path).unwrap_err();
    match err {
        ExecErr::Council(council::CouncilErr::Defect(m)) => {
            assert!(m.contains("blank form"))
        }
        other => panic!("{other:?}"),
    }
    // Open + both usage rows landed (the lanes answered); no close.
    let rows = read_rows(&path);
    assert_eq!(rows.len(), 3);
    assert!(!rows.iter().any(|r| matches!(r, SessionRow::Close(_))));
}

// --- P3 slice 2: QUORUM execution through the executor ----------------------

use crate::executor::run_quorum;
use crate::quorum::{self, QuorumErr};

/// A lane whose invoke PANICS — wiring defect class, distinct from a
/// transport refusal (missing seat in a quorum run).
struct PanicLane {
    id: String,
    lt: LaneType,
}

impl Lane for PanicLane {
    fn lane_id(&self) -> &str {
        &self.id
    }
    fn lane_type(&self) -> LaneType {
        self.lt
    }
    fn invoke(&self, _task: &str) -> Result<LaneOutput, LaneErr> {
        panic!("stub lane wiring is broken");
    }
}

/// The council the quorum deliberates after: 4 seated seats (floors
/// 4/2/1 via `Floors::default()`), all families distinct.
fn council_for_quorum() -> council::CouncilSession {
    let candidates = vec![
        seat("groq/llama", "groq", LaneType::Http, 1),
        seat("anthropic/claude", "anthropic", LaneType::Bridge, 1),
        seat("zai/glm", "zai", LaneType::Cli, 1),
        seat("nvidia/nem", "nvidia", LaneType::Http, 1),
    ];
    council::convene(
        "c1",
        "rule on the slice",
        Stakes::Complex,
        &card(1, Floors::default()),
        &candidates,
        ACTOR,
        &gate_open(),
    )
    .unwrap()
}

/// Council candidates PLUS three disjoint free live seats — the quorum
/// pool is the three disjoint ones (free-first, lane_id ties):
/// cohere/c < gemini/g < mistral/m.
fn quorum_candidates() -> Vec<crate::Seat> {
    let mut c = vec![
        seat("groq/llama", "groq", LaneType::Http, 1),
        seat("anthropic/claude", "anthropic", LaneType::Bridge, 1),
        seat("zai/glm", "zai", LaneType::Cli, 1),
        seat("nvidia/nem", "nvidia", LaneType::Http, 1),
    ];
    c.push(seat("mistral/m", "mistral", LaneType::Http, 1));
    c.push(seat("gemini/g", "gemini", LaneType::Http, 1));
    c.push(seat("cohere/c", "cohere", LaneType::Http, 1));
    c
}

fn quorum_registry() -> Registry {
    registry_with_caps(
        &[
            ("mistral", 1, LaneType::Http),
            ("gemini", 1, LaneType::Http),
            ("cohere", 1, LaneType::Http),
        ],
        &[
            ("mistral/m", "mistral", 1, LaneType::Http),
            ("gemini/g", "gemini", 1, LaneType::Http),
            ("cohere/c", "cohere", 1, LaneType::Http),
        ],
    )
}

fn convened_quorum() -> quorum::QuorumSession {
    quorum::convene(
        "q1",
        &council_for_quorum(),
        &quorum::protocol_v1(),
        &quorum_candidates(),
        ACTOR,
        &gate_open(),
    )
    .unwrap()
}

/// Registry that SERIALIZES the pool into three waves: cohere/c and
/// gemini/g share the capped `cohere` provider (caps 1) — the mid-run
/// pin check must fire between waves.
fn quorum_registry_serial() -> Registry {
    registry_with_caps(
        &[
            ("cohere", 1, LaneType::Http),
            ("gemini", 1, LaneType::Http),
            ("mistral", 1, LaneType::Http),
        ],
        &[
            ("cohere/c", "cohere", 1, LaneType::Http),
            ("gemini/g", "cohere", 1, LaneType::Http),
            ("mistral/m", "mistral", 1, LaneType::Http),
        ],
    )
}

fn pool_lane_ids(session: &quorum::QuorumSession) -> Vec<String> {
    session
        .pool
        .seats
        .iter()
        .map(|s| s.lane_id.clone())
        .collect()
}

#[test]
fn quorum_runs_end_to_end_over_stub_lanes() {
    let session = convened_quorum();
    assert_eq!(
        pool_lane_ids(&session),
        vec!["cohere/c", "gemini/g", "mistral/m"]
    );
    let lanes = LaneSet::new()
        .with(StubLane::stub("cohere/c", LaneType::Http, Position::Ship).into_lane())
        .with(StubLane::stub("gemini/g", LaneType::Http, Position::Ship).into_lane())
        .with(StubLane::stub("mistral/m", LaneType::Http, Position::DoNotShip).into_lane());
    let path = tmp_stream("qe2e");
    let _ = std::fs::remove_file(&path);
    let v1 = quorum::protocol_v1();
    let executed = run_quorum(
        session,
        move || v1.clone(),
        &quorum_registry(),
        &lanes,
        &path,
    )
    .unwrap();

    // Ruling: ship at 2/3 (strict majority of the FULL pool), never degraded.
    assert!(!executed.verdict.degraded);
    assert_eq!(executed.verdict.ruling, "ship");
    assert_eq!(executed.ruling.position, Position::Ship);
    assert_eq!(executed.ruling.agreeing.len(), 2);
    assert_eq!(executed.ruling.pool_size, 3);
    assert!(executed.votes.missing.is_empty());

    // Provenance: TRANSPORT-served models, never the registered ones.
    for p in &executed.verdict.provenance {
        assert!(p.transport_served_model.ends_with("/transport-served"));
        assert!(!p.transport_served_model.contains("registered"));
    }

    // Dispatch log: one entry per answered leg (the council law, pool body).
    assert_eq!(executed.session.dispatch_log.len(), 3);
    for e in &executed.session.dispatch_log {
        assert_eq!(e.stage, "dispatch");
    }

    // Session cards: open(kind=quorum) + 3 usage + close.
    let rows = read_rows(&path);
    assert_eq!(rows.len(), 5);
    match &rows[0] {
        SessionRow::Open(o) => {
            assert_eq!(o.kind, "quorum");
            assert_eq!(o.conv, "q1");
            assert_eq!(o.rerun_of, "");
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(rows[3], SessionRow::Usage(_)));
    assert!(matches!(rows[4], SessionRow::Close(_)));
    let row = quorum::parse_ledger_row(&executed.ledger_row).unwrap();
    assert_eq!(row.conv, "q1");
    assert_eq!(row.ruled, "ship");
    assert_eq!(row.missing, 0);
    assert_eq!(row.floor, "2/3");
    let close_digest = match &rows[4] {
        SessionRow::Close(c) => c.verdict_digest.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(close_digest, row.verdict_digest);

    // VERDICT.md artifact: ruling line, full pool answered.
    assert!(executed.verdict_md.contains("verdict: ship\n"));
    assert!(executed.verdict_md.contains("missing: none"));
    assert!(executed.verdict_md.contains("council: c1"));
}

#[test]
fn quorum_degraded_run_tolerates_one_missing_seat() {
    let lanes = LaneSet::new()
        .with(StubLane::stub("cohere/c", LaneType::Http, Position::Ship).into_lane())
        .with(StubLane::stub("gemini/g", LaneType::Http, Position::Ship).into_lane())
        .with(
            StubLane::stub("mistral/m", LaneType::Http, Position::Ship)
                .failing()
                .into_lane(),
        );
    let path = tmp_stream("qdegraded");
    let _ = std::fs::remove_file(&path);
    let v1 = quorum::protocol_v1();
    let executed = run_quorum(
        convened_quorum(),
        move || v1.clone(),
        &quorum_registry(),
        &lanes,
        &path,
    )
    .unwrap();

    // Degraded but DECIDED: the asterisk rides the ruling, the missing
    // lane is named in every artifact, and the floor still holds (2/3).
    assert!(executed.verdict.degraded);
    assert_eq!(executed.verdict.ruling, "ship*");
    assert_eq!(executed.votes.missing, vec!["mistral/m".to_string()]);
    assert!(executed.verdict_md.contains("verdict: ship*\n"));
    assert!(executed.verdict_md.contains("missing: mistral/m"));

    // Usage rows: ANSWERED seats only (2), close still lands.
    let rows = read_rows(&path);
    assert_eq!(rows.len(), 4);
    assert!(
        rows.iter()
            .filter(|r| matches!(r, SessionRow::Usage(_)))
            .count()
            == 2
    );
    assert!(matches!(rows[3], SessionRow::Close(_)));

    let row = quorum::parse_ledger_row(&executed.ledger_row).unwrap();
    assert_eq!(row.missing, 1);
    assert!(row.degraded);
}

#[test]
fn quorum_below_floor_refuses_fail_closed() {
    let lanes = LaneSet::new()
        .with(
            StubLane::stub("cohere/c", LaneType::Http, Position::Ship)
                .failing()
                .into_lane(),
        )
        .with(
            StubLane::stub("gemini/g", LaneType::Http, Position::Ship)
                .failing()
                .into_lane(),
        )
        .with(StubLane::stub("mistral/m", LaneType::Http, Position::Ship).into_lane());
    let path = tmp_stream("qfloor");
    let _ = std::fs::remove_file(&path);
    let v1 = quorum::protocol_v1();
    let err = run_quorum(
        convened_quorum(),
        move || v1.clone(),
        &quorum_registry(),
        &lanes,
        &path,
    )
    .unwrap_err();
    match err {
        ExecErr::Quorum(QuorumErr::CollectIncomplete {
            ref missing,
            have,
            floor,
        }) => {
            assert_eq!(
                missing,
                &vec!["cohere/c".to_string(), "gemini/g".to_string()]
            );
            assert_eq!(have, 1);
            assert_eq!(floor, 2);
        }
        other => panic!("{other:?}"),
    }
    assert!(err.is_refusal());

    // Crash-honest partial: open + the one answered usage, NO close.
    let rows = read_rows(&path);
    assert_eq!(rows.len(), 2);
    assert!(!rows.iter().any(|r| matches!(r, SessionRow::Close(_))));
}

#[test]
fn quorum_panicked_lane_thread_is_a_defect_not_a_missing_seat() {
    let lanes = LaneSet::new()
        .with(StubLane::stub("cohere/c", LaneType::Http, Position::Ship).into_lane())
        .with(StubLane::stub("gemini/g", LaneType::Http, Position::Ship).into_lane())
        .with(Arc::new(PanicLane {
            id: "mistral/m".to_string(),
            lt: LaneType::Http,
        }));
    let path = tmp_stream("qpanic");
    let _ = std::fs::remove_file(&path);
    let v1 = quorum::protocol_v1();
    let err = run_quorum(
        convened_quorum(),
        move || v1.clone(),
        &quorum_registry(),
        &lanes,
        &path,
    )
    .unwrap_err();
    match &err {
        ExecErr::Defect(m) => assert!(m.contains("panicked")),
        other => panic!("{other:?}"),
    }
    assert!(!err.is_refusal());
}

#[test]
fn council_panicked_lane_thread_is_a_defect() {
    // Pins the slice-1 doc law the code now enforces: a panicked lane
    // thread is a Defect (wiring), never a LaneRefused.
    let candidates = vec![
        seat("groq/a", "groqa", LaneType::Http, 1),
        seat("zai/b", "zaib", LaneType::Http, 1),
    ];
    let reg = registry_with_caps(
        &[("groqa", 1, LaneType::Http), ("zaib", 1, LaneType::Http)],
        &[
            ("groq/a", "groqa", 1, LaneType::Http),
            ("zai/b", "zaib", 1, LaneType::Http),
        ],
    );
    let session = council::convene(
        "c1",
        "task",
        Stakes::Small,
        &card(1, small_floors()),
        &candidates,
        ACTOR,
        &gate_open(),
    )
    .unwrap();
    let lanes = LaneSet::new()
        .with(StubLane::stub("groq/a", LaneType::Http, Position::Ship).into_lane())
        .with(Arc::new(PanicLane {
            id: "zai/b".to_string(),
            lt: LaneType::Http,
        }));
    let path = tmp_stream("cpanic");
    let _ = std::fs::remove_file(&path);
    let v1 = card(1, small_floors());
    let err = run_council(session, move || v1.clone(), &reg, &lanes, &path).unwrap_err();
    match &err {
        ExecErr::Defect(m) => assert!(m.contains("panicked")),
        other => panic!("{other:?}"),
    }
    assert!(!err.is_refusal());
}

#[test]
fn quorum_pin_move_returns_session_then_f11_redispatch_completes() {
    let lanes = LaneSet::new()
        .with(StubLane::stub("cohere/c", LaneType::Http, Position::Ship).into_lane())
        .with(StubLane::stub("gemini/g", LaneType::Http, Position::Ship).into_lane())
        .with(StubLane::stub("mistral/m", LaneType::Http, Position::Ship).into_lane());
    let path = tmp_stream("qf11");
    let _ = std::fs::remove_file(&path);

    // v1 for the first pin check, v2 afterwards: the card moves mid-run
    // (caps 1 → three serialized waves → the second check sees v2).
    let v2 = {
        let mut p = quorum::protocol_v1();
        p.version = 2;
        p
    };
    let seen = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = seen.clone();
    let protocol_reader = move || {
        let n = counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if n == 0 {
            quorum::protocol_v1()
        } else {
            v2.clone()
        }
    };
    let err = run_quorum(
        convened_quorum(),
        protocol_reader,
        &quorum_registry_serial(),
        &lanes,
        &path,
    )
    .unwrap_err();
    let (moved_session, _mismatch) = match err {
        ExecErr::PinMovedQuorum { session, mismatch } => (*session, mismatch),
        other => panic!("{other:?}"),
    };
    // The paused run leaves an auditable hole: open + partial usage, no close.
    let rows = read_rows(&path);
    assert!(matches!(rows[0], SessionRow::Open(_)));
    assert!(!rows.iter().any(|r| matches!(r, SessionRow::Close(_))));

    // F11: pause → bump → re-dispatch under v2, then the run completes.
    let council = council_for_quorum();
    let paused = quorum::pause_and_re_dispatch(
        moved_session,
        &quorum::protocol_v1(),
        &{
            let mut p = quorum::protocol_v1();
            p.version = 2;
            p
        },
        &council,
        &quorum_candidates(),
        ACTOR,
        &gate_open(),
    )
    .unwrap();
    assert_eq!(paused.archived.id, "q1");
    assert_eq!(paused.re_dispatched.id, "q1#r2");
    let v2_static = {
        let mut p = quorum::protocol_v1();
        p.version = 2;
        p
    };
    let executed = run_quorum(
        paused.re_dispatched,
        move || v2_static.clone(),
        &quorum_registry(),
        &lanes,
        &path,
    )
    .unwrap();
    let row = quorum::parse_ledger_row(&executed.ledger_row).unwrap();
    assert_eq!(row.conv, "q1#r2");
    assert_eq!(row.rerun_of, "q1");
}

#[test]
fn quorum_pool_identity_crossing_is_a_defect() {
    let lanes = LaneSet::new()
        .with(StubLane::stub("cohere/c", LaneType::Http, Position::Ship).into_lane())
        .with(StubLane::stub("gemini/g", LaneType::Http, Position::Ship).into_lane())
        // Seated as Http in the pool, declared Bridge in the LaneSet.
        .with(StubLane::stub("mistral/m", LaneType::Bridge, Position::Ship).into_lane());
    let path = tmp_stream("qcross");
    let _ = std::fs::remove_file(&path);
    let v1 = quorum::protocol_v1();
    let err = run_quorum(
        convened_quorum(),
        move || v1.clone(),
        &quorum_registry(),
        &lanes,
        &path,
    )
    .unwrap_err();
    match &err {
        ExecErr::Defect(m) => assert!(m.contains("identity crossing")),
        other => panic!("{other:?}"),
    }
}

#[test]
fn quorum_pool_lane_missing_from_laneset_is_a_defect() {
    let lanes = LaneSet::new()
        .with(StubLane::stub("cohere/c", LaneType::Http, Position::Ship).into_lane())
        .with(StubLane::stub("gemini/g", LaneType::Http, Position::Ship).into_lane());
    let path = tmp_stream("qnolane");
    let _ = std::fs::remove_file(&path);
    let v1 = quorum::protocol_v1();
    let err = run_quorum(
        convened_quorum(),
        move || v1.clone(),
        &quorum_registry(),
        &lanes,
        &path,
    )
    .unwrap_err();
    match &err {
        ExecErr::Defect(m) => assert!(m.contains("no such lane")),
        other => panic!("{other:?}"),
    }
}
