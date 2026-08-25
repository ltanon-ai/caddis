//! Direct tests for law discovery (CARD-0113, unit D).

use super::*;

fn row(seq: u64, from: &str, body: &str) -> String {
    format!(
        "{{\"seq\":{seq},\"v\":1,\"id\":\"x\",\"idem_key\":\"k\",\"type\":\"tool.bash\",\
         \"from\":\"{from}\",\"to\":\"warden\",\"body\":\"{body}\",\"ts\":100}}\n"
    )
}

fn allow(seq: u64, from: &str, cmd: &str) -> String {
    row(seq, from, &format!("allow|{cmd}||"))
}

fn of(text: &str) -> Proposals {
    build(text, None, 100)
}

#[test]
fn an_allow_then_undo_pair_becomes_a_candidate() {
    let led = allow(1, "t", "git commit -m wip") + &allow(2, "t", "git reset --hard HEAD~1");
    let p = of(&led);
    assert_eq!(p.candidates.len(), 1);
    let c = &p.candidates[0];
    assert_eq!(c.signature, "git commit");
    assert_eq!(c.occurrences, 1);
    assert_eq!(c.example_seq, 1);
}

#[test]
fn an_allow_with_no_undo_after_it_proposes_nothing() {
    let led = allow(1, "t", "git commit -m done") + &allow(2, "t", "cargo test");
    assert!(of(&led).candidates.is_empty());
}

#[test]
fn the_undo_itself_is_never_proposed_as_its_own_law() {
    // Otherwise every `git reset` followed by another `git reset` proposes a
    // law against resetting, which is exactly backwards.
    let led = allow(1, "t", "git reset --hard") + &allow(2, "t", "git reset --hard");
    assert!(of(&led).candidates.is_empty());
}

#[test]
fn an_undo_by_a_different_caller_does_not_implicate_this_one() {
    let led = allow(1, "peleda", "git commit -m x") + &allow(2, "omp", "git reset --hard");
    assert!(of(&led).candidates.is_empty());
}

#[test]
fn every_candidate_carries_its_whole_ledger_cost() {
    // ⛔ THE FALSIFIER IS THE POINT. `git commit` appears 3 times in all; a law
    // on that signature would have denied all 3, and the reader must see that
    // BEFORE adopting it.
    let led = allow(1, "t", "git commit -m a")
        + &allow(2, "t", "git reset --hard")
        + &allow(3, "t", "git commit -m b")
        + &allow(4, "t", "git commit -m c");
    let p = of(&led);
    let c = p
        .candidates
        .iter()
        .find(|c| c.signature == "git commit")
        .expect("found");
    assert_eq!(c.occurrences, 1, "one allow-then-undo pair");
    assert_eq!(c.would_deny, 3, "but three rows share the signature");
}

#[test]
fn candidates_are_ordered_by_strength_of_evidence() {
    let mut led = String::new();
    let mut seq = 0;
    for _ in 0..3 {
        seq += 1;
        led.push_str(&allow(seq, "t", "git push origin main"));
        seq += 1;
        led.push_str(&allow(seq, "t", "git revert HEAD"));
    }
    seq += 1;
    led.push_str(&allow(seq, "t", "git merge feature"));
    seq += 1;
    led.push_str(&allow(seq, "t", "git reset --hard"));
    let p = of(&led);
    assert!(p.candidates.len() >= 2);
    assert_eq!(p.candidates[0].signature, "git push");
    assert_eq!(p.candidates[0].occurrences, 3);
}

#[test]
fn the_signature_is_two_words_so_it_can_recur() {
    // A whole command line never recurs verbatim, and one word cannot tell
    // `git commit` from `git push`.
    assert_eq!(
        signature("git commit -m 'a long message here'"),
        "git commit"
    );
    assert_eq!(signature("rm -rf /tmp/x"), "rm -rf");
    assert_eq!(signature("cargo"), "cargo");
    assert_eq!(signature(""), "");
}

#[test]
fn no_candidates_is_a_legitimate_answer_and_says_so() {
    let text = render_text(&of(&allow(1, "t", "echo hello")));
    assert!(text.contains("NO CANDIDATES"), "{text}");
    assert!(text.contains("not a failure to look"), "{text}");
}

#[test]
fn the_rendered_proposal_states_the_falsifier_in_words() {
    let led = allow(1, "t", "git commit -m a") + &allow(2, "t", "git reset --hard");
    let text = render_text(&of(&led));
    assert!(text.contains("FALSIFIER"), "{text}");
    assert!(text.contains("would have denied"), "{text}");
    assert!(text.contains("BEFORE adopting it"), "{text}");
    // And it must say plainly that it installs nothing.
    assert!(text.contains("Nothing here installs a law"), "{text}");
}

#[test]
fn a_torn_row_is_counted_and_does_not_stop_the_mining() {
    let torn = "{\"seq\":{\"seq\":538,\"v\":5381,\"v\":1,\"id\":\",\"id\":\"x\"}\n";
    let led = allow(1, "t", "git commit -m a") + &allow(2, "t", "git reset --hard") + torn;
    let p = of(&led);
    assert_eq!(p.unreadable, 1);
    assert_eq!(p.candidates.len(), 1);
}

#[test]
fn the_json_shape_is_one_object_with_an_array() {
    let json = render_json(&of(""));
    assert!(json.starts_with('{') && json.ends_with('}'), "{json}");
    assert!(json.contains("\"candidates\":[]"), "{json}");
}
