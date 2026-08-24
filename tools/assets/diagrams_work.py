"""diagrams_work.py — the work-system diagrams: card anatomy, the
ladder, the goal tree. Split from the records story under the 300-line
hard cap; same chrome (palette + primitives) as the other modules.
"""

import os

from .chrome import (
    AMBER,
    DIM,
    TEAL,
    TEXT,
    arrow,
    canvas,
    label,
    mono,
    panel,
)

ASSETS = os.path.join(
    os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))),
    "assets",
)


def draw_card_anatomy():
    """One card, anatomized: frontmatter, falsifiable sections, contract."""
    W, H = 1600, 900
    img, d = canvas(W, H)
    d.text((60, 44), "THE CARD, ANATOMIZED", font=mono(34, bold=True), fill=TEXT)
    label(d, 60, 92, "falsifiable completion, mechanical proof, a contract the executor cannot renegotiate", 22)

    # the document
    panel(d, 60, 170, 560, 640, None, accent=TEAL)
    label(d, 84, 190, "card.md", 20, TEAL, bold=True)
    doc = [
        ("---", DIM),
        ("id: CARD-42", TEXT),
        ("class: fix", TEXT),
        ("owner: my-agent", TEXT),
        ("---", DIM),
        ("# CARD-42: the bug", TEXT),
        ("## Done-When", TEAL),
        ("- pytest -q: 12 passed", DIM),
        ("## RED-TEST", TEAL),
        ("- test_z fails today: E1", DIM),
        ("## EXECUTION", AMBER),
        ("level: L2", DIM),
        ("blast: 2", DIM),
        ("claims-forbidden: true", DIM),
        ("anchors:  (verbatim)", DIM),
        ("allowlist: (exact paths)", DIM),
    ]
    for i, (s, c) in enumerate(doc):
        label(d, 84, 226 + i * 26, s, 17, c)

    # annotations
    ann = [
        (700, 180, "FRONTMATTER", "id / class / owner - who owns the unit,\nwhat kind of work it is", TEAL),
        (700, 315, "DONE-WHEN", "completion a machine can check: a command,\nan exit code, a grep - never \"looks correct\"", TEAL),
        (700, 455, "RED-TEST", "the failing proof, run BEFORE the work -\nthe work cannot quietly redefine success", AMBER),
        (700, 600, "EXECUTION (strict)", "level L1-L3 - blast 1..=3, hard error outside -\nEXACT-verbatim anchors - an exact allowlist -\nclaims-forbidden", AMBER),
    ]
    for x, y, t, g, col in ann:
        panel(d, x, y, 840, 115, None, accent=col)
        label(d, x + 24, y + 18, t, 22, col, bold=True)
        for j, line in enumerate(g.split("\n")):
            label(d, x + 24, y + 52 + j * 24, line, 17)
    for y in (240, 500):
        arrow(d, 620, y, 700, y)

    label(d, 60, H - 64, "sections sit at # or ## - and a fenced block is CONTENT: an embedded plan never leaks its sections into the wrapper", 19)
    img.save(os.path.join(ASSETS, "diagram-card-anatomy.png"))


def draw_ladder():
    """Levels are earned by measurement; every rule is mechanical."""
    W, H = 1600, 900
    img, d = canvas(W, H)
    d.text((60, 44), "THE LADDER", font=mono(34, bold=True), fill=TEXT)
    label(d, 60, 92, "dispatching to a local model earns levels by measurement - never a promise, never a judgment call", 22)

    # levels
    for i, (lv, desc) in enumerate([
        ("L1", "one verbatim line replace"),
        ("L2", "one small function"),
        ("L3", "one change, two anchored files"),
    ]):
        y = 180 + i * 190
        col = TEAL if i < 2 else AMBER
        panel(d, 60, y, 330, 150, None, accent=col)
        label(d, 84, y + 22, lv, 34, col, bold=True)
        label(d, 84, y + 76, desc, 18)
        if i < 2:
            arrow(d, 225, y + 150, 225, y + 190)
    label(d, 430, 196, "+1 only after 2 consecutive", 17, TEAL)
    label(d, 430, 222, "first-attempt untransformed accepts", 17, TEAL)
    label(d, 430, 280, "blast / claims / retired-transform:", 17, AMBER)
    label(d, 430, 306, "immediate -1, floor L1", 17, AMBER)

    # the bounded loop
    panel(d, 480, 380, 500, 440, None)
    label(d, 504, 400, "THE BOUNDED LOOP", 20, TEXT, bold=True)
    steps = [
        ("dispatch one-shot", "FRESH context each attempt"),
        ("gates decide", "never the model's own claim"),
        ("reject?", "classify mode, apply ONE transform"),
        ("retry", "max 3 attempts, then strong lane"),
    ]
    for i, (t, g) in enumerate(steps):
        y = 442 + i * 90
        panel(d, 504, y, 452, 74, None)
        label(d, 524, y + 10, t, 19, TEAL, bold=True)
        label(d, 524, y + 40, g, 16, DIM)

    # telemetry
    panel(d, 1040, 180, 500, 640, None)
    label(d, 1064, 200, "THE PROFILE (telemetry)", 20, TEXT, bold=True)
    rows = [
        ("~/.caddis/executor-profiles/<model>.json", TEAL),
        ("levels: attempts / accepts / fallbacks", DIM),
        ("streak: clean_first_attempts", DIM),
        ("transforms: used / converted", DIM),
        ("plan: proposed / well_formed /", DIM),
        ("   intent_accepted / intent_rejected", DIM),
        ("stamped rows, one per dispatch:", DIM),
        ("{goal_id, card_id, strategy,", DIM),
        (" blast_set, outcome}", DIM),
    ]
    for i, (s, c) in enumerate(rows):
        label(d, 1064, 244 + i * 34, s, 18, c, bold=(c == TEAL))
    label(d, 1064, 560, "PRESET SWITCH = HYSTERESIS", 19, AMBER, bold=True)
    label(d, 1064, 592, "4 consecutive non-accepts under the", 18)
    label(d, 1064, 618, "current preset - never a ratio,", 18)
    label(d, 1064, 644, "never a judgment call (BC4)", 18)
    label(d, 1064, 700, "a transform with >=3 uses and 0", 17, DIM)
    label(d, 1064, 724, "conversions is RETIRED - never re-proposed", 17, DIM)
    img.save(os.path.join(ASSETS, "diagram-ladder.png"))


def draw_tree():
    """The goal tree: append-only events, one writer, resume from the file."""
    W, H = 1600, 900
    img, d = canvas(W, H)
    d.text((60, 44), "THE GOAL TREE", font=mono(34, bold=True), fill=TEXT)
    label(d, 60, 92, "plans decompose into ordered children; an append-only event log is the only state; a killed walk resumes from the file alone", 22)

    events = [
        ("seq 7  PlanAccepted", DIM),
        ("seq 8  SubtreeLive", DIM),
        ("seq 9  LeafDispatch{strategy}", TEAL),
        ("seq 10 LeafGated accept", DIM),
        ("seq 11 LeafDispatch{strategy}", TEAL),
        ("seq 12 LeafGated reject", AMBER),
        ("seq 13 LeafDispatch{strategy}", TEAL),
        ("seq 14 LeafGated reject", AMBER),
        ("seq 15 BubbleUp -> PLAN", AMBER),
        ("seq 16 ReplanParent", AMBER),
        ("... StrongClose on a LATER", DIM),
        ("    walk, after retries exhaust", DIM),
    ]
    for i, (s, c) in enumerate(events):
        label(d, 84, 232 + i * 40, s, 18, c, bold=(c in (TEAL, AMBER)))
    label(d, 84, 640, "KILLED MID-TREE ->", 19, AMBER, bold=True)
    label(d, 84, 670, "rebuild: gated leaves refuse re-dispatch", 18)
    label(d, 84, 696, "(AlreadyDone); the caller continues", 18)
    label(d, 684, 190, "PLAN (children + review)", 19, TEAL, bold=True)
    panel(d, 700, 250, 150, 90, None)
    label(d, 84, 670, "rebuild: passed or strong-closed leaves", 18)
    label(d, 84, 696, "refuse re-dispatch (AlreadyDone);", 18)
    label(d, 84, 722, "the caller continues", 18)
    label(d, 896, 274, "CARD-B", 20, TEXT, bold=True)
    label(d, 896, 302, "order 2", 16, DIM)
    arrow(d, 775, 250, 775, 230)
    arrow(d, 955, 250, 955, 230)
    label(d, 684, 372, "the parent's gate is all children green", 17, DIM)

    # the laws of the log
    for i, q in enumerate([
        "seq is monotonic - a mismatched log is refused",
        "ONE writer per log - a second session refused",
        "caps: attempts checked before dispatch,",
        "  cost checked at append (used + incoming)",
        "the in-memory view is ONLY ever rebuilt",
        "  from the file",
    ]):
        label(d, 684, 568 + i * 30, q, 17)

    # the bench
    panel(d, 1120, 170, 420, 640, None, accent=AMBER)
    label(d, 1144, 190, "THE BENCH (honest numbers)", 19, AMBER, bold=True)
    label(d, 1144, 240, "CheckerExecutor", 20, TEAL, bold=True)
    for j, line in enumerate(_wrap(
        "the leaf gate is the TARGET repo's own checker (a real subprocess) - never a simulated verdict", 42
    )):
        label(d, 1144, 272 + j * 24, line, 17, DIM)
    label(d, 1144, 400, "walk_goal", 20, TEAL, bold=True)
    for j, line in enumerate(_wrap(
        "retries a failing leaf up to 3 attempts, bubbles up, replans once - a stuck subtree strong-closes on a later walk", 42
    )):
        label(d, 1144, 432 + j * 24, line, 17, DIM)
    label(d, 1144, 560, "BenchCols", 20, TEAL, bold=True)
    for j, line in enumerate(_wrap(
        "goals_attempted, first_attempt_green, bubble_ups, strong_closures - nothing flattering is invented", 42
    )):
        label(d, 1144, 592 + j * 24, line, 17, DIM)
    label(d, 1144, 700, "strong-lane plan review is the SHIPPED", 17)
    label(d, 1144, 726, "default - structurally: no weak review", 17)
    label(d, 1144, 752, "path exists to fall back to", 17)
    img.save(os.path.join(ASSETS, "diagram-tree.png"))


def _wrap(text, width):
    out, cur = [], ""
    for word in text.split():
        if len(cur) + len(word) + 1 > width:
            out.append(cur)
            cur = word
        else:
            cur = f"{cur} {word}".strip()
    if cur:
        out.append(cur)
    return out
