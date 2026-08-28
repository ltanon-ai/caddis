//! disjoint_tests.rs — P0 slice 3 gates (plan Done-When, slice 3): the F9
//! STRICT disjointness law + floors as DATA, both proven by a data-driven
//! DAY TABLE (each fixture is one day: council panel + candidate pool +
//! size + floors + the expected outcome), plus the direct F9 law check,
//! selection determinism, and the serde round-trip of [`QuorumPool`].

use std::time::SystemTime;

use crate::disjoint::{select_quorum_pool, DisjointErr, QuorumPool};
use crate::{construct_panel, CostClass, Floors, LaneType, Panel, PanelErr, Seat, SeatState};

fn seat(id: &str, family: &str, cost: CostClass, state: SeatState) -> Seat {
    Seat {
        lane_id: id.to_string(),
        lane_type: LaneType::Http,
        family: family.to_string(),
        provider: family.to_string(),
        model: format!("{family}-model"),
        cost_class: cost,
        state,
        caps: 1,
        last_probe: Some(SystemTime::UNIX_EPOCH),
    }
}

/// A healthy council fixture: 4 live seats, 4 families, 2 non-Chinese —
/// constructs under default floors. Lane ids prefixed `c-`; quorum
/// candidates use other prefixes so overlaps are deliberate.
fn healthy_council() -> Vec<Seat> {
    vec![
        seat("c-zai-1", "zai", CostClass::Free, SeatState::Live),
        seat("c-groq-1", "groq", CostClass::Free, SeatState::Live),
        seat("c-zhipu-1", "zhipu", CostClass::Free, SeatState::Live),
        seat("c-google-1", "google", CostClass::Free, SeatState::Live),
    ]
}

fn council_panel(candidates: &[Seat]) -> Panel {
    construct_panel(candidates, &Floors::default()).expect("fixture council must construct")
}

/// Quorum floors for the table (panel_size is ignored by the pool law —
/// construction-time constraint, not a pool shape).
fn quorum_floors() -> Floors {
    Floors {
        panel_size: 4,
        min_families: 2,
        min_non_chinese: 1,
    }
}

/// What a day must produce: the selected lane ids in selection order, or
/// an exact refusal.
enum Expect {
    Pool(Vec<&'static str>),
    Refusal(DisjointErr),
}

struct Day {
    name: &'static str,
    pool: Vec<Seat>,
    size: usize,
    floors: Floors,
    expect: Expect,
}

fn days() -> Vec<Day> {
    vec![
        // GOOD DAY: an overlapped FREE seat (c-zai-1) is skipped, so the
        // free-first order fills from disjoint seats and the Mid seat
        // takes the third slot — skip-then-order, never order-then-skip.
        Day {
            name: "good_day_skips_overlapped_free_before_ordering",
            pool: vec![
                seat("c-zai-1", "zai", CostClass::Free, SeatState::Live),
                seat("q-mid-c", "anthropic", CostClass::Mid, SeatState::Live),
                seat("q-free-b", "openai", CostClass::Free, SeatState::Live),
                seat("q-free-a", "groq", CostClass::Free, SeatState::Live),
            ],
            size: 3,
            floors: quorum_floors(),
            expect: Expect::Pool(vec!["q-free-a", "q-free-b", "q-mid-c"]),
        },
        // DEGRADED DAY (F9): one disjoint live seat + one overlapped —
        // the honest refusal carries the day's shape; NO soft pool.
        Day {
            name: "degraded_day_refuses_with_overlap_evidence",
            pool: vec![
                seat("c-zai-1", "zai", CostClass::Free, SeatState::Live),
                seat("q-free-a", "groq", CostClass::Free, SeatState::Live),
            ],
            size: 3,
            floors: quorum_floors(),
            expect: Expect::Refusal(DisjointErr::InsufficientDisjointPool {
                have: 1,
                want: 3,
                skipped_overlap: 1,
                skipped_non_live: 0,
            }),
        },
        // DEGRADED DAY, dead lanes edition: Expired/RateLimited seats
        // never rescue the shortfall (F10) — they are counted, not seated.
        Day {
            name: "degraded_day_non_live_never_rescues",
            pool: vec![
                seat("c-zai-1", "zai", CostClass::Free, SeatState::Live),
                seat("q-free-a", "groq", CostClass::Free, SeatState::Live),
                seat("q-expired", "mistral", CostClass::Free, SeatState::Expired),
                seat(
                    "q-ratelimited",
                    "nvidia",
                    CostClass::Free,
                    SeatState::RateLimited,
                ),
            ],
            size: 3,
            floors: quorum_floors(),
            expect: Expect::Refusal(DisjointErr::InsufficientDisjointPool {
                have: 1,
                want: 3,
                skipped_overlap: 1,
                skipped_non_live: 2,
            }),
        },
        // MONOCULTURE, families edition: 3 disjoint seats, all one family.
        // BOTH floors fail; families is reported first (fixed refusal
        // order — the same law the council panel proves).
        Day {
            name: "family_monoculture_refused_families_first",
            pool: vec![
                seat("q-a", "zai", CostClass::Free, SeatState::Live),
                seat("q-b", "zai", CostClass::Free, SeatState::Live),
                seat("q-c", "zai", CostClass::Free, SeatState::Live),
            ],
            size: 3,
            floors: quorum_floors(),
            expect: Expect::Refusal(DisjointErr::Floors(PanelErr::FamiliesFloor {
                have: 1,
                want: 2,
            })),
        },
        // MONOCULTURE, origin edition: 3 DISTINCT Chinese families — the
        // families floor passes, the non-Chinese floor refuses. Distinct
        // families are not enough; origin monoculture is the floor's
        // meaning.
        Day {
            name: "origin_monoculture_refused_non_chinese",
            pool: vec![
                seat("q-a", "qwen", CostClass::Free, SeatState::Live),
                seat("q-b", "deepseek", CostClass::Free, SeatState::Live),
                seat("q-c", "kimi", CostClass::Free, SeatState::Live),
            ],
            size: 3,
            floors: quorum_floors(),
            expect: Expect::Refusal(DisjointErr::Floors(PanelErr::NonChineseFloor {
                have: 0,
                want: 1,
            })),
        },
        // Usage defect: a zero-sized pool is refused, never an empty Ok.
        Day {
            name: "zero_size_refused",
            pool: vec![seat("q-free-a", "groq", CostClass::Free, SeatState::Live)],
            size: 0,
            floors: quorum_floors(),
            expect: Expect::Refusal(DisjointErr::EmptyPool),
        },
        // Floors are DATA: zeroed monoculture floors admit the 3-seats-
        // one-family pool the default floors refuse — same selector, no
        // code changes (router F6 precedent).
        Day {
            name: "floors_are_data_zeroed_floors_admit_monoculture",
            pool: vec![
                seat("q-a", "zai", CostClass::Free, SeatState::Live),
                seat("q-b", "zai", CostClass::Free, SeatState::Live),
                seat("q-c", "zai", CostClass::Free, SeatState::Live),
            ],
            size: 3,
            floors: Floors {
                panel_size: 4,
                min_families: 0,
                min_non_chinese: 0,
            },
            expect: Expect::Pool(vec!["q-a", "q-b", "q-c"]),
        },
    ]
}

/// The DAY TABLE: every fixture runs through the same selection law and
/// asserts its exact outcome — pools are also re-proven disjoint (the
/// selection post-condition, checked from the outside too).
#[test]
fn day_table_proves_f9_and_floors_as_data() {
    let council = council_panel(&healthy_council());
    for day in days() {
        let got = select_quorum_pool(&council, &day.pool, day.size, &day.floors);
        match (day.expect, got) {
            (Expect::Pool(ids), Ok(pool)) => {
                let got_ids: Vec<&str> = pool.seats.iter().map(|s| s.lane_id.as_str()).collect();
                assert_eq!(got_ids, ids, "day {}: wrong selection", day.name);
                assert!(
                    pool.check_disjoint_from(&council).is_ok(),
                    "day {}: selected pool overlaps the council",
                    day.name
                );
            }
            (Expect::Refusal(want), Err(got)) => {
                assert_eq!(got, want, "day {}: wrong refusal", day.name);
            }
            (Expect::Pool(_), Err(e)) => {
                panic!("day {}: expected a pool, got refusal: {e}", day.name)
            }
            (Expect::Refusal(_), Ok(pool)) => panic!(
                "day {}: expected a refusal, got pool {:?}",
                day.name,
                pool.seats
                    .iter()
                    .map(|s| s.lane_id.as_str())
                    .collect::<Vec<_>>()
            ),
        }
    }
}

/// The DIRECT F9 law check, independent of selection: a hand-built pool
/// carrying a council lane is refused with the sorted lane evidence; a
/// disjoint pool passes.
#[test]
fn check_disjoint_from_is_the_direct_law() {
    let council = council_panel(&healthy_council());
    let overlapping = QuorumPool {
        seats: vec![
            seat("q-a", "groq", CostClass::Free, SeatState::Live),
            seat("c-groq-1", "groq", CostClass::Free, SeatState::Live),
            seat("c-zai-1", "zai", CostClass::Free, SeatState::Live),
        ],
    };
    assert_eq!(
        overlapping.check_disjoint_from(&council),
        Err(DisjointErr::Overlap {
            lanes: vec!["c-groq-1".to_string(), "c-zai-1".to_string()],
        })
    );
    let disjoint = QuorumPool {
        seats: vec![
            seat("q-a", "groq", CostClass::Free, SeatState::Live),
            seat("q-b", "openai", CostClass::Free, SeatState::Live),
            seat("q-c", "anthropic", CostClass::Mid, SeatState::Live),
        ],
    };
    assert_eq!(disjoint.check_disjoint_from(&council), Ok(()));
}

/// Determinism (F1 replay law): input order must not change the pool.
#[test]
fn selection_is_deterministic_regardless_of_input_order() {
    let council = council_panel(&healthy_council());
    let mut candidates = vec![
        seat("q-mid-c", "anthropic", CostClass::Mid, SeatState::Live),
        seat("q-free-b", "openai", CostClass::Free, SeatState::Live),
        seat("q-free-a", "groq", CostClass::Free, SeatState::Live),
        seat("c-zai-1", "zai", CostClass::Free, SeatState::Live),
        seat("q-prem-d", "cohere", CostClass::Premium, SeatState::Live),
    ];
    let first = select_quorum_pool(&council, &candidates, 3, &quorum_floors())
        .expect("forward order must select");
    candidates.reverse();
    let second = select_quorum_pool(&council, &candidates, 3, &quorum_floors())
        .expect("reversed order must select");
    assert_eq!(first, second);
}

/// Plan Done-When: types round-trip through serde (dev-only proof; the
/// wire format freeze is a later phase).
#[test]
fn quorum_pool_serde_round_trips() {
    let pool = QuorumPool {
        seats: vec![
            seat("q-a", "groq", CostClass::Free, SeatState::Live),
            seat("q-b", "openai", CostClass::Free, SeatState::Live),
        ],
    };
    let json = serde_json::to_string(&pool).expect("serialize");
    let back: QuorumPool = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back, pool);
}
