//! edits_tests.rs — P1 slice 3 gates for the warden-gated propose→confirm
//! edit path.

use super::*;
use crate::registry::render_seed;
use std::fs;
use std::path::PathBuf;

// --- fixtures ---------------------------------------------------------------

fn provider(id: &str) -> Card {
    Card::Provider(ProviderCard {
        id: id.into(),
        lane_type: crate::LaneType::Http,
        base_url: format!("https://{id}.example/v1"),
        auth_path: String::new(),
        caps: 1,
        source: "models.json#deadbeef".into(),
    })
}

fn seat(id: &str, provider: &str) -> Card {
    Card::Seat(SeatCard {
        id: id.into(),
        provider: provider.into(),
        family: provider.into(),
        model: id.rsplit('/').next().unwrap().into(),
        lane_type: crate::LaneType::Http,
        cost_class: crate::CostClass::Free,
        state: crate::SeatState::Probing,
        since_epoch_s: 0,
        caps: 1,
        cost_in_usd_per_mtok: 0.0,
        cost_out_usd_per_mtok: 0.0,
        context_window: 128_000,
        max_tokens: 16_384,
        source: "models.json#deadbeef".into(),
    })
}

/// Fresh sandbox with a seeded two-card stream + synced view.
fn sandbox(name: &str) -> (PathBuf, PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("caddis-edits-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let stream = dir.join("seats.jsonl");
    let view = dir.join("seats-view.json");
    let journal = dir.join("edits.jsonl");
    fs::write(
        &stream,
        render_seed(&[provider("groq"), seat("groq/llama-4", "groq")]),
    )
    .unwrap();
    registry::load_and_sync(&stream, &view).unwrap();
    (stream, view, journal)
}

fn stream_lines(p: &Path) -> usize {
    fs::read_to_string(p)
        .unwrap()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .count()
}

// Warden ledger fixtures — rows built through the warden's OWN body law,
// never a hand-copied format.
fn warden_row(typ: &str, from: &str, body_text: &str) -> String {
    format!(
        "{{\"seq\":1,\"v\":1,\"id\":\"x\",\"idem_key\":\"k\",\"type\":\"{typ}\",\
         \"from\":\"{from}\",\"to\":\"warden\",\"body\":\"{body_text}\",\"ts\":\"1\"}}\n"
    )
}

fn warden_open(from: &str, card_id: &str) -> String {
    warden_row(
        "card.open",
        from,
        &card_state::body("open", card_id, "_card_x.md", "deadbeef"),
    )
}

fn warden_close(from: &str, card_id: &str) -> String {
    warden_row(
        "card.close",
        from,
        &card_state::body("close", card_id, "_card_x.md", "deadbeef"),
    )
}

const OPERATOR: &str = "terminal.ashpac";

fn gate_open() -> String {
    warden_open(OPERATOR, "CARD-0001")
}

// --- propose ----------------------------------------------------------------

#[test]
fn propose_writes_durable_pending_row_and_never_touches_the_stream() {
    let (stream, _view, journal) = sandbox("propose-pending");
    let before = fs::read_to_string(&stream).unwrap();
    let op = EditOp::UpsertProvider(match provider("nemotron") {
        Card::Provider(p) => p,
        _ => unreachable!(),
    });
    let id = propose(&stream, &journal, op.clone(), OPERATOR, "terminal").unwrap();
    assert_eq!(id, "e1");
    // Stream UNCHANGED — proposing is read-only against the truth.
    assert_eq!(fs::read_to_string(&stream).unwrap(), before);
    // One durable pending row; fold state Pending with the pinned prior.
    let j = journal_load(&journal);
    assert_eq!(j.rows.len(), 1);
    assert_eq!(j.unparseable, Vec::<usize>::new());
    let folded = fold_journal(&j).unwrap();
    let p = folded.get("e1").expect("pending proposal");
    assert_eq!(p.state, ProposalState::Pending);
    assert_eq!(p.op, op);
    assert_eq!(p.actor, OPERATOR);
    assert_eq!(p.actor_kind, "terminal");
    assert_eq!(p.prior16, digest16(&before));
    // Embedded card round-trips through the ONE card parser.
    assert_eq!(p.op.to_card(), provider("nemotron"));
}

#[test]
fn propose_refuses_noop_against_current_fold() {
    let (stream, _view, journal) = sandbox("propose-noop");
    let op = EditOp::UpsertProvider(match provider("groq") {
        Card::Provider(p) => p,
        _ => unreachable!(),
    });
    assert_eq!(
        propose(&stream, &journal, op, OPERATOR, "terminal"),
        Err(EditErr::Noop {
            key: "provider/groq".into()
        })
    );
    assert!(!journal.exists(), "a refused propose writes nothing");
}

#[test]
fn propose_bytes_are_deterministic_across_identical_sandboxes() {
    let (s1, _v1, j1) = sandbox("determinism-a");
    let (s2, _v2, j2) = sandbox("determinism-b");
    let op = EditOp::UpsertSeat(match seat("groq/new-model", "groq") {
        Card::Seat(s) => s,
        _ => unreachable!(),
    });
    propose(&s1, &j1, op.clone(), OPERATOR, "terminal").unwrap();
    propose(&s2, &j2, op, OPERATOR, "terminal").unwrap();
    assert_eq!(
        fs::read_to_string(&j1).unwrap(),
        fs::read_to_string(&j2).unwrap(),
        "no clocks, no secrets — identical inputs give identical journal bytes"
    );
}

// --- confirm ----------------------------------------------------------------

#[test]
fn confirm_happy_path_lands_card_then_row_and_resyncs_the_view() {
    let (stream, view, journal) = sandbox("confirm-happy");
    let op = EditOp::UpsertSeat(match seat("nemotron/nem-4", "nemotron") {
        Card::Seat(s) => s,
        _ => unreachable!(),
    });
    let id = propose(&stream, &journal, op.clone(), OPERATOR, "terminal").unwrap();
    let before_lines = stream_lines(&stream);
    let out = confirm(
        &stream,
        &view,
        &journal,
        &id,
        OPERATOR,
        "terminal",
        &gate_open(),
    )
    .expect("gate is open, prior fresh");
    assert_eq!(out.proposal_id, "e1");
    assert_eq!(out.applied_key, "seat/nemotron/nem-4");
    assert_eq!(out.warden_card, "CARD-0001");
    assert_eq!(out.confirm_seq, 2);
    // Card landed in the stream (ONE new row).
    assert_eq!(stream_lines(&stream), before_lines + 1);
    // View re-synced against the truth.
    let (reg, rewritten) = registry::load_and_sync(&stream, &view).unwrap();
    assert!(!rewritten, "append_card already synced the view");
    assert!(reg.seats.contains_key("nemotron/nem-4"));
    // Fold = Confirmed.
    let folded = fold_journal(&journal_load(&journal)).unwrap();
    assert_eq!(folded.get("e1").unwrap().state, ProposalState::Confirmed);
    assert_eq!(
        folded.get("e1").unwrap().resolved_by.as_deref(),
        Some(OPERATOR)
    );
}

#[test]
fn confirm_refuses_stale_when_the_stream_moved_since_propose() {
    let (stream, view, journal) = sandbox("confirm-stale");
    let op = EditOp::UpsertSeat(match seat("nemotron/nem-4", "nemotron") {
        Card::Seat(s) => s,
        _ => unreachable!(),
    });
    let id = propose(&stream, &journal, op, OPERATOR, "terminal").unwrap();
    // A TTL-sweep-style append moves the stream WITHOUT our journal lock —
    // the confirm must catch it through the pinned prior.
    registry::append_card(&stream, &view, &provider("sweep")).unwrap();
    let before_stream = fs::read_to_string(&stream).unwrap();
    let before_journal = fs::read_to_string(&journal).unwrap();
    let err = confirm(
        &stream,
        &view,
        &journal,
        &id,
        OPERATOR,
        "terminal",
        &gate_open(),
    )
    .unwrap_err();
    assert!(matches!(err, EditErr::Stale { .. }), "got {err:?}");
    assert!(err.is_refusal());
    assert_eq!(
        fs::read_to_string(&stream).unwrap(),
        before_stream,
        "nothing written"
    );
    assert_eq!(fs::read_to_string(&journal).unwrap(), before_journal);
}

#[test]
fn confirm_refuses_when_the_gate_is_closed() {
    let (stream, view, journal) = sandbox("confirm-gate");
    let op = EditOp::UpsertSeat(match seat("nemotron/nem-4", "nemotron") {
        Card::Seat(s) => s,
        _ => unreachable!(),
    });
    let id = propose(&stream, &journal, op, OPERATOR, "terminal").unwrap();
    let before_stream = fs::read_to_string(&stream).unwrap();
    // No ledger at all.
    let err = confirm(&stream, &view, &journal, &id, OPERATOR, "terminal", "")
        .err()
        .unwrap();
    assert_eq!(
        err,
        EditErr::GateClosed {
            actor: OPERATOR.into()
        }
    );
    // A card open for a DIFFERENT session is NOT this actor's card
    // (CARD-0109 exact-caller law).
    let other_actor_ledger = warden_open("peleda.aaaaaaaa", "CARD-2");
    let err = confirm(
        &stream,
        &view,
        &journal,
        &id,
        OPERATOR,
        "terminal",
        &other_actor_ledger,
    )
    .err()
    .unwrap();
    assert_eq!(
        err,
        EditErr::GateClosed {
            actor: OPERATOR.into()
        }
    );
    // A closed card is closed.
    let closed = gate_open() + &warden_close(OPERATOR, "CARD-0001");
    let err = confirm(&stream, &view, &journal, &id, OPERATOR, "terminal", &closed)
        .err()
        .unwrap();
    assert_eq!(
        err,
        EditErr::GateClosed {
            actor: OPERATOR.into()
        }
    );
    // Nothing written in any of the three refusals.
    assert_eq!(fs::read_to_string(&stream).unwrap(), before_stream);
    assert_eq!(stream_lines(&journal), 1, "only the propose row exists");
    // The same ledger OPEN for this actor passes the gate.
    confirm(
        &stream,
        &view,
        &journal,
        &id,
        OPERATOR,
        "terminal",
        &gate_open(),
    )
    .unwrap();
}

#[test]
fn confirm_defects_on_an_unreadable_warden_ledger() {
    let (stream, view, journal) = sandbox("confirm-unreadable");
    let op = EditOp::UpsertSeat(match seat("nemotron/nem-4", "nemotron") {
        Card::Seat(s) => s,
        _ => unreachable!(),
    });
    let id = propose(&stream, &journal, op, OPERATOR, "terminal").unwrap();
    // A card-marked line that cannot parse: the ledger cannot attest
    // either way — fail-closed Defect, never a silent "closed".
    let torn = format!("{}{{\"type\":\"card.open\" broken\n", gate_open());
    let err = confirm(&stream, &view, &journal, &id, OPERATOR, "terminal", &torn)
        .err()
        .unwrap();
    assert!(
        !err.is_refusal(),
        "unreadable is a Defect, not a refusal: {err}"
    );
    assert!(err.to_string().contains("unreadable"));
    assert_eq!(stream_lines(&stream), 2, "nothing written");
}

#[test]
fn confirm_refuses_unknown_and_already_resolved() {
    let (stream, view, journal) = sandbox("confirm-unknown");
    assert_eq!(
        confirm(
            &stream,
            &view,
            &journal,
            "e99",
            OPERATOR,
            "terminal",
            &gate_open()
        ),
        Err(EditErr::UnknownProposal {
            proposal_id: "e99".into()
        })
    );
    let op = EditOp::UpsertProvider(match provider("nemotron") {
        Card::Provider(p) => p,
        _ => unreachable!(),
    });
    let id = propose(&stream, &journal, op, OPERATOR, "terminal").unwrap();
    confirm(
        &stream,
        &view,
        &journal,
        &id,
        OPERATOR,
        "terminal",
        &gate_open(),
    )
    .unwrap();
    assert_eq!(
        confirm(
            &stream,
            &view,
            &journal,
            &id,
            OPERATOR,
            "terminal",
            &gate_open()
        ),
        Err(EditErr::NotPending {
            proposal_id: id.clone(),
            state: ProposalState::Confirmed
        })
    );
}

#[test]
fn confirm_check_order_stale_wins_when_the_stream_moved_to_identical() {
    let (stream, view, journal) = sandbox("confirm-noop-drift");
    let new_seat = seat("nemotron/nem-4", "nemotron");
    let op = EditOp::UpsertSeat(match new_seat.clone() {
        Card::Seat(s) => s,
        _ => unreachable!(),
    });
    let id = propose(&stream, &journal, op, OPERATOR, "terminal").unwrap();
    // The exact card arrives through another path (a sweep) between
    // propose and confirm. Applying it would be a fold no-op, but the
    // stream MOVED — and the pinned prior is the OUTER law: Stale fires
    // first (check order: stale → no-op). The no-op re-check stays as
    // fail-closed defense-in-depth, proven directly below.
    registry::append_card(&stream, &view, &new_seat).unwrap();
    let err = confirm(
        &stream,
        &view,
        &journal,
        &id,
        OPERATOR,
        "terminal",
        &gate_open(),
    )
    .unwrap_err();
    assert!(matches!(err, EditErr::Stale { .. }), "got {err:?}");
}

#[test]
fn fold_noop_guard_matches_identical_and_differs_on_any_field() {
    let cards = vec![provider("groq"), seat("groq/llama-4", "groq")];
    assert!(fold_is_noop(&cards, &provider("groq")));
    assert!(fold_is_noop(&cards, &seat("groq/llama-4", "groq")));
    assert!(!fold_is_noop(&cards, &provider("nemotron")));
    // Same id, different field (caps) = NOT a no-op — a ruling is a change.
    let mut bumped = match provider("groq") {
        Card::Provider(p) => p,
        _ => unreachable!(),
    };
    bumped.caps = 2;
    assert!(!fold_is_noop(&cards, &Card::Provider(bumped)));
}
#[test]
fn refuse_resolves_pending_and_blocks_confirm() {
    let (stream, view, journal) = sandbox("refuse");
    let op = EditOp::UpsertProvider(match provider("nemotron") {
        Card::Provider(p) => p,
        _ => unreachable!(),
    });
    let id = propose(&stream, &journal, op, OPERATOR, "terminal").unwrap();
    let seq = refuse(&journal, &id, "world.operator", "ticketActor").unwrap();
    assert_eq!(seq, 2);
    let folded = fold_journal(&journal_load(&journal)).unwrap();
    let p = folded.get("e1").unwrap();
    assert_eq!(p.state, ProposalState::Refused);
    assert_eq!(p.resolved_by.as_deref(), Some("world.operator"));
    assert_eq!(
        confirm(
            &stream,
            &view,
            &journal,
            &id,
            OPERATOR,
            "terminal",
            &gate_open()
        ),
        Err(EditErr::NotPending {
            proposal_id: id,
            state: ProposalState::Refused
        })
    );
    assert_eq!(stream_lines(&stream), 2, "the card never landed");
}

// --- crash order ------------------------------------------------------------

#[test]
fn orphan_pending_never_double_applies() {
    let (stream, view, journal) = sandbox("orphan");
    let op = EditOp::UpsertSeat(match seat("nemotron/nem-4", "nemotron") {
        Card::Seat(s) => s,
        _ => unreachable!(),
    });
    let id = propose(&stream, &journal, op, OPERATOR, "terminal").unwrap();
    // Simulate the crash window: the card landed (STREAM FIRST), the
    // confirm row never did (JOURNAL LAST).
    registry::append_card(&stream, &view, &seat("nemotron/nem-4", "nemotron")).unwrap();
    let err = confirm(
        &stream,
        &view,
        &journal,
        &id,
        OPERATOR,
        "terminal",
        &gate_open(),
    )
    .unwrap_err();
    assert!(matches!(err, EditErr::Stale { .. }), "got {err:?}");
    assert_eq!(
        stream_lines(&stream),
        3,
        "exactly one copy of the card exists"
    );
}

// --- journal integrity ------------------------------------------------------

#[test]
fn a_forked_journal_refuses_writes_until_repaired() {
    let (stream, _view, journal) = sandbox("forked-seq");
    // A hand-forked journal: two rows share seq 1 (the model-voice
    // failure mode). Ids DERIVE from seqs here, so a forked seq is a
    // duplicate proposal id — fold refuses it and every write fails
    // closed until a human repairs the journal. Never silently healed.
    let line1 = propose_line(
        1,
        &EditOp::UpsertProvider(match provider("nemotron") {
            Card::Provider(p) => p,
            _ => unreachable!(),
        }),
        "aaaaaaaaaaaaaaaa",
        OPERATOR,
        "terminal",
    );
    let line1_dup = propose_line(
        1,
        &EditOp::UpsertProvider(match provider("groq2") {
            Card::Provider(p) => p,
            _ => unreachable!(),
        }),
        "bbbbbbbbbbbbbbbb",
        OPERATOR,
        "terminal",
    );
    fs::write(&journal, format!("{line1}{line1_dup}")).unwrap();
    let op = EditOp::UpsertProvider(match provider("zephyr") {
        Card::Provider(p) => p,
        _ => unreachable!(),
    });
    let err = propose(&stream, &journal, op, OPERATOR, "terminal").unwrap_err();
    assert!(!err.is_refusal(), "a fork is a Defect: {err}");
    // Repair: drop the forked duplicate; the next row takes seq 2 (max
    // parsed + 1 — never the line count).
    fs::write(&journal, line1).unwrap();
    let id = propose(
        &stream,
        &journal,
        EditOp::UpsertProvider(match provider("zephyr") {
            Card::Provider(p) => p,
            _ => unreachable!(),
        }),
        OPERATOR,
        "terminal",
    )
    .unwrap();
    assert_eq!(id, "e2");
}

#[test]
fn parse_rejects_op_class_mismatch_and_bad_priors() {
    // op word contradicts the embedded card class → unparseable line.
    let seat_line = registry::encode_card(&seat("groq/x", "groq"));
    let bad = format!(
        "{{\"seq\":1,\"kind\":\"propose\",\"proposal_id\":\"e1\",\"op\":\"upsert-provider\",\
         \"prior16\":\"aaaaaaaaaaaaaaaa\",\"actor\":\"a\",\"actor_kind\":\"terminal\",\
         \"card\":{}}}\n",
        crate::json::to_string(&Value::Str(seat_line))
    );
    let j = parse_journal(&bad);
    assert_eq!(j.unparseable, vec![1]);
    // prior16 not 16 lowercase hex → unparseable.
    let prov_line = registry::encode_card(&provider("groq"));
    let bad_prior = format!(
        "{{\"seq\":1,\"kind\":\"propose\",\"proposal_id\":\"e1\",\"op\":\"upsert-provider\",\
         \"prior16\":\"XYZ\",\"actor\":\"a\",\"actor_kind\":\"terminal\",\
         \"card\":{}}}\n",
        crate::json::to_string(&Value::Str(prov_line))
    );
    assert_eq!(parse_journal(&bad_prior).unparseable, vec![1]);
    // propose id not derived from its own seq → unparseable.
    let ok = propose_line(
        7,
        &EditOp::UpsertProvider(match provider("groq") {
            Card::Provider(p) => p,
            _ => unreachable!(),
        }),
        "aaaaaaaaaaaaaaaa",
        "a",
        "terminal",
    );
    let rewritten = ok.replace("\"proposal_id\":\"e7\"", "\"proposal_id\":\"e9\"");
    let j = parse_journal(&rewritten);
    assert_eq!(j.unparseable, vec![1], "id/seq mismatch must not parse");
    // The honest encode parses clean.
    assert_eq!(parse_journal(&ok).unparseable, Vec::<usize>::new());
}

#[test]
fn row_cap_refuses_oversized_writes() {
    let (stream, _view, journal) = sandbox("rowcap");
    let big_actor = "x".repeat(5000);
    let op = EditOp::UpsertProvider(match provider("nemotron") {
        Card::Provider(p) => p,
        _ => unreachable!(),
    });
    let err = propose(&stream, &journal, op, &big_actor, "terminal").unwrap_err();
    assert!(!err.is_refusal());
    assert!(err.to_string().contains("single-write cap"));
    assert!(!journal.exists());
}
