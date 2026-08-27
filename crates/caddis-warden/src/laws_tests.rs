//! Direct tests for the law market (CARD-0113, unit C).

use super::*;

fn row(seq: u64, from: &str, body: &str) -> String {
    format!(
        "{{\"seq\":{seq},\"v\":1,\"id\":\"x\",\"idem_key\":\"k\",\"type\":\"tool.bash\",\
         \"from\":\"{from}\",\"to\":\"warden\",\"body\":\"{body}\",\"ts\":100}}\n"
    )
}

fn deny(seq: u64, from: &str, cmd: &str, law: &str) -> String {
    row(seq, from, &format!("deny|{cmd}||caddis-warden [{law}]: no"))
}

fn allow(seq: u64, from: &str, cmd: &str) -> String {
    row(seq, from, &format!("allow|{cmd}||"))
}

fn market(text: &str) -> Market {
    build(text, None, None, 100)
}

#[test]
fn a_registered_law_that_never_fired_is_dead() {
    let m = market("");
    let id = crate::checks::registered_ids()[0];
    assert_eq!(m.laws.get(id).expect("seeded").verdict(), "DEAD");
    // EVERY registered law appears, or the market cannot report a dead rule —
    // which is the one finding it exists to produce.
    assert_eq!(m.laws.len(), crate::checks::registered_ids().len());
}

#[test]
fn a_law_that_fires_and_is_obeyed_is_earning() {
    let led: String = (1..=4)
        .map(|n| deny(n, "t", "rm -rf /", "fs.rmrf"))
        .collect();
    let u = market(&led).laws.get("fs.rmrf").cloned().expect("fired");
    assert_eq!((u.deny, u.steer, u.circumvented), (4, 0, 0));
    assert_eq!(u.verdict(), "EARNING");
    assert_eq!(u.circumvention_rate(), 0.0);
}

#[test]
fn a_law_routinely_routed_around_is_wallpaper() {
    // Three of four denials followed by the same caller getting `rm` through.
    let mut led = String::new();
    for n in 0..3 {
        led.push_str(&deny(n * 2 + 1, "t", "rm -rf /x", "fs.rmrf"));
        led.push_str(&allow(n * 2 + 2, "t", "rm -rf /x --yes-really"));
    }
    led.push_str(&deny(99, "t", "rm -rf /y", "fs.rmrf"));
    let u = market(&led).laws.get("fs.rmrf").cloned().expect("fired");
    assert_eq!(u.deny, 4);
    assert_eq!(u.circumvented, 3);
    assert_eq!(u.circumvention_rate(), 75.0);
    assert_eq!(u.verdict(), "WALLPAPER");
}

#[test]
fn another_callers_allow_is_not_this_callers_circumvention() {
    // Attributing one agent's work-around to another would make the record
    // worse than no record at all.
    let led = deny(1, "peleda", "rm -rf /x", "fs.rmrf") + &allow(2, "omp", "rm -rf /x");
    let u = market(&led).laws.get("fs.rmrf").cloned().expect("fired");
    assert_eq!(u.deny, 1);
    assert_eq!(u.circumvented, 0);
    assert_eq!(u.verdict(), "EARNING");
}

#[test]
fn a_different_head_verb_afterwards_is_not_circumvention() {
    let led = deny(1, "t", "rm -rf /x", "fs.rmrf") + &allow(2, "t", "echo unrelated");
    assert_eq!(
        market(&led)
            .laws
            .get("fs.rmrf")
            .expect("fired")
            .circumvented,
        0
    );
}

#[test]
fn steers_count_as_fires_but_never_as_circumvention() {
    // A steer blocks nothing, so proceeding after one is obedience.
    let led = row(1, "t", "steer|git reset --hard||some.law") + &allow(2, "t", "git reset --hard");
    let u = market(&led).laws.get("some.law").cloned().expect("fired");
    assert_eq!((u.deny, u.steer, u.circumvented), (0, 1, 0));
    assert_eq!(u.verdict(), "EARNING");
}

#[test]
fn a_steer_carrying_several_ids_credits_each_of_them() {
    let u = market(&row(1, "t", "steer|git x||one.law, two.law"));
    assert_eq!(u.laws.get("one.law").expect("fired").steer, 1);
    assert_eq!(u.laws.get("two.law").expect("fired").steer, 1);
}

#[test]
fn a_torn_row_is_counted_and_does_not_break_the_market() {
    let torn = "{\"seq\":{\"seq\":538,\"v\":5381,\"v\":1,\"id\":\",\"id\":\"x\"}\n";
    let m = market(&(deny(1, "t", "rm -rf /", "fs.rmrf") + torn));
    assert_eq!(m.unreadable, 1);
    assert_eq!(m.laws.get("fs.rmrf").expect("fired").deny, 1);
    let text = render_text(&m);
    assert!(text.contains("FILE-WIDE"), "{text}");
}

#[test]
fn the_rendered_market_labels_the_heuristic_as_one() {
    // The number is inferred, and a reader who takes it as observed will
    // mis-blame an agent for an honest fix-and-retry.
    let text = render_text(&market(&deny(1, "t", "rm -rf /", "fs.rmrf")));
    assert!(text.contains("HEURISTIC"), "{text}");
    assert!(text.contains("never as a verdict about an agent"), "{text}");
    assert!(text.contains("EARNING"), "{text}");
}

#[test]
fn the_json_carries_the_heuristic_caveat_too() {
    // A machine consumer must not be handed a number the human report hedges.
    let json = render_json(&market(""));
    assert!(json.contains("\"heuristic\""), "{json}");
    assert!(json.contains("inferred"), "{json}");
}
