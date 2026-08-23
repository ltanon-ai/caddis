//! codec.rs — the flat-jsonl event codec (split from event.rs under the
//! 280-line law). Events serialize as FLAT json objects (no nesting):
//! keys are known, every value is an escaped string, which keeps the
//! hand-rolled codec (the zero-dependency law) honest and round-trip safe.

use crate::event::{EventKind, Lane, StateErr, TreeEvent};

pub(crate) fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

pub(crate) fn unesc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some(n) => out.push(n),
                None => out.push(c),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Leaf-shaped events (dispatch/gate) as flat fields.
fn leaf_fields(kind: &EventKind) -> Option<Vec<(String, String)>> {
    match kind {
        EventKind::LeafDispatch {
            card,
            attempt,
            cost,
            lane,
        } => Some(vec![
            ("card".to_string(), esc(card)),
            ("attempt".to_string(), attempt.to_string()),
            ("cost".to_string(), cost.to_string()),
            ("lane".to_string(), esc(&lane.tag())),
        ]),
        EventKind::LeafGated { card, pass } => Some(vec![
            ("card".to_string(), esc(card)),
            (
                "pass".to_string(),
                if *pass { "1" } else { "0" }.to_string(),
            ),
        ]),
        _ => None,
    }
}

/// Tree-shaped events as flat fields.
fn tree_fields(kind: &EventKind) -> Option<Vec<(String, String)>> {
    let pair = |a: &str, b: &str| vec![(a.to_string(), esc(b))];
    match kind {
        EventKind::GoalIntake { root_red } => Some(vec![("root_red".to_string(), esc(root_red))]),
        EventKind::PlanAccepted { plan, children } => Some(vec![
            ("plan".to_string(), esc(plan)),
            ("children".to_string(), esc(&children.join(","))),
        ]),
        EventKind::SubtreeLive { parent } => Some(pair("parent", parent)),
        EventKind::SubtreeClosed { parent } => Some(pair("parent", parent)),
        EventKind::BubbleUp { from, to } => Some(vec![
            ("from".to_string(), esc(from)),
            ("to".to_string(), esc(to)),
        ]),
        EventKind::ReplanParent { parent, reason } => Some(vec![
            ("parent".to_string(), esc(parent)),
            ("reason".to_string(), esc(reason)),
        ]),
        EventKind::StrongClose { card } => Some(pair("card", card)),
        _ => None,
    }
}

fn tag_of(kind: &EventKind) -> &'static str {
    match kind {
        EventKind::GoalIntake { .. } => "goal_intake",
        EventKind::PlanAccepted { .. } => "plan_accepted",
        EventKind::SubtreeLive { .. } => "subtree_live",
        EventKind::SubtreeClosed { .. } => "subtree_closed",
        EventKind::LeafDispatch { .. } => "leaf_dispatch",
        EventKind::LeafGated { .. } => "leaf_gated",
        EventKind::BubbleUp { .. } => "bubble_up",
        EventKind::ReplanParent { .. } => "replan_parent",
        EventKind::StrongClose { .. } => "strong_close",
    }
}

/// One flat json line (no nesting; children are comma-joined ids). Seq is
/// written QUOTED so every value-to-key boundary is `","` — that is what
/// the pair splitter below relies on.
pub(crate) fn event_line(ev: &TreeEvent) -> String {
    let fields = leaf_fields(&ev.kind).or_else(|| tree_fields(&ev.kind));
    let mut s = format!(
        "{{\"seq\":\"{}\",\"writer\":\"{}\",\"kind\":\"{}\"",
        ev.seq,
        esc(&ev.writer),
        tag_of(&ev.kind)
    );
    if let Some(fields) = fields {
        for (k, v) in fields {
            s.push_str(&format!(",\"{k}\":\"{v}\""));
        }
    }
    s.push_str("}\n");
    s
}

pub(crate) fn parse_line(line: &str) -> Result<TreeEvent, StateErr> {
    let inner = line.trim().trim_start_matches('{').trim_end_matches('}');
    let mut seq = 0u64;
    let mut writer = String::new();
    let mut kind = String::new();
    let mut fields: Vec<(String, String)> = Vec::new();
    for pair in inner.split("\",\"") {
        let (k, v) = pair
            .split_once(':')
            .ok_or(StateErr::Io("bad pair".into()))?;
        let key = k.trim().trim_matches('"').to_string();
        let val = v.trim().trim_matches('"').to_string();
        match key.as_str() {
            "seq" => seq = val.parse().map_err(|_| StateErr::Io("bad seq".into()))?,
            "writer" => writer = unesc(&val),
            "kind" => kind = val,
            _ => fields.push((key, val)),
        }
    }
    let built = build_leaf(&kind, &fields).or_else(|| build_tree(&kind, &fields));
    built
        .map(|k| TreeEvent {
            seq,
            writer,
            kind: k,
        })
        .ok_or_else(|| StateErr::Io(format!("unknown kind {kind}")))
}

fn take(fields: &[(String, String)], key: &str) -> String {
    fields
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| unesc(v))
        .unwrap_or_default()
}

fn build_leaf(kind: &str, f: &[(String, String)]) -> Option<EventKind> {
    match kind {
        "leaf_dispatch" => Some(EventKind::LeafDispatch {
            card: take(f, "card"),
            attempt: take(f, "attempt").parse().unwrap_or(0),
            cost: take(f, "cost").parse().unwrap_or(0),
            lane: Lane::from_tag(&take(f, "lane")),
        }),
        "leaf_gated" => Some(EventKind::LeafGated {
            card: take(f, "card"),
            pass: take(f, "pass") == "1",
        }),
        _ => None,
    }
}

fn build_tree(kind: &str, f: &[(String, String)]) -> Option<EventKind> {
    match kind {
        "goal_intake" => Some(EventKind::GoalIntake {
            root_red: take(f, "root_red"),
        }),
        "plan_accepted" => Some(EventKind::PlanAccepted {
            plan: take(f, "plan"),
            children: take(f, "children")
                .split(',')
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect(),
        }),
        "subtree_live" => Some(EventKind::SubtreeLive {
            parent: take(f, "parent"),
        }),
        "subtree_closed" => Some(EventKind::SubtreeClosed {
            parent: take(f, "parent"),
        }),
        "bubble_up" => Some(EventKind::BubbleUp {
            from: take(f, "from"),
            to: take(f, "to"),
        }),
        "replan_parent" => Some(EventKind::ReplanParent {
            parent: take(f, "parent"),
            reason: take(f, "reason"),
        }),
        "strong_close" => Some(EventKind::StrongClose {
            card: take(f, "card"),
        }),
        _ => None,
    }
}
